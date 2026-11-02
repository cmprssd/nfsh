//! A zone made ready to draw, on the CPU: every placed mesh's first LOD packed into
//! one vertex and one index buffer, its placements into one instance buffer, and one
//! instanced draw per section, with the section's colour texture and how it is
//! shaded.

use std::collections::{BTreeMap, HashMap};

use frostbite::level::{Mesh, Transform, Zone};
use frostbite::mesh::{Geometry, Meshes};
use frostbite::terrain::{Ground, GroundMap};
use frostbite::texture::Material;

/// A placement's transform as the vertex shader reads it: right, up and forward,
/// each followed by one coordinate of the translation.
pub type Instance = [f32; 12];

/// One instanced indexed draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Draw {
    pub first_index: u32,
    pub index_count: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
    pub instance_count: u32,
    /// Index into [`Scene::textures`]; `None` draws plain grey.
    pub texture: Option<u32>,
    /// Asphalt's other textures ([`ASPHALT_LAYERS`]), likewise.
    pub layers: [Option<u32>; 6],
    /// The normal map, likewise; `None` draws the mesh's own normals.
    pub normal_map: Option<u32>,
    pub shading: Shading,
}

/// How a draw's colour comes from its texture (see `docs/DECISIONS.md`, "Road
/// materials").
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Shading {
    /// The texture's colour.
    #[default]
    Colour,
    /// The texture's colour; pixels whose alpha is under a half are not drawn
    /// (leaves).
    Cutout,
    /// The texture's red channel as a grey, times `tint`: a road's asphalt when its
    /// layers are not all there.
    Grey { tint: [f32; 3] },
    /// A road's asphalt, the texture its clean layer's diffuse.
    Asphalt(Asphalt),
    /// The ground: the vertex colour (sRGB-encoded), with [`Scene::detail`]'s layers
    /// over it as detail, each weighted by the vertex's weight for it (packed in its
    /// texture coordinates' bytes).
    Ground,
    /// Blended over what is drawn under it, after everything opaque: the opacity is
    /// the texture's channels weighted by `select`, times `opacity` and the vertex
    /// colour's red; the colour is `colour`, or else the texture's red times the
    /// vertex colour (road lines, white or yellow).
    Decal {
        select: [f32; 4],
        opacity: f32,
        colour: Option<[f32; 3]>,
    },
}

/// A road's asphalt (see `docs/FORMATS.md`, "Road materials"): a clean and a
/// damaged layer, each a grey (its diffuse's red, sRGB-encoded) tinted and tiled,
/// blended by the road's mask; five detail colours laid over them; the wheel paths
/// tinted. Each value is its material's, or else its shader's default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Asphalt {
    /// The clean layer's, then the damaged one's (`Tarmac_Tint_FG`, `…_BG`).
    pub tint: [[f32; 3]; 2],
    /// Likewise, times the first texture coordinates (`Tarmac_Tiling_FG`, `…_BG`).
    pub tiling: [[f32; 2]; 2],
    /// Where the damaged layer's height times the mask's red falls under the offset,
    /// the damaged layer shows, sharper as the contrast rises
    /// (`Tarmac_Blend_Offset`, `…_Contrast`).
    pub blend: [f32; 2],
    /// Likewise for the wheel paths, the mask's blue (`RoadTire_Blend_Offset`,
    /// `…_Contrast`)…
    pub tyre: [f32; 2],
    /// …which take this tint (`RoadTire_Tint_Color`, 0.5 leaving them as they are).
    pub tyre_tint: [f32; 3],
    /// Added to both blends (`VertexColor_Blend_Offset`).
    pub bias: f32,
    /// `detailColor1` to `5`, each laid over the colour (an overlay, 0.5 leaving it
    /// as it is) as much as the layers' diffuse blue and the noise say.
    pub detail: [[f32; 3]; 5],
    /// Where the road gives way to the ground under it; `None` for a road drawn to
    /// its edges.
    pub edge: Option<Edge>,
}

/// Where a road gives way to the ground (`rd_drv_blendwithterrain`, with its
/// `Shoulder_NH`): the shoulder texture's blue, in the third texture coordinates,
/// says where the road shows, and the road's mask's green times its height takes it
/// away again where it falls under the offset (see `docs/FORMATS.md`, "Asphalt").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    /// Times the third texture coordinates (`Shoulder_Tiling`).
    pub tiling: [f32; 2],
    /// `Dirt_Height_Offset`, `…_Contrast`, then `…_Paint_Boost`, which takes from the
    /// mask's green.
    pub dirt: [f32; 3],
}

/// The textures asphalt is drawn with besides its clean layer's diffuse, in the
/// order [`Draw::layers`] holds them: the damaged layer's diffuse and normal map
/// (its height in blue), the road's mask (in the second texture coordinates), the
/// noise its detail colours follow, the clean layer's normal map and the shoulder
/// ([`Edge`]). Most materials leave the noise to the shader, whose own is
/// [`ASPHALT_NOISE`]. The first [`ASPHALT_NEEDS`] must be there for asphalt to be
/// drawn as such; the clean layer's height is a half without its map.
pub const ASPHALT_LAYERS: [&str; 6] = [
    "Tarmac_BG_Diffuse",
    "Tarmac_BG_Normal",
    "detailMask",
    "Noise",
    "Tarmac_FG_Normal",
    "Shoulder_NH",
];
pub const ASPHALT_NEEDS: usize = 4;

pub const ASPHALT_NOISE: &str =
    "worldassets_roboto/common_textures/roadsurfaces/detailtexture/drv_road_detail_d";

/// The partition of the `TextureAsset` named `name`, if the store holds it.
pub fn texture_partition(store: &frostbite::store::Store, name: &str) -> Option<[u8; 16]> {
    let asset = store.ebx(name)?;
    let data = store.entries().ok()?.read(asset.location).ok()?;
    Some(frostbite::ebx::Partition::parse(&data).ok()?.header.guid)
}

