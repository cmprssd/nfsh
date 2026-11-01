//! A zone made ready to draw, on the CPU: every placed mesh's first LOD packed into
//! one vertex and one index buffer, its placements into one instance buffer, and one
//! instanced draw per section, with the section's diffuse texture.

use std::collections::HashMap;

use frostbite::level::{Mesh, Transform, Zone};
use frostbite::mesh::{Geometry, Meshes};
use frostbite::terrain::Ground;
use frostbite::texture::TextureParameter;

/// A placement's transform as the vertex shader reads it: right, up and forward,
/// each followed by one coordinate of the translation.
pub type Instance = [f32; 12];

/// One instanced indexed draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    pub first_index: u32,
    pub index_count: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
    pub instance_count: u32,
    /// Index into [`Scene::textures`]; `None` draws plain grey.
    pub texture: Option<u32>,
    /// Pixels whose texture alpha is under a half are not drawn (leaves).
    pub cutout: bool,
}

/// A mesh section and what it is drawn with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    pub geometry: Geometry,
    /// The `TextureAsset` partition of its diffuse colour; `None` for none, or when
    /// the section has no texture coordinates.
    pub texture: Option<[u8; 16]>,
    pub cutout: bool,
}

/// The texture parameters that hold a material's colour, in the order they are
/// looked for, and whether the colour's alpha cuts the surface out.
const COLOUR: [(&str, bool); 4] = [
    ("Diffuse", false),
    ("ColorAlpha", true),
    ("Diffuse_Tiling", false),
    ("foregroundBasecolor", false),
];

/// The colour texture of a material with the texture parameters `parameters`, and
/// whether it cuts out.
pub fn colour_texture(parameters: &[TextureParameter]) -> Option<([u8; 16], bool)> {
    COLOUR.iter().find_map(|&(name, cutout)| {
        let p = parameters.iter().find(|p| p.name == name)?;
        Some((p.texture?, cutout))
    })
}

