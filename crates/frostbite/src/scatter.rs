//! Palm City's terrain scattering: the meshes (grass, bushes, rocks, twigs) each
//! terrain layer scatters over the ground, and their settings, from the terrain's
//! `VisualResource` (see docs/FORMATS.md, "Scattering").
//! Little-endian, packed with no alignment.

use crate::reader::Reader;
use crate::store::Store;
use crate::{Error, Result, bail};

/// The res holding the scattering (in the terrain's root bundle; the `TerrainData`
/// ebx of the same name names it as its `VisualResource`).
pub const VISUAL_RESOURCE: &str = "levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048";

/// The header before the layers, not worked out; the layer count follows it.
const HEADER: usize = 0xC2;
/// More layers than this is not a scattering res (the layer tree's index is a u16, but
/// Palm City has 31 and a shader name's layer bits stop at 32).
const MAX_LAYERS: usize = 64;
/// A scatter type's bytes after its identifier.
const SETTINGS: usize = 173;

/// Every terrain layer's scatter types.
#[derive(Debug, Clone, PartialEq)]
pub struct Scattering {
    /// By layer, the index the layer tree and the shaders' names use (not the
    /// `TerrainData`'s `TerrainLayers` order): its scatter types, often none.
    pub layers: Vec<Vec<ScatterType>>,
}

/// One mesh a layer scatters and how: the compiled form of a
/// `TerrainMeshScatteringType` ebx instance, whose field names these take.
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterType {
    /// The mesh asset's ebx name (a `RigidMeshAsset`).
    pub mesh: String,
    /// That asset's `MeshSetResource`.
    pub mesh_set: u64,
    /// The ebx instance's `Identifier`: its place in the res, from 0.
    pub identifier: u32,
    /// Three floats some types carry (1, 1, 1 on most), not worked out.
    pub triple: Option<[f32; 3]>,
    /// `Density` at the quality levels Low, Medium, High and Ultra.
    pub density: [f32; 4],
    /// `MinScale` and `MaxScale`, x and y each.
    pub min_scale: [f32; 2],
    pub max_scale: [f32; 2],
    pub scale_randomness: f32,
    pub wind_scale: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub wind_wiggle: f32,
    pub dissolve_range_ratio: f32,
    /// Four increasing distances, one for each of four levels (15, 20, 25 and 30 most
    /// often); likely metres to each level of detail, not checked.
    pub distances: [f32; 4],
}

impl Scattering {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::at(data, HEADER)?;
        let count = r.u32_le()? as usize;
        if count > MAX_LAYERS {
            bail!("{count} layers, more than {MAX_LAYERS}");
        }
        let mut layers = Vec::with_capacity(count);
        for layer in 0..count {
            let types = r.u32_le()? as usize;
            // A scatter type takes at least its settings' bytes.
            if types > data.len() / SETTINGS {
                bail!("layer {layer}: {types} scatter types can't fit");
            }
            let mut list = Vec::with_capacity(types);
            for _ in 0..types {
                list.push(
                    scatter_type(&mut r)
                        .map_err(|e| Error(format!("layer {layer}, type {}: {e}", list.len())))?,
                );
            }
            // Two bytes end a layer (0, 0; 0, 1 after the last), not worked out.
            r.bytes(2)?;
            layers.push(list);
        }
        if r.pos() != data.len() {
            bail!("{} bytes after the last layer", data.len() - r.pos());
        }
        Ok(Self { layers })
    }

    /// Reads Palm City's scattering from `store`.
    pub fn read(store: &Store) -> Result<Self> {
        let asset = store
            .res(VISUAL_RESOURCE)
            .ok_or_else(|| Error(format!("{VISUAL_RESOURCE}: not in the store")))?;
        let data = store.entries()?.read(asset.location)?;
        Self::parse(&data).map_err(|e| Error(format!("{VISUAL_RESOURCE}: {e}")))
    }

    /// Every scatter type, layer by layer.
    pub fn types(&self) -> impl Iterator<Item = &ScatterType> {
        self.layers.iter().flatten()
    }
}

fn f32_le(r: &mut Reader) -> Result<f32> {
    Ok(f32::from_bits(r.u32_le()?))
}

fn f32s<const N: usize>(r: &mut Reader) -> Result<[f32; N]> {
    let mut v = [0.0; N];
    for x in &mut v {
        *x = f32_le(r)?;
    }
    Ok(v)
}