impl Asphalt {
    /// The asphalt of material `m`, with the shader's defaults for what it doesn't
    /// set.
    pub fn of(m: &Material) -> Self {
        let rgb =
            |name: &str, default: f32| m.vector(name).map_or([default; 3], |v| [v[0], v[1], v[2]]);
        let xy = |name: &str| m.vector(name).map_or([1.0; 2], |v| [v[0], v[1]]);
        let x = |name: &str, default: f32| m.vector(name).map_or(default, |v| v[0]);
        Self {
            tint: [rgb("Tarmac_Tint_FG", 1.0), rgb("Tarmac_Tint_BG", 1.0)],
            tiling: [xy("Tarmac_Tiling_FG"), xy("Tarmac_Tiling_BG")],
            blend: [
                x("Tarmac_Blend_Offset", 0.171_457),
                x("Tarmac_Blend_Contrast", 0.879_622),
            ],
            tyre: [
                x("RoadTire_Blend_Offset", 0.501_961),
                x("RoadTire_Blend_Contrast", 0.879_622),
            ],
            tyre_tint: rgb("RoadTire_Tint_Color", 0.5),
            bias: x("VertexColor_Blend_Offset", 0.214_041),
            detail: [1, 2, 3, 4, 5].map(|i| rgb(&format!("detailColor{i}"), 0.5)),
            edge: m.texture("Shoulder_NH").map(|_| Edge {
                tiling: xy("Shoulder_Tiling"),
                dirt: [
                    x("Dirt_Height_Offset", 0.215_860),
                    x("Dirt_Height_Contrast", 0.814_847),
                    x("Dirt_Height_Paint_Boost", 0.0),
                ],
            }),
        }
    }
}

/// A mesh section and what it is drawn with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    pub geometry: Geometry,
    /// The `TextureAsset` partition its colour comes from; `None` for none, or when
    /// the section has no texture coordinates.
    pub texture: Option<[u8; 16]>,
    /// Asphalt's other textures ([`ASPHALT_LAYERS`]).
    pub layers: [Option<[u8; 16]>; 6],
    /// The `TextureAsset` partition of its material's `Normal`, if it names one;
    /// asphalt's are among its layers.
    pub normal_map: Option<[u8; 16]>,
    pub shading: Shading,
}

/// The texture parameters that hold a material's colour, in the order they are
/// looked for, and whether the colour's alpha cuts the surface out.
const COLOUR: [(&str, bool); 4] = [
    ("Diffuse", false),
    ("ColorAlpha", true),
    ("Diffuse_Tiling", false),
    ("foregroundBasecolor", false),
];

/// A vertex normal as the GPU reads it: four signed bytes, 127 being 1, the fourth
/// 1 to say it is there.
pub fn normal(n: [f32; 3]) -> [i8; 4] {
    let byte = |x: f32| (x.clamp(-1.0, 1.0) * 127.0).round() as i8;
    [byte(n[0]), byte(n[1]), byte(n[2]), 127]
}

/// A vertex's tangent as the GPU reads it, likewise, the fourth byte the bitangent's
/// sign ([`frostbite::mesh::Frame::sign`]).
pub fn tangent(f: &frostbite::mesh::Frame) -> [i8; 4] {
    let [x, y, z, _] = normal(f.tangent);
    [x, y, z, if f.sign < 0.0 { -127 } else { 127 }]
}

/// The texture a material's colour comes from, and how it is shaded, for the
/// material `m` of the shader named `shader`: road decals (shaders named
/// `…transparentdcl…`) blend their `Diffuse` by `alphaSelector`, `Opacity` and
/// `lineColor`; asphalt (`Tarmac_FG_Diffuse`) is [`Asphalt`] when it names the rest
/// of its layers ([`asphalt_layers`]), else that texture's grey times
/// `Tarmac_Tint_FG`, and gives way to the ground at its edges only with the shader
/// that blends it (`…blendwithterrain`); anything else takes the first colour texture
/// it names.
pub fn look(shader: &str, m: &Material) -> Option<([u8; 16], Shading)> {
    let rgb = |v: [f32; 4]| [v[0], v[1], v[2]];
    if shader.contains("transparentdcl") {
        let shading = Shading::Decal {
            select: m.vector("alphaSelector").unwrap_or([0.0, 0.0, 0.0, 1.0]),
            opacity: m.vector("Opacity").map_or(1.0, |v| v[0]),
            colour: m.vector("lineColor").map(rgb),
        };
        return Some((m.texture("Diffuse")?, shading));
    }
    if let Some(t) = m.texture("Tarmac_FG_Diffuse") {
        if asphalt_layers(m, None)[..3].iter().all(Option::is_some) {
            let mut asphalt = Asphalt::of(m);
            if !shader.contains("blendwithterrain") {
                asphalt.edge = None;
            }
            return Some((t, Shading::Asphalt(asphalt)));
        }
        let tint = m.vector("Tarmac_Tint_FG").map_or([1.0; 3], rgb);
        return Some((t, Shading::Grey { tint }));
    }
    COLOUR.iter().find_map(|&(name, cutout)| {
        let shading = if cutout {
            Shading::Cutout
        } else {
            Shading::Colour
        };
        Some((m.texture(name)?, shading))
    })
}

/// The textures material `m` names for [`ASPHALT_LAYERS`], the noise being `noise`
/// where it names none.
pub fn asphalt_layers(m: &Material, noise: Option<[u8; 16]>) -> [Option<[u8; 16]>; 6] {
    let mut layers = ASPHALT_LAYERS.map(|name| m.texture(name));
    layers[3] = layers[3].or(noise);
    layers
}