/// What a zone draws, and what it could not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub positions: Vec<[f32; 3]>,
    /// One per position; zero where a section has none.
    pub uvs: Vec<[f32; 2]>,
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
    /// Meshes that could not be read, with why, and how many placements that left
    /// out.
    pub failed: Vec<(String, String)>,
    pub placements_left_out: usize,
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
            scene.indices.extend_from_slice(&g.indices);
            if g.indices.is_empty() {
                continue;
            }
            // Without its own texture coordinates a section samples one texel.
            let texture = s.texture.filter(|_| g.uvs.is_some()).map(|t| {
                *texture_index.entry(t).or_insert_with(|| {
                    scene.textures.push(t);
                    scene.textures.len() as u32 - 1
                })
            });
            scene.draws.push(Draw {
                first_index: index_start,
                index_count: g.indices.len() as u32,
                vertex_offset,
                first_instance,
                instance_count: transforms.len() as u32,
                texture,
                cutout: s.cutout && texture.is_some(),
            });
        }
        if scene.draws.len() == first_draw {
            scene.positions.truncate(first_vertex);
            scene.uvs.truncate(first_vertex);
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

/// Adds `ground`, already in world space, as one draw in plain grey; it leaves the
/// bounds, which are of the placements, as they are.
pub fn add_ground(scene: &mut Scene, ground: &Ground) {
    if ground.indices.is_empty() {
        return;
    }
    let vertex_offset = scene.positions.len() as i32;
    let first_index = scene.indices.len() as u32;
    scene.positions.extend_from_slice(&ground.positions);
    scene.uvs.resize(scene.positions.len(), [0.0; 2]);
    scene.indices.extend_from_slice(&ground.indices);
    scene.draws.push(Draw {
        first_index,
        index_count: ground.indices.len() as u32,
        vertex_offset,
        first_instance: scene.instances.len() as u32,
        instance_count: 1,
        texture: None,
        cutout: false,
    });
    scene.instances.push(instance(&Transform::IDENTITY));
}

/// Every section of the first LOD of the mesh asset `mesh`, with its material's
/// colour texture, from the store. A section whose material is out of range is drawn
/// grey.
pub fn first_lod(meshes: &mut Meshes, mesh: &Mesh) -> frostbite::Result<Vec<Section>> {
    let set = meshes.by_partition(&mesh.partition)?;
    let Some(lod) = set.lods.first() else {
        return Err(frostbite::Error(format!("{}: no LODs", mesh.name)));
    };
    let materials = meshes.materials(&mesh.partition)?;
    let data = meshes.chunk(lod)?;
    lod.sections
        .iter()
        .map(|s| {
            let colour = materials
                .get(s.material as usize)
                .and_then(|m| colour_texture(m));
            Ok(Section {
                geometry: lod.section(s, &data)?,
                texture: colour.map(|c| c.0),
                cutout: colour.is_some_and(|c| c.1),
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
                uvs: None,
                indices: vec![0, 1, 2],
            },
            ..Section::default()
        }
    }

    /// A triangle with texture coordinates, coloured by `texture`.
    fn textured(texture: u8, cutout: bool) -> Section {
        let mut s = triangle();
        s.geometry.uvs = Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        s.texture = Some([texture; 16]);
        s.cutout = cutout;
        s
    }

    fn draw(first_index: u32, vertex_offset: i32, first_instance: u32, instances: u32) -> Draw {
        Draw {
            first_index,
            index_count: 3,
            vertex_offset,
            first_instance,
            instance_count: instances,
            texture: None,
            cutout: false,
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
    fn the_ground_is_one_grey_draw_after_the_meshes() {
        let zone = Zone {
            meshes: vec![mesh("a")],
            placements: vec![at(0, 5.0)],
            ..Zone::default()
        };
        let mut scene = build(&zone, |_| Ok(vec![triangle()]));
        let ground = Ground {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
            indices: vec![0, 2, 1, 0, 3, 2],
            ..Ground::default()
        };
        add_ground(&mut scene, &ground);
        assert_eq!(scene.positions.len(), 7);
        assert_eq!(scene.uvs.len(), 7);
        assert_eq!(scene.indices[3..], [0, 2, 1, 0, 3, 2]);
        let ground_draw = Draw {
            index_count: 6,
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
        add_ground(&mut scene, &Ground::default());
        assert_eq!(scene, before);
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
        let scene = build(&zone, |m| {
            Ok(match m.name.as_str() {
                // A section without texture coordinates is drawn grey, its texture
                // not loaded.
                "a" => vec![textured(7, false), {
                    let mut s = textured(9, false);
                    s.geometry.uvs = None;
                    s
                }],
                _ => vec![textured(8, true), textured(7, false)],
            })
        });
        assert_eq!(scene.textures, [[7; 16], [8; 16]]);
        let used: Vec<_> = scene.draws.iter().map(|d| (d.texture, d.cutout)).collect();
        assert_eq!(
            used,
            [
                (Some(0), false),
                (None, false),
                (Some(1), true),
                (Some(0), false)
            ]
        );
        assert_eq!(scene.uvs[1], [1.0, 0.0]);
        assert_eq!(scene.uvs[4], [0.0; 2]);
    }

    #[test]
    fn the_colour_is_the_first_named_texture_set() {
        let p = |name: &str, t: Option<u8>| TextureParameter {
            name: name.into(),
            texture: t.map(|t| [t; 16]),
        };
        assert_eq!(
            colour_texture(&[p("Normal", Some(1)), p("Diffuse", Some(2))]),
            Some(([2; 16], false))
        );
        assert_eq!(
            colour_texture(&[p("Diffuse", None), p("ColorAlpha", Some(3))]),
            Some(([3; 16], true))
        );
        assert_eq!(
            colour_texture(&[p("Diffuse_Tiling", Some(4)), p("Diffuse", Some(5))]),
            Some(([5; 16], false))
        );
        assert_eq!(
            colour_texture(&[p("Normal", Some(1)), p("SMD", Some(2))]),
            None
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
        let scene = build(&zone, |m| first_lod(&mut meshes, m));
        assert_eq!(scene.failed, []);
        // Counted on the owner's store, 2026-10-31.
        assert_eq!(zone.placements.len(), 2171);
        assert_eq!(scene.draws.len(), 780);
        assert_eq!(scene.textures.len(), 129);
        assert_eq!(scene.instances.len(), 2171);
        assert_eq!(scene.positions.len(), 1_356_354);
        assert_eq!(scene.triangles(), 3_988_832);
    }
}