/// Reads the scatter type at `r`: its mesh's name, ids, the optional triple, its
/// identifier, then [`SETTINGS`] bytes of which the named fields are read.
fn scatter_type(r: &mut Reader) -> Result<ScatterType> {
    let mesh = r.cstr()?.to_owned();
    let mesh_set = r.u64_le()?;
    // A u32 (nonzero on 22 types) and a zero u32, not worked out.
    r.bytes(8)?;
    let triple = match r.u8()? {
        0 => None,
        1 => Some(f32s::<3>(r)?),
        f => bail!("{mesh}: triple flag {f}"),
    };
    r.u8()?;
    let identifier = r.u32_le()?;
    let s = r.bytes(SETTINGS)?;
    let at = |pos: usize| Reader::at(s, pos);
    Ok(ScatterType {
        mesh,
        mesh_set,
        identifier,
        triple,
        density: f32s::<4>(&mut at(0)?)?,
        min_scale: f32s::<2>(&mut at(37)?)?,
        max_scale: f32s::<2>(&mut at(45)?)?,
        scale_randomness: f32_le(&mut at(61)?)?,
        wind_scale: f32_le(&mut at(65)?)?,
        stiffness: f32_le(&mut at(69)?)?,
        damping: f32_le(&mut at(73)?)?,
        mass: f32_le(&mut at(77)?)?,
        wind_wiggle: f32_le(&mut at(81)?)?,
        dissolve_range_ratio: f32_le(&mut at(85)?)?,
        distances: f32s::<4>(&mut at(138)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;

    use crate::ebx::{Partition, Value};
    use crate::level::{elements, field};

    fn put_f32s(out: &mut Vec<u8>, v: &[f32]) {
        v.iter()
            .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    }

    /// One scatter type's bytes: `name`, mesh set `set`, an optional triple, the
    /// identifier `id`, and settings whose named fields count up from `id`.
    fn scatter_bytes(name: &str, set: u64, triple: Option<[f32; 3]>, id: u32) -> Vec<u8> {
        let mut d = name.as_bytes().to_vec();
        d.push(0);
        d.extend_from_slice(&set.to_le_bytes());
        d.extend_from_slice(&[0; 8]);
        match triple {
            Some(t) => {
                d.push(1);
                put_f32s(&mut d, &t);
            }
            None => d.push(0),
        }
        d.push(0);
        d.extend_from_slice(&id.to_le_bytes());
        let start = d.len();
        d.resize(start + SETTINGS, 0xAB);
        let f = id as f32;
        let mut put = |pos: usize, v: &[f32]| {
            for (k, x) in v.iter().enumerate() {
                d[start + pos + 4 * k..][..4].copy_from_slice(&x.to_le_bytes());
            }
        };
        put(0, &[f, f + 0.1, f + 0.2, f + 0.3]);
        put(37, &[1.0, 2.0, 3.0, 4.0]);
        put(61, &[5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]);
        put(138, &[15.0, 20.0, 25.0, 30.0]);
        d
    }

    /// A res of three layers: two types, none, one.
    fn sample() -> Vec<u8> {
        let mut d = vec![0x5A; HEADER];
        d.extend_from_slice(&3u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend(scatter_bytes(
            "a/grass_mesh",
            0x1122,
            Some([1.0, 1.0, 0.5]),
            0,
        ));
        d.extend(scatter_bytes("b/bush_mesh", 0x3344, None, 1));
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend(scatter_bytes("c/rock_mesh", 0x5566, None, 2));
        d.extend_from_slice(&[0, 1]);
        d
    }

    #[test]
    fn reads_each_layers_scatter_types_and_their_settings() {
        let s = Scattering::parse(&sample()).unwrap();
        let shape: Vec<Vec<(&str, u64, u32)>> = s
            .layers
            .iter()
            .map(|l| {
                l.iter()
                    .map(|t| (t.mesh.as_str(), t.mesh_set, t.identifier))
                    .collect()
            })
            .collect();
        assert_eq!(
            shape,
            [
                vec![("a/grass_mesh", 0x1122, 0), ("b/bush_mesh", 0x3344, 1)],
                vec![],
                vec![("c/rock_mesh", 0x5566, 2)],
            ]
        );
        let t = &s.layers[0][0];
        assert_eq!(t.triple, Some([1.0, 1.0, 0.5]));
        assert_eq!(s.layers[0][1].triple, None);
        assert_eq!(t.density, [0.0, 0.1, 0.2, 0.3]);
        assert_eq!((t.min_scale, t.max_scale), ([1.0, 2.0], [3.0, 4.0]));
        assert_eq!(
            [
                t.scale_randomness,
                t.wind_scale,
                t.stiffness,
                t.damping,
                t.mass,
                t.wind_wiggle,
                t.dissolve_range_ratio
            ],
            [5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]
        );
        assert_eq!(t.distances, [15.0, 20.0, 25.0, 30.0]);
        assert_eq!(s.layers[2][0].density[3], 2.3);
        assert_eq!(s.types().count(), 3);
    }

    #[test]
    fn a_short_or_overlong_res_is_an_error_not_a_panic() {
        let d = sample();
        for n in 0..d.len() {
            assert!(Scattering::parse(&d[..n]).is_err(), "cut at {n}");
        }
        let mut longer = d.clone();
        longer.push(0);
        assert!(Scattering::parse(&longer).is_err());
        let mut many = d.clone();
        many[HEADER..HEADER + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Scattering::parse(&many).is_err());
        let mut flag = d;
        // The first type's triple flag, after its name, mesh set and 8 bytes.
        flag[HEADER + 8 + "a/grass_mesh".len() + 1 + 16] = 2;
        assert!(Scattering::parse(&flag).is_err());
    }

    fn number(v: Option<Value>) -> f32 {
        match v {
            Some(Value::Float32(x)) => x,
            Some(Value::Int(x)) => x as f32,
            Some(Value::UInt(x)) => x as f32,
            other => panic!("not a number: {other:?}"),
        }
    }

    /// Reads Palm City's scattering from the store `NFSH_STORE` points at and checks it
    /// against the `TerrainData` ebx: every type's settings are its
    /// `TerrainMeshScatteringType`'s with the same identifier, each layer's types are
    /// one `TerrainLayers` entry's, in the same order, and every mesh is a mesh asset
    /// with that `MeshSetResource`.
    #[test]
    fn the_scattering_is_the_terrain_ebxs_scatter_types() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let s = Scattering::read(&store).unwrap();
        let counts: Vec<usize> = s.layers.iter().map(Vec::len).collect();
        assert_eq!(
            counts,
            [
                0, 6, 5, 0, 1, 2, 2, 5, 0, 0, 1, 0, 2, 4, 3, 0, 4, 2, 7, 4, 4, 3, 1, 3, 5, 0, 0, 5,
                0, 0, 0
            ]
        );
        let ids: Vec<u32> = s.types().map(|t| t.identifier).collect();
        assert_eq!(ids, (0..69).collect::<Vec<_>>());
        // The road layers (8 and 9) scatter nothing; the forest floor (7) ferns.
        assert!(s.layers[7].iter().any(|t| t.mesh.contains("fern")));

        let mut entries = store.entries().unwrap();
        let data = entries
            .read(store.ebx(VISUAL_RESOURCE).unwrap().location)
            .unwrap();
        let p = Partition::parse(&data).unwrap();
        let mut by_id = HashMap::new();
        let mut ebx_layers = Vec::new();
        for i in &p.instances {
            let o = p.object(i);
            match p.type_name(o.type_index).as_str() {
                "TerrainMeshScatteringType" => {
                    let id = number(field(&p, o, "Identifier").unwrap()) as u32;
                    by_id.insert(id, o);
                }
                "TerrainData" => {
                    for layer in elements(&p, o, "TerrainLayers").unwrap() {
                        let Value::Class(crate::ebx::Ref::Internal(l)) = layer else {
                            panic!("a layer is {layer:?}");
                        };
                        let lo = p.object(&p.instances[l]);
                        let types = elements(&p, lo, "MeshScatteringTypes").unwrap();
                        ebx_layers.push(
                            types
                                .iter()
                                .map(|t| match t {
                                    Value::Class(crate::ebx::Ref::Internal(k)) => {
                                        let to = p.object(&p.instances[*k]);
                                        number(field(&p, to, "Identifier").unwrap()) as u32
                                    }
                                    other => panic!("a type is {other:?}"),
                                })
                                .collect::<Vec<_>>(),
                        );
                    }
                }
                _ => {}
            }
        }
        assert_eq!((by_id.len(), ebx_layers.len()), (69, 39));
        // Each non-empty layer is a TerrainLayers entry, in order, but not at its index.
        let mut from = 0;
        for layer in s.layers.iter().filter(|l| !l.is_empty()) {
            let want: Vec<u32> = layer.iter().map(|t| t.identifier).collect();
            let at = ebx_layers[from..].iter().position(|l| *l == want).unwrap();
            from += at + 1;
        }
        let get = |o, name: &str| number(field(&p, o, name).unwrap());
        let pair = |o, name: &str| match field(&p, o, name).unwrap() {
            Some(Value::Struct(v)) => [get(v, "x"), get(v, "y")],
            other => panic!("{name} is {other:?}"),
        };
        for t in s.types() {
            let o = by_id[&t.identifier];
            let Some(Value::Struct(d)) = field(&p, o, "Density").unwrap() else {
                panic!("no Density");
            };
            let density = ["Low", "Medium", "High", "Ultra"].map(|q| get(d, q));
            assert_eq!(t.density, density, "{}", t.mesh);
            assert_eq!(
                (t.min_scale, t.max_scale),
                (pair(o, "MinScale"), pair(o, "MaxScale"))
            );
            assert_eq!(
                [
                    t.scale_randomness,
                    t.wind_scale,
                    t.stiffness,
                    t.damping,
                    t.mass,
                    t.wind_wiggle,
                    t.dissolve_range_ratio
                ],
                [
                    "ScaleRandomness",
                    "WindScale",
                    "Stiffness",
                    "Damping",
                    "Mass",
                    "WindWiggle",
                    "DissolveRangeRatio"
                ]
                .map(|n| get(o, n)),
                "{}",
                t.mesh
            );
            assert!(t.distances.is_sorted(), "{}: {:?}", t.mesh, t.distances);
            let mesh = store.ebx(&t.mesh).unwrap_or_else(|| panic!("{}", t.mesh));
            let mesh_data = entries.read(mesh.location).unwrap();
            let mp = Partition::parse(&mesh_data).unwrap();
            assert_eq!(mp.resource("MeshSetResource").unwrap(), t.mesh_set);
        }
    }
}