/// What a zone draws, and what it could not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub positions: Vec<[f32; 3]>,
    /// One per position; zero where a section has none.
    pub uvs: Vec<[f32; 2]>,
    /// One per position (RGBA, 255 is 1); white where a section has none.
    pub colours: Vec<[u8; 4]>,
    /// The second texture coordinates and colours, one per position; zero where a
    /// section has none.
    pub uvs2: Vec<[f32; 2]>,
    pub colours2: Vec<[u8; 4]>,
    /// The third texture coordinates, likewise.
    pub uvs3: Vec<[f32; 2]>,
    /// One per position, in the mesh's space ([`normal`]); zero where a section has
    /// none, which draws the triangle's own.
    pub normals: Vec<[i8; 4]>,
    /// Their tangents ([`tangent`]), likewise.
    pub tangents: Vec<[i8; 4]>,
    pub indices: Vec<u32>,
    pub instances: Vec<Instance>,
    pub draws: Vec<Draw>,
    /// The `TextureAsset` partitions the draws use, each once.
    pub textures: Vec<[u8; 16]>,
    /// Of where the drawn meshes are placed (their origins, not their vertices: some
    /// meshes' vertices are authored far from their origin, e.g. a speedway stand's
    /// steps already in world space, which their placements then move kilometres
    /// away); `None` if nothing is drawn.
    pub bounds: Option<([f32; 3], [f32; 3])>,
    /// The ground's detail: one per weight its vertices carry, in that order.
    pub detail: Vec<Detail>,
    /// The ground's colour and weights on a grid, for asphalt's edges; `None` without
    /// a ground.
    pub ground_map: Option<GroundMap>,
    /// Meshes that could not be read, with why, and how many placements that left
    /// out.
    pub failed: Vec<(String, String)>,
    pub placements_left_out: usize,
}

/// A layer drawn as detail over the ground's colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    /// Its diffuse map's ebx name.
    pub texture: String,
    /// Its normal map's, if it has one.
    pub normal: Option<String>,
    /// World x and z times this are both maps' coordinates.
    pub scale: f32,
}

/// The road sets' maps (`drv_…_d`), whose channels are not plain colour.
const ROAD_SET: &str = "drv_";

/// Each layer's detail, from its own textures ([`frostbite::layers::textures`]): the
/// diffuse map (`…_d`) with the finest scale (the first by name among equals),
/// leaving out maps with no scale and the road sets', with the normal map of the same
/// name (`…_n`) where the layer has it. A layer with no diffuse has no detail.
pub fn ground_detail(
    textures: &BTreeMap<u16, Vec<frostbite::layers::Texture>>,
) -> BTreeMap<u16, Detail> {
    textures
        .iter()
        .filter_map(|(&layer, maps)| {
            let finest = maps
                .iter()
                .filter(|d| {
                    let short = d.name.rsplit('/').next().unwrap_or("");
                    short.ends_with("_d") && !short.starts_with(ROAD_SET)
                })
                .filter_map(|d| Some((d.scale?, &d.name)))
                .reduce(|a, b| if b.0 > a.0 { b } else { a })?;
            let normal = format!("{}_n", finest.1.strip_suffix("_d").unwrap_or(finest.1));
            Some((
                layer,
                Detail {
                    texture: finest.1.clone(),
                    normal: maps.iter().any(|n| n.name == normal).then_some(normal),
                    scale: finest.0,
                },
            ))
        })
        .collect()
}

impl Scene {
    pub fn triangles(&self) -> usize {
        self.draws
            .iter()
            .map(|d| d.index_count as usize / 3 * d.instance_count as usize)
            .sum()
    }
}

fn instance(t: &Transform) -> Instance {
    let [r, u, f, o] = [t.right, t.up, t.forward, t.trans];
    [
        r[0], r[1], r[2], o[0], u[0], u[1], u[2], o[1], f[0], f[1], f[2], o[2],
    ]
}

/// The draws for `zone`, its meshes' sections from `load`. A mesh that fails to load,
/// or has no triangles, is left out and listed.
pub fn build(zone: &Zone, mut load: impl FnMut(&Mesh) -> frostbite::Result<Vec<Section>>) -> Scene {
    let mut scene = Scene::default();
    let mut texture_index = HashMap::new();
    let mut by_mesh: Vec<Vec<&Transform>> = vec![Vec::new(); zone.meshes.len()];
    for p in &zone.placements {
        // Level::zone only makes valid indices; a bad one is left out, not a panic.
        if let Some(list) = by_mesh.get_mut(p.mesh) {
            list.push(&p.transform);
        }
    }
    let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for (mesh, transforms) in zone.meshes.iter().zip(&by_mesh) {
        if transforms.is_empty() {
            continue;
        }
        let sections = match load(mesh) {
            Ok(s) => s,
            Err(e) => {
                scene.failed.push((mesh.name.clone(), e.0));
                scene.placements_left_out += transforms.len();
                continue;
            }
        };
        let (first_vertex, first_index) = (scene.positions.len(), scene.indices.len());
        let first_draw = scene.draws.len();
        let first_instance = scene.instances.len() as u32;
        for s in &sections {
            let g = &s.geometry;
            let vertex_offset = scene.positions.len() as i32;
            let index_start = scene.indices.len() as u32;
            scene.positions.extend_from_slice(&g.positions);
            match &g.uvs {
                Some(uvs) if uvs.len() == g.positions.len() => scene.uvs.extend_from_slice(uvs),
                _ => scene.uvs.resize(scene.positions.len(), [0.0; 2]),
            }
            match &g.colours {
                Some(c) if c.len() == g.positions.len() => scene.colours.extend_from_slice(c),
                _ => scene.colours.resize(scene.positions.len(), [255; 4]),
            }
            match &g.uvs2 {
                Some(uvs) if uvs.len() == g.positions.len() => scene.uvs2.extend_from_slice(uvs),
                _ => scene.uvs2.resize(scene.positions.len(), [0.0; 2]),
            }
            match &g.colours2 {
                Some(c) if c.len() == g.positions.len() => scene.colours2.extend_from_slice(c),
                _ => scene.colours2.resize(scene.positions.len(), [0; 4]),
            }
            // Asphalt without them samples its shoulder at zero, as its shader does.
            match &g.uvs3 {
                Some(uvs) if uvs.len() == g.positions.len() => scene.uvs3.extend_from_slice(uvs),
                _ => scene.uvs3.resize(scene.positions.len(), [0.0; 2]),
            }
            match &g.frames {
                Some(f) if f.len() == g.positions.len() => {
                    scene.normals.extend(f.iter().map(|f| normal(f.normal)));
                    scene.tangents.extend(f.iter().map(tangent));
                }
                _ => {
                    scene.normals.resize(scene.positions.len(), [0; 4]);
                    scene.tangents.resize(scene.positions.len(), [0; 4]);
                }
            }
            scene.indices.extend_from_slice(&g.indices);
            if g.indices.is_empty() {
                continue;
            }
            let mut index = |t: [u8; 16]| {
                *texture_index.entry(t).or_insert_with(|| {
                    scene.textures.push(t);
                    scene.textures.len() as u32 - 1
                })
            };
            // Without its own texture coordinates a section samples one texel.
            let texture = s.texture.filter(|_| g.uvs.is_some()).map(&mut index);
            // A normal map needs the section's frame to bend.
            let normal_map = s
                .normal_map
                .filter(|_| g.uvs.is_some() && g.frames.is_some())
                .map(&mut index);
            let mut layers = [None; 6];
            let shading = match s.shading {
                // What has no texture is drawn plain grey.
                _ if texture.is_none() => Shading::Colour,
                // Asphalt without its layers or its mask's coordinates is its clean
                // layer's grey.
                Shading::Asphalt(a) => {
                    if g.uvs2.is_some() && s.layers[..ASPHALT_NEEDS].iter().all(Option::is_some) {
                        let mut wanted = s.layers;
                        if a.edge.is_none() {
                            wanted[5] = None;
                        }
                        layers = wanted.map(|t| t.map(&mut index));
                        Shading::Asphalt(a)
                    } else {
                        Shading::Grey { tint: a.tint[0] }
                    }
                }
                shading => shading,
            };
            scene.draws.push(Draw {
                first_index: index_start,
                index_count: g.indices.len() as u32,
                vertex_offset,
                first_instance,
                instance_count: transforms.len() as u32,
                texture,
                layers,
                normal_map,
                shading,
            });
        }
        if scene.draws.len() == first_draw {
            scene.positions.truncate(first_vertex);
            scene.uvs.truncate(first_vertex);
            scene.colours.truncate(first_vertex);
            scene.uvs2.truncate(first_vertex);
            scene.colours2.truncate(first_vertex);
            scene.uvs3.truncate(first_vertex);
            scene.normals.truncate(first_vertex);
            scene.tangents.truncate(first_vertex);
            scene.indices.truncate(first_index);
            scene
                .failed
                .push((mesh.name.clone(), "no triangles".into()));
            scene.placements_left_out += transforms.len();
            continue;
        }
        for t in transforms {
            for k in 0..3 {
                min[k] = min[k].min(t.trans[k]);
                max[k] = max[k].max(t.trans[k]);
            }
        }
        scene
            .instances
            .extend(transforms.iter().map(|t| instance(t)));
    }
    if !scene.draws.is_empty() && min.iter().chain(&max).all(|v| v.is_finite()) {
        scene.bounds = Some((min, max));
    }
    scene
}

/// Adds `ground`, already in world space, as one draw in its vertices' colours with
/// `detail` (one per layer of `ground.detail`) over them, and its map for asphalt's
/// edges; it leaves the bounds, which are of the placements, as they are.
pub fn add_ground(scene: &mut Scene, ground: &Ground, detail: Vec<Detail>) {
    if ground.indices.is_empty() {
        return;
    }
    let vertex_offset = scene.positions.len() as i32;
    let first_index = scene.indices.len() as u32;
    scene.positions.extend_from_slice(&ground.positions);
    // The weights' 8 bytes go up as they are, in the texture coordinates' place.
    scene
        .uvs
        .extend(ground.weights.iter().take(ground.positions.len()).map(|w| {
            [0, 4].map(|i| f32::from_bits(u32::from_le_bytes(w[i..i + 4].try_into().expect("4"))))
        }));
    scene.uvs.resize(scene.positions.len(), [0.0; 2]);
    scene.detail = detail;
    // Ground::colours has one per position; a short list is made up in grey.
    scene
        .colours
        .extend(ground.colours.iter().take(ground.positions.len()));
    scene
        .colours
        .resize(scene.positions.len(), [128, 128, 128, 255]);
    scene.uvs2.resize(scene.positions.len(), [0.0; 2]);
    scene.colours2.resize(scene.positions.len(), [0; 4]);
    scene.uvs3.resize(scene.positions.len(), [0.0; 2]);
    scene.normals.resize(scene.positions.len(), [0; 4]);
    scene.tangents.resize(scene.positions.len(), [0; 4]);
    scene.ground_map = Some(ground.map.clone());
    scene.indices.extend_from_slice(&ground.indices);
    scene.draws.push(Draw {
        first_index,
        index_count: ground.indices.len() as u32,
        vertex_offset,
        first_instance: scene.instances.len() as u32,
        instance_count: 1,
        texture: None,
        layers: [None; 6],
        normal_map: None,
        shading: Shading::Ground,
    });
    scene.instances.push(instance(&Transform::IDENTITY));
}

/// Every section of the first LOD of the mesh asset `mesh`, with its material's
/// [`look`], from the store; asphalt takes the texture `noise` (the partition of
/// [`ASPHALT_NOISE`]) where its material names none. A section whose material is out
/// of range is drawn grey.
pub fn first_lod(
    meshes: &mut Meshes,
    mesh: &Mesh,
    noise: Option<[u8; 16]>,
) -> frostbite::Result<Vec<Section>> {
    let set = meshes.by_partition(&mesh.partition)?;
    let Some(lod) = set.lods.first() else {
        return Err(frostbite::Error(format!("{}: no LODs", mesh.name)));
    };
    let materials = meshes.materials(&mesh.partition)?;
    let data = meshes.chunk(lod)?;
    lod.sections
        .iter()
        .map(|s| {
            let look = materials.get(s.material as usize).and_then(|m| {
                let shader = m.shader.and_then(|g| meshes.store().partition_name(&g));
                look(shader.unwrap_or_default(), m)
            });
            let material = materials.get(s.material as usize);
            let (layers, normal_map) = match (look, material) {
                (Some((_, Shading::Asphalt(_))), Some(m)) => (asphalt_layers(m, noise), None),
                (Some((_, Shading::Grey { .. })), _) => ([None; 6], None),
                _ => ([None; 6], material.and_then(|m| m.texture("Normal"))),
            };
            Ok(Section {
                geometry: lod.section(s, &data)?,
                texture: look.map(|l| l.0),
                layers,
                normal_map,
                shading: look.map(|l| l.1).unwrap_or_default(),
            })
        })
        .collect()
}

/// A view of the whole scene from above one corner of its bounds, looking at the
/// middle; at the origin looking along +z when there is nothing to look at.
pub fn overview(scene: &Scene, width: u32, height: u32) -> crate::Camera {
    let Some((min, max)) = scene.bounds else {
        return crate::Camera::new([0.0; 3], 0.0, 0.0, width, height);
    };
    let centre = [0, 1, 2].map(|k| (min[k] + max[k]) / 2.0);
    let size = (max[0] - min[0]).max(max[2] - min[2]).max(1.0);
    let eye = [
        centre[0] - size * 0.45,
        max[1] + size * 0.3,
        centre[2] - size * 0.45,
    ];
    crate::Camera::looking_at(eye, centre, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use frostbite::level::Placement;
    use frostbite::texture::TextureParameter;

    fn mesh(name: &str) -> Mesh {
        Mesh {
            name: name.into(),
            kind: "RigidMeshAsset".into(),
            partition: [0; 16],
        }
    }

    fn at(mesh: usize, x: f32) -> Placement {
        let mut transform = Transform::IDENTITY;
        transform.trans = [x, 0.0, 0.0];
        Placement { mesh, transform }
    }

    fn triangle() -> Section {
        Section {
            geometry: Geometry {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                indices: vec![0, 1, 2],
                ..Geometry::default()
            },
            ..Section::default()
        }
    }

    /// A triangle with texture coordinates, coloured by `texture` as `shading` says.
    fn textured(texture: u8, shading: Shading) -> Section {
        let mut s = triangle();
        s.geometry.uvs = Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        s.texture = Some([texture; 16]);
        s.shading = shading;
        s
    }

    #[test]
    fn normals_are_signed_bytes_marked_present() {
        assert_eq!(normal([1.0, 0.0, -1.0]), [127, 0, -127, 127]);
        assert_eq!(normal([0.6, -0.8, 0.0]), [76, -102, 0, 127]);
    }

    #[test]
    fn tangents_carry_the_bitangent_sign() {
        let frame = |sign| frostbite::mesh::Frame {
            normal: [0.0, 1.0, 0.0],
            tangent: [0.6, 0.0, -0.8],
            sign,
        };
        assert_eq!(tangent(&frame(1.0)), [76, 0, -102, 127]);
        assert_eq!(tangent(&frame(-1.0)), [76, 0, -102, -127]);
    }

    #[test]
    fn a_normal_map_is_drawn_only_where_a_frame_bends_it() {
        let frames = Some(vec![
            frostbite::mesh::Frame {
                normal: [0.0, 0.0, 1.0],
                tangent: [1.0, 0.0, 0.0],
                sign: -1.0,
            };
            3
        ]);
        let mapped = |frames: Option<Vec<frostbite::mesh::Frame>>| {
            let mut s = textured(1, Shading::Colour);
            s.normal_map = Some([2; 16]);
            s.geometry.frames = frames;
            s
        };
        let mut no_uvs = mapped(frames.clone());
        no_uvs.geometry.uvs = None;
        let zone = Zone {
            meshes: vec![mesh("framed"), mesh("plain"), mesh("no uvs")],
            placements: vec![at(0, 0.0), at(1, 0.0), at(2, 0.0)],
            ..Zone::default()
        };
        let scene = build(&zone, |m| {
            Ok(vec![match m.name.as_str() {
                "framed" => mapped(frames.clone()),
                "plain" => mapped(None),
                _ => no_uvs.clone(),
            }])
        });
        let maps: Vec<_> = scene.draws.iter().map(|d| d.normal_map).collect();
        assert_eq!(maps, [Some(1), None, None]);
        assert_eq!(scene.textures, [[1; 16], [2; 16]]);
        assert_eq!(scene.tangents.len(), scene.positions.len());
        assert_eq!(
            scene.tangents[..4],
            [
                [127, 0, 0, -127],
                [127, 0, 0, -127],
                [127, 0, 0, -127],
                [0; 4]
            ]
        );
    }

    #[test]
    fn sections_without_frames_take_no_normal() {
        let mut framed = triangle();
        framed.geometry.frames = Some(vec![
            frostbite::mesh::Frame {
                normal: [0.0, 0.0, 1.0],
                tangent: [1.0, 0.0, 0.0],
                sign: 1.0,
            };
            3
        ]);
        let zone = Zone {
            meshes: vec![mesh("plain"), mesh("framed")],
            placements: vec![at(0, 0.0), at(1, 0.0)],
            ..Zone::default()
        };
        let scene = build(&zone, |m| {
            Ok(vec![if m.name == "framed" {
                framed.clone()
            } else {
                triangle()
            }])
        });
        assert_eq!(scene.normals.len(), scene.positions.len());
        assert_eq!(
            scene.normals,
            [
                [0; 4],
                [0; 4],
                [0; 4],
                [0, 0, 127, 127],
                [0, 0, 127, 127],
                [0, 0, 127, 127]
            ]
        );
    }

    fn draw(first_index: u32, vertex_offset: i32, first_instance: u32, instances: u32) -> Draw {
        Draw {
            first_index,
            index_count: 3,
            vertex_offset,
            first_instance,
            instance_count: instances,
            texture: None,
            layers: [None; 6],
            normal_map: None,
            shading: Shading::Colour,
        }
    }

    #[test]
    fn packs_meshes_once_and_placements_as_instances() {
        let zone = Zone {
            meshes: vec![mesh("a"), mesh("b"), mesh("unplaced")],
            placements: vec![at(1, 10.0), at(0, 0.0), at(1, 20.0)],
            ..Zone::default()
        };
        let mut loaded = Vec::new();
        let scene = build(&zone, |m| {
            loaded.push(m.name.clone());
            // b has two sections, each drawn on its own, from its own first vertex.
            Ok(match m.name.as_str() {
                "b" => vec![triangle(), triangle()],
                _ => vec![triangle()],
            })
        });
        assert_eq!(loaded, ["a", "b"]);
        assert_eq!(scene.positions.len(), 9);
        assert_eq!(scene.uvs, [[0.0; 2]; 9]);
        assert_eq!(scene.colours, [[255; 4]; 9]);
        assert_eq!(scene.indices, [0, 1, 2, 0, 1, 2, 0, 1, 2]);
        assert_eq!(
            scene.draws,
            [draw(0, 0, 0, 1), draw(3, 3, 1, 2), draw(6, 6, 1, 2)]
        );
        // b's instances in placement order, the translation in each row's last value.
        assert_eq!(scene.instances[1][3], 10.0);
        assert_eq!(scene.instances[2][3], 20.0);
        assert_eq!(scene.instances[2][..3], [1.0, 0.0, 0.0]);
        assert_eq!(scene.triangles(), 1 + 2 * 2);
        assert!(scene.textures.is_empty());
        assert_eq!(scene.bounds, Some(([0.0; 3], [20.0, 0.0, 0.0])));
        assert!(scene.failed.is_empty());
    }

    #[test]
    fn the_ground_is_one_draw_in_its_vertex_colours_after_the_meshes() {
        let zone = Zone {
            meshes: vec![mesh("a")],
            placements: vec![at(0, 5.0)],
            ..Zone::default()
        };
        let mut scene = build(&zone, |_| Ok(vec![triangle()]));
        let ground = Ground {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
            indices: vec![0, 2, 1, 0, 3, 2],
            // One short: the last is grey, with no weights.
            colours: vec![[10, 20, 30, 255]; 3],
            weights: vec![[1, 2, 3, 4, 5, 6, 7, 8]; 3],
            ..Ground::default()
        };
        let detail = vec![Detail {
            texture: "grass".into(),
            normal: None,
            scale: 0.2,
        }];
        add_ground(&mut scene, &ground, detail.clone());
        assert_eq!(scene.positions.len(), 7);
        assert_eq!(scene.uvs.len(), 7);
        let bytes: &[u8] = bytemuck::cast_slice(&scene.uvs[3..]);
        assert_eq!(bytes[..8], [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(bytes[24..], [0; 8]);
        assert_eq!(scene.detail, detail);
        assert_eq!(scene.colours[3..6], [[10, 20, 30, 255]; 3]);
        assert_eq!(scene.colours[6], [128, 128, 128, 255]);
        assert_eq!(scene.indices[3..], [0, 2, 1, 0, 3, 2]);
        let ground_draw = Draw {
            index_count: 6,
            shading: Shading::Ground,
            ..draw(3, 3, 1, 1)
        };
        assert_eq!(scene.draws[1], ground_draw);
        assert_eq!(
            scene.instances[1],
            [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(scene.triangles(), 3);
        assert_eq!(scene.bounds, Some(([5.0, 0.0, 0.0], [5.0, 0.0, 0.0])));
        // Nothing to draw adds nothing.
        let before = scene.clone();
        add_ground(&mut scene, &Ground::default(), Vec::new());
        assert_eq!(scene, before);
    }

    #[test]
    fn each_layers_detail_is_its_finest_scaled_diffuse_but_a_road_sets() {
        use frostbite::layers::Texture;
        let d = |name: &str, scale| Texture {
            name: format!("textures/{name}"),
            scale,
        };
        let textures = BTreeMap::from([
            (1, vec![d("grass_d", Some(0.2)), d("grass_n", Some(0.2))]),
            (
                3,
                vec![
                    d("footprints_d", Some(0.2)),
                    d("footprints_smd", Some(0.2)),
                    d("waves_d", None),
                    d("waves_n", None),
                ],
            ),
            (
                6,
                vec![
                    d("burnt_01_d", Some(0.1)),
                    d("burnt_02_d", Some(0.1)),
                    d("burnt_02_n", Some(0.1)),
                ],
            ),
            (12, vec![d("pine_d", Some(0.48)), d("field_d", Some(0.4))]),
            (8, vec![d("drv_set3_clean_d", Some(0.1))]),
            (28, vec![d("gravel_d", None), d("gravel_n", Some(0.1))]),
            (30, vec![d("rock_n", Some(0.01))]),
        ]);
        let detail = ground_detail(&textures);
        let pick = |layer| {
            let d = &detail[&layer];
            (d.texture.as_str(), d.normal.as_deref(), d.scale)
        };
        assert_eq!(pick(1), ("textures/grass_d", Some("textures/grass_n"), 0.2));
        assert_eq!(pick(3), ("textures/footprints_d", None, 0.2));
        // The normal map goes with the diffuse chosen, not another of the layer's.
        assert_eq!(pick(6), ("textures/burnt_01_d", None, 0.1));
        assert_eq!(pick(12), ("textures/pine_d", None, 0.48));
        assert_eq!(detail.keys().copied().collect::<Vec<_>>(), [1, 3, 6, 12]);
    }

    #[test]
    fn meshes_that_fail_are_listed_not_drawn() {
        let zone = Zone {
            meshes: vec![mesh("broken"), mesh("empty"), mesh("good")],
            placements: vec![at(0, 0.0), at(0, 1.0), at(1, 0.0), at(2, 5.0), at(9, 0.0)],
            ..Zone::default()
        };
        let scene = build(&zone, |m| match m.name.as_str() {
            "broken" => Err(frostbite::Error("no chunk".into())),
            "empty" => Ok(vec![Section {
                geometry: Geometry {
                    positions: vec![[0.0; 3]],
                    ..Geometry::default()
                },
                ..Section::default()
            }]),
            _ => Ok(vec![triangle()]),
        });
        assert_eq!(
            scene.failed,
            [
                ("broken".to_owned(), "no chunk".to_owned()),
                ("empty".to_owned(), "no triangles".to_owned())
            ]
        );
        assert_eq!(scene.placements_left_out, 3);
        assert_eq!(scene.positions.len(), 3);
        assert_eq!(scene.uvs.len(), 3);
        assert_eq!(scene.colours.len(), 3);
        assert_eq!(scene.draws.len(), 1);
        assert_eq!(scene.draws[0].vertex_offset, 0);
    }

    #[test]
    fn sections_name_their_textures_once() {
        let zone = Zone {
            meshes: vec![mesh("a"), mesh("b")],
            placements: vec![at(0, 0.0), at(1, 0.0)],
            ..Zone::default()
        };
        let decal = Shading::Decal {
            select: [1.0, 0.0, 0.0, 0.0],
            opacity: 0.5,
            colour: None,
        };
        let scene = build(&zone, |m| {
            Ok(match m.name.as_str() {
                // A section without texture coordinates is drawn plain grey, its
                // texture not loaded.
                "a" => vec![textured(7, Shading::Colour), {
                    let mut s = textured(9, decal);
                    s.geometry.uvs = None;
                    s
                }],
                _ => vec![textured(8, Shading::Cutout), {
                    let mut s = textured(7, decal);
                    s.geometry.colours = Some(vec![[255, 146, 19, 255]; 3]);
                    s
                }],
            })
        });
        assert_eq!(scene.textures, [[7; 16], [8; 16]]);
        let used: Vec<_> = scene.draws.iter().map(|d| (d.texture, d.shading)).collect();
        assert_eq!(
            used,
            [
                (Some(0), Shading::Colour),
                (None, Shading::Colour),
                (Some(1), Shading::Cutout),
                (Some(0), decal)
            ]
        );
        assert_eq!(scene.uvs[1], [1.0, 0.0]);
        assert_eq!(scene.uvs[4], [0.0; 2]);
        assert_eq!(scene.colours[11], [255, 146, 19, 255]);
        assert_eq!(scene.colours[8], [255; 4]);
        assert_eq!(scene.colours[5], [255; 4]);
    }

    fn material(textures: &[(&str, Option<u8>)], vectors: &[(&str, [f32; 4])]) -> Material {
        Material {
            shader: None,
            textures: textures
                .iter()
                .map(|&(name, t)| TextureParameter {
                    name: name.into(),
                    texture: t.map(|t| [t; 16]),
                })
                .collect(),
            vectors: vectors.iter().map(|&(n, v)| (n.into(), v)).collect(),
        }
    }

    #[test]
    fn the_colour_is_the_first_named_texture_set() {
        let look =
            |textures: &[(&str, Option<u8>)]| look("shaders/props", &material(textures, &[]));
        assert_eq!(
            look(&[("Normal", Some(1)), ("Diffuse", Some(2))]),
            Some(([2; 16], Shading::Colour))
        );
        assert_eq!(
            look(&[("Diffuse", None), ("ColorAlpha", Some(3))]),
            Some(([3; 16], Shading::Cutout))
        );
        assert_eq!(
            look(&[("Diffuse_Tiling", Some(4)), ("Diffuse", Some(5))]),
            Some(([5; 16], Shading::Colour))
        );
        assert_eq!(look(&[("Normal", Some(1)), ("SMD", Some(2))]), None);
    }

    #[test]
    fn asphalt_is_its_foreground_layer_tinted() {
        let road = material(
            &[
                ("Shoulder_NH", Some(1)),
                ("Tarmac_FG_Diffuse", Some(2)),
                ("Tarmac_BG_Diffuse", Some(3)),
            ],
            &[
                ("Tarmac_Tint_BG", [0.5, 0.5, 0.5, 0.0]),
                ("Tarmac_Tint_FG", [0.57, 0.57, 0.57, 0.0]),
            ],
        );
        let shader = "shaders/world/roads/rd_drv_blendwithterrain";
        assert_eq!(
            look(shader, &road),
            Some((
                [2; 16],
                Shading::Grey {
                    tint: [0.57, 0.57, 0.57]
                }
            ))
        );
        // Without a tint, untinted.
        let road = material(&[("Tarmac_FG_Diffuse", Some(2))], &[]);
        assert_eq!(
            look(shader, &road),
            Some(([2; 16], Shading::Grey { tint: [1.0; 3] }))
        );
    }

    #[test]
    fn asphalt_gives_way_to_the_ground_only_with_its_blending_shader() {
        let layers = [
            ("Tarmac_FG_Diffuse", Some(2)),
            ("Tarmac_BG_Diffuse", Some(3)),
            ("Tarmac_BG_Normal", Some(4)),
            ("detailMask", Some(5)),
            ("Shoulder_NH", Some(6)),
        ];
        let road = material(
            &layers,
            &[
                ("Shoulder_Tiling", [2.0, 2.0, 0.0, 0.0]),
                ("Dirt_Height_Offset", [0.3, 0.0, 0.0, 0.0]),
            ],
        );
        let edge = |shader: &str, m: &Material| match look(shader, m) {
            Some((_, Shading::Asphalt(a))) => a.edge,
            other => panic!("not asphalt: {other:?}"),
        };
        // The material's tiling and offset, the shader's contrast and paint boost.
        assert_eq!(
            edge("shaders/world/roads/rd_drv_blendwithterrain", &road),
            Some(Edge {
                tiling: [2.0, 2.0],
                dirt: [0.3, 0.814_847, 0.0],
            })
        );
        // The wet road's shader draws it to its edges.
        assert_eq!(
            edge("shaders/world/roads/rd_drv_basesurface_wetness", &road),
            None
        );
        // So does a material without a shoulder.
        let road = material(&layers[..4], &[]);
        assert_eq!(
            edge("shaders/world/roads/rd_drv_blendwithterrain", &road),
            None
        );
    }

    #[test]
    fn road_decals_blend_by_their_selected_channel() {
        let shader = "wstroads/rd_dcl_transparentdcl_lerp";
        let lines = material(
            &[
                ("Diffuse", Some(4)),
                ("Normal", Some(5)),
                ("detailMask", Some(6)),
            ],
            &[
                ("alphaSelector", [0.0, 1.0, 0.0, 0.0]),
                ("Opacity", [1.0, 0.0, 0.0, 0.0]),
            ],
        );
        assert_eq!(
            look(shader, &lines),
            Some((
                [4; 16],
                Shading::Decal {
                    select: [0.0, 1.0, 0.0, 0.0],
                    opacity: 1.0,
                    colour: None
                }
            ))
        );
        // A line colour paints it; without a selector or opacity, the alpha, whole.
        let marks = material(
            &[("Diffuse", Some(4))],
            &[("lineColor", [0.003, 0.003, 0.003, 0.0])],
        );
        assert_eq!(
            look(
                "shaders/world/roads/rd_dcl_transparentdcl_lerpnsmultiply",
                &marks
            ),
            Some((
                [4; 16],
                Shading::Decal {
                    select: [0.0, 0.0, 0.0, 1.0],
                    opacity: 1.0,
                    colour: Some([0.003; 3])
                }
            ))
        );
        // A decal without a colour texture is not drawn with another one.
        let bare = material(&[("Normal", Some(5))], &[]);
        assert_eq!(look(shader, &bare), None);
        // The same parameters under another shader are an ordinary colour.
        assert_eq!(
            look("shaders/world/roads/rd_dcl_base_parallax", &lines),
            Some(([4; 16], Shading::Colour))
        );
    }

    #[test]
    fn nothing_to_draw_has_no_bounds() {
        let scene = build(&Zone::default(), |_| Ok(vec![triangle()]));
        assert_eq!(scene, Scene::default());
        let c = overview(&scene, 640, 480);
        assert_eq!(c.origin, [0.0; 3]);
    }

    /// Builds zone 495 (the speedway) from the store `NFSH_STORE` points at (local-ci
    /// sets it on the Windows side): every mesh it places is drawn.
    #[test]
    fn builds_the_speedway_zone() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = frostbite::store::Store::open(std::path::Path::new(&dir)).unwrap();
        let zone = frostbite::level::Level::new(&store, "Win32/levels/mainlevel/mainlevel")
            .unwrap()
            .zone("levels/mainlevel/worldart_zs_495")
            .unwrap();
        let mut meshes = Meshes::new(&store).unwrap();
        let noise = texture_partition(&store, ASPHALT_NOISE);
        let scene = build(&zone, |m| first_lod(&mut meshes, m, noise));
        assert_eq!(scene.failed, []);
        // Counted on the owner's store, 2026-10-31; the textures again on 2026-11-01,
        // with the normal maps (109 of them).
        assert_eq!(zone.placements.len(), 2171);
        assert_eq!(scene.draws.len(), 780);
        assert_eq!(scene.textures.len(), 238);
        assert_eq!(scene.instances.len(), 2171);
        assert_eq!(scene.positions.len(), 1_356_354);
        assert_eq!(scene.triangles(), 3_988_832);
    }

    /// The roads near zone 495, from the store `NFSH_STORE` points at: asphalt is
    /// drawn with its layers and road lines, cracks and tyre marks as decals.
    #[test]
    fn the_speedway_roads_are_asphalt_and_decals() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = frostbite::store::Store::open(std::path::Path::new(&dir)).unwrap();
        let mut level =
            frostbite::level::Level::new(&store, "Win32/levels/mainlevel/mainlevel").unwrap();
        let mut zone = level.zone("levels/mainlevel/worldart_zs_495").unwrap();
        level.add_drivable_surfaces(&mut zone, 150.0).unwrap();
        let mut meshes = Meshes::new(&store).unwrap();
        let noise = texture_partition(&store, ASPHALT_NOISE);
        let scene = build(&zone, |m| first_lod(&mut meshes, m, noise));
        assert_eq!(scene.failed, []);
        let mut count = [0; 5];
        for d in &scene.draws {
            count[match d.shading {
                Shading::Colour | Shading::Ground => 0,
                Shading::Cutout => 1,
                Shading::Grey { .. } => 2,
                Shading::Asphalt(_) => 3,
                Shading::Decal { .. } => 4,
            }] += 1;
        }
        let painted = scene
            .draws
            .iter()
            .filter(|d| matches!(d.shading, Shading::Decal { colour: None, .. }))
            .count();
        let edged = scene
            .draws
            .iter()
            .filter(|d| matches!(d.shading, Shading::Asphalt(a) if a.edge.is_some()))
            .count();
        // Counted on the owner's store, 2026-11-01: 88 asphalt sections, all with
        // their layers (the 36 with positions only are plain grey), 58 of them giving
        // way to the ground at their edges (the 30 others are the wet road's shader);
        // 130 decals, 53 of them lines coloured by their vertices; the asphalt's
        // layers add 22 textures, and the normal maps 132.
        assert_eq!(count, [1047, 20, 0, 88, 130]);
        assert_eq!(edged, 58);
        assert_eq!(painted, 53);
        assert_eq!(scene.textures.len(), 310);
    }
}
