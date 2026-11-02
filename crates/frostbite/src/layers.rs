//! The terrain's layers: the shaders drawing each combination of them, and each
//! layer's own textures (see docs/FORMATS.md, "Terrain layers").
//! Little-endian.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::dxbc::{self, Scale};
use crate::ebx::name_hash;
use crate::reader::{Reader, cstr_at};
use crate::shaderdb::{self, ShaderDb};
use crate::store::Store;
use crate::{Error, Result, bail};

/// The res listing Palm City's layer combinations (in the terrain's root bundle).
pub const COMBINATIONS: &str =
    "levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048.layercombinations_win32";

/// The entries' count, and where the first starts.
const COUNT_AT: usize = 8;
const ENTRIES: usize = 0x20;
/// An entry: seven arrays, each a count (u32) and an offset (u64), then a u32.
const ENTRY: usize = 0x58;
const ARRAY: usize = 12;
/// The res type of a shader program's bytecode.
const BYTECODE: u32 = 0x0E2B_02F7;
/// The array of an entry listing its `__2d` shaders: those drawing the ground flat,
/// from the layers' textures.
const FLAT: usize = 4;
/// A shader's record: its name's offset (u64), then its layers' count (u32) and
/// offset (u64, to u16s).
const SHADER: usize = 0x28;

/// A shader drawing one combination of layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shader {
    pub name: String,
    /// Its layers, indices into the `TerrainData`'s `TerrainLayers`.
    pub layers: Vec<u16>,
}

/// Reads every entry's `__2d` shaders, each once, in the order first listed.
pub fn flat_shaders(data: &[u8]) -> Result<Vec<Shader>> {
    let count = Reader::at(data, COUNT_AT)?.u32_le()? as usize;
    let array = |r: &mut Reader| -> Result<(usize, usize)> {
        Ok((r.u32_le()? as usize, r.u64_le()? as usize))
    };
    let mut shaders: Vec<Shader> = Vec::new();
    for e in 0..count {
        let mut r = Reader::at(data, ENTRIES + e * ENTRY + FLAT * ARRAY)?;
        let (n, at) = array(&mut r)?;
        for s in 0..n {
            let record = at
                .checked_add(s * SHADER)
                .ok_or_else(|| Error(format!("entry {e}: shader {s} past the end")))?;
            let mut r = Reader::at(data, record)?;
            let name = cstr_at(data, r.u64_le()? as usize)?;
            let (layers, at) = array(&mut r)?;
            let mut r = Reader::at(data, at)?;
            let layers = (0..layers)
                .map(|_| r.u16_le())
                .collect::<Result<Vec<_>>>()?;
            if !shaders.iter().any(|s| s.name == name) {
                shaders.push(Shader {
                    name: name.to_owned(),
                    layers,
                });
            }
        }
    }
    Ok(shaders)
}

/// Each layer's own textures (name hashes): those every shader with the layer binds,
/// less those that in every such shader another of its layers accounts for (binds in
/// all of its own shaders). A texture two layers share is bound once, so it counts
/// for both. An error when a shader is not in `db`.
pub fn own_textures(shaders: &[Shader], db: &ShaderDb) -> Result<BTreeMap<u16, BTreeSet<u32>>> {
    let mut bound = Vec::with_capacity(shaders.len());
    for s in shaders {
        let textures: BTreeSet<u32> = db
            .textures(name_hash(&s.name.to_lowercase()))
            .map(|b| b.texture)
            .collect();
        if textures.is_empty() {
            bail!("{}: no permutation binds a texture", s.name);
        }
        bound.push(textures);
    }
    // What every shader with each layer binds.
    let mut common: BTreeMap<u16, BTreeSet<u32>> = BTreeMap::new();
    for (s, textures) in shaders.iter().zip(&bound) {
        for &layer in &s.layers {
            common
                .entry(layer)
                .and_modify(|c| c.retain(|t| textures.contains(t)))
                .or_insert_with(|| textures.clone());
        }
    }
    let with = |layer: u16| shaders.iter().filter(move |s| s.layers.contains(&layer));
    Ok(common
        .iter()
        .map(|(&layer, textures)| {
            let own = textures
                .iter()
                .filter(|t| {
                    !with(layer).all(|s| {
                        s.layers
                            .iter()
                            .any(|&o| o != layer && common.get(&o).is_some_and(|c| c.contains(t)))
                    })
                })
                .copied()
                .collect();
            (layer, own)
        })
        .collect())
}

/// The scales a layer's texture is sampled at in each of the layer's shaders.
///
/// A layer's primary scale is the finest of those every one of them samples it at
/// straight from the world position, or failing that the finest turned one; none when
/// no scale is common to all (a texture a shader binds but only another layer reads).
pub fn primary_scale<'a>(mut shaders: impl Iterator<Item = &'a [Scale]>) -> Option<f32> {
    let first = shaders.next()?;
    let mut common = first.to_vec();
    for s in shaders {
        common.retain(|c| s.contains(c));
    }
    let finest = |rotated: bool| {
        common
            .iter()
            .filter_map(|c| match (c, rotated) {
                (Scale::Direct(s), false) | (Scale::Rotated(s), true) => Some(*s),
                _ => None,
            })
            .reduce(f32::max)
    };
    finest(false).or_else(|| finest(true))
}

/// One of a layer's own textures and the scale it is drawn at.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    /// Its ebx name.
    pub name: String,
    /// Its primary scale ([`primary_scale`]): world x and z times this are its
    /// coordinates (so it repeats every 1 / scale metres); none when it has none.
    pub scale: Option<f32>,
}

/// Each layer's own textures ([`own_textures`]) the store names, in name order, each
/// with its primary scale.
pub fn textures(store: &Store) -> Result<BTreeMap<u16, Vec<Texture>>> {
    let asset = store
        .res(COMBINATIONS)
        .ok_or_else(|| Error(format!("{COMBINATIONS}: not in the store")))?;
    let data = store.entries()?.read(asset.location)?;
    let shaders = flat_shaders(&data).map_err(|e| Error(format!("{COMBINATIONS}: {e}")))?;
    let db = ShaderDb::read(store)?;
    let own = own_textures(&shaders, &db)?;
    let sampled = sampled(store, &shaders, &db)?;
    let wanted: BTreeSet<u32> = own.values().flatten().copied().collect();
    let mut names: HashMap<u32, &str> = HashMap::new();
    for e in store
        .superbundles
        .iter()
        .flat_map(|sb| &sb.bundles)
        .flat_map(|b| &b.bundle.ebx)
    {
        let hash = name_hash(&e.name.to_lowercase());
        if wanted.contains(&hash) {
            names.entry(hash).or_insert(&e.name);
        }
    }
    Ok(own
        .into_iter()
        .map(|(layer, textures)| {
            let mut d: Vec<Texture> = textures
                .iter()
                .filter_map(|t| Some((t, names.get(t)?)))
                .map(|(t, n)| Texture {
                    name: n.to_string(),
                    scale: primary_scale(
                        shaders
                            .iter()
                            .zip(&sampled)
                            .filter(|(s, _)| s.layers.contains(&layer))
                            .map(|(_, by)| by.get(t).map_or(&[][..], Vec::as_slice)),
                    ),
                })
                .collect();
            d.sort_by(|a, b| a.name.cmp(&b.name));
            (layer, d)
        })
        .collect())
}

/// For each of `shaders`, the scales its programs sample each of its textures at (by
/// name hash).
fn sampled(
    store: &Store,
    shaders: &[Shader],
    db: &ShaderDb,
) -> Result<Vec<HashMap<u32, Vec<Scale>>>> {
    let programs: HashMap<String, crate::Location> = store
        .superbundles
        .iter()
        .flat_map(|sb| &sb.bundles)
        .flat_map(|b| &b.bundle.res)
        .filter(|r| r.res_type == BYTECODE)
        .map(|r| (r.asset.name.to_lowercase(), r.asset.location))
        .collect();
    let mut entries = store.entries()?;
    let mut code: HashMap<u32, Vec<dxbc::Instruction>> = HashMap::new();
    let mut out = Vec::with_capacity(shaders.len());
    for s in shaders {
        let mut by: HashMap<u32, Vec<Scale>> = HashMap::new();
        for p in db.permutations_of(name_hash(&s.name.to_lowercase())) {
            if let Entry::Vacant(e) = code.entry(p.program) {
                let name = shaderdb::program_name(&db.programs[p.program as usize]);
                let at = programs
                    .get(&name)
                    .ok_or_else(|| Error(format!("{}: {name} not in the store", s.name)))?;
                let read =
                    dxbc::parse(&entries.read(*at)?).map_err(|e| Error(format!("{name}: {e}")))?;
                e.insert(read);
            }
            for b in &db.blocks[p.block as usize] {
                by.entry(b.texture)
                    .or_default()
                    .extend(dxbc::sample_scales(&code[&p.program], b.slot));
            }
        }
        out.push(by);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderdb::tests::sample;

    #[test]
    fn the_primary_scale_is_the_finest_direct_one_all_shaders_share() {
        use Scale::{Direct, Rotated};
        let primary = |s: &[&[Scale]]| primary_scale(s.iter().copied());
        assert_eq!(primary(&[&[Direct(0.02), Direct(0.2)]]), Some(0.2));
        assert_eq!(
            primary(&[&[Direct(0.2), Rotated(0.3)], &[Direct(0.2), Direct(0.25)]]),
            Some(0.2),
            "0.25 is only in one"
        );
        assert_eq!(primary(&[&[Rotated(0.1), Rotated(0.16)]]), Some(0.16));
        assert_eq!(primary(&[&[Rotated(0.1), Direct(0.05)]]), Some(0.05));
        assert_eq!(primary(&[&[Direct(0.1)], &[]]), None, "bound, not sampled");
        assert_eq!(primary(&[]), None);
    }
    use std::path::Path;

    /// A combinations res with one entry per list of `__2d` shaders (name, layers).
    fn combinations(entries: &[&[(&str, &[u16])]]) -> Vec<u8> {
        let shaders: usize = entries.iter().map(|e| e.len()).sum();
        let records = ENTRIES + entries.len() * ENTRY;
        let mut tail = records + shaders * SHADER;
        let mut d = vec![0; tail];
        d[COUNT_AT..COUNT_AT + 4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        let mut extra = Vec::new();
        let mut record = records;
        for (e, list) in entries.iter().enumerate() {
            let at = ENTRIES + e * ENTRY + FLAT * ARRAY;
            d[at..at + 4].copy_from_slice(&(list.len() as u32).to_le_bytes());
            d[at + 4..at + 12].copy_from_slice(&(record as u64).to_le_bytes());
            for (name, layers) in *list {
                d[record..record + 8].copy_from_slice(&(tail as u64).to_le_bytes());
                extra.extend_from_slice(name.as_bytes());
                extra.push(0);
                tail += name.len() + 1;
                d[record + 8..record + 12].copy_from_slice(&(layers.len() as u32).to_le_bytes());
                d[record + 12..record + 20].copy_from_slice(&(tail as u64).to_le_bytes());
                for l in *layers {
                    extra.extend_from_slice(&l.to_le_bytes());
                }
                tail += 2 * layers.len();
                record += SHADER;
            }
        }
        d.extend_from_slice(&extra);
        d
    }

    #[test]
    fn reads_each_entrys_flat_shaders_once() {
        let d = combinations(&[
            &[("a__2d", &[1, 2])],
            &[],
            &[("b__2d", &[3]), ("a__2d", &[1, 2])],
        ]);
        let shaders = flat_shaders(&d).unwrap();
        let read: Vec<_> = shaders
            .iter()
            .map(|s| (s.name.as_str(), s.layers.clone()))
            .collect();
        assert_eq!(read, [("a__2d", vec![1, 2]), ("b__2d", vec![3])]);
        assert!(flat_shaders(&d[..d.len() - 1]).is_err());
    }

    #[test]
    fn a_layer_owns_what_no_other_layer_of_its_shaders_accounts_for() {
        let shader = |name: &str, layers: &[u16]| Shader {
            name: name.into(),
            layers: layers.to_vec(),
        };
        // Layer 1 binds 10; layer 4 shares 10 and adds 40, and appears without 1;
        // layer 6 binds 60 and only ever appears beside 1 or 4, so 10 is theirs.
        let shaders = [
            shader("s1", &[1]),
            shader("s14", &[1, 4]),
            shader("s4", &[4]),
            shader("s16", &[1, 6]),
            shader("s46", &[4, 6]),
        ];
        let h = |n: &str| name_hash(n);
        let db = sample(
            &[
                &[(10, 0)],
                &[(10, 0), (40, 1)],
                &[(10, 0), (60, 1)],
                &[(10, 0), (40, 1), (60, 2)],
            ],
            &[
                (h("s1"), 0),
                (h("s14"), 1),
                (h("s4"), 1),
                (h("s16"), 2),
                (h("s46"), 3),
            ],
        );
        let db = ShaderDb::parse(&db).unwrap();
        let own = own_textures(&shaders, &db).unwrap();
        let set = |t: &[u32]| t.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(own[&1], set(&[10]));
        assert_eq!(own[&4], set(&[10, 40]));
        assert_eq!(own[&6], set(&[60]));
        let missing = [shader("s9", &[9])];
        assert!(own_textures(&missing, &db).is_err());
    }

    /// Reads every layer's own textures from the store `NFSH_STORE` points at.
    #[test]
    fn each_layer_has_its_own_diffuse_and_normal_maps() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let diffuse = textures(&store).unwrap();
        let short = |layer: u16| -> Vec<&str> {
            diffuse[&layer]
                .iter()
                .map(|d| d.name.rsplit('/').next().unwrap())
                .filter(|n| n.ends_with("_d"))
                .collect()
        };
        let scale = |layer: u16, name: &str| {
            diffuse[&layer]
                .iter()
                .find(|d| d.name.ends_with(name))
                .unwrap()
                .scale
        };
        assert_eq!(diffuse.len(), 23);
        assert_eq!(short(1), ["t_mega_grass_01_d"]);
        assert_eq!(short(2), ["t_field_base_03_d"]);
        assert_eq!(short(4), ["t_mega_grass_01_d"]);
        assert_eq!(
            short(6),
            ["t_burnforestground_01_d", "t_burnforestground_02_d"]
        );
        assert_eq!(short(8), ["drv_set3_clean_d", "drv_set4_damage_d"]);
        assert_eq!(short(9), ["drv_set1_full_damage_d", "drv_set4_damage_d"]);
        assert_eq!(short(25), ["t_gravel_test_01_d"]);
        // 11 binds only a shared grass and an overlay; 30 only a shared detail normal.
        assert_eq!(short(11), ["t_miami_grass_01_d"]);
        assert!(short(30).is_empty());
        // The scales: one texture two layers share can be drawn at two.
        assert_eq!(scale(1, "t_mega_grass_01_d"), Some(0.2));
        assert_eq!(scale(4, "t_mega_grass_01_d"), Some(0.25));
        assert_eq!(scale(2, "t_field_base_03_d"), Some(0.1));
        assert_eq!(scale(8, "drv_set4_damage_d"), Some(0.1));
        assert_eq!(scale(9, "drv_set4_damage_d"), Some(0.15));
        // Only turned: the finest turned scale.
        let burnt = scale(6, "t_burnforestground_01_d").unwrap();
        assert!((burnt - 0.1).abs() < 1e-6, "{burnt}");
        // Bound for layers 5 and 29, never sampled for 28; the sand waves' coordinates
        // are not a plain scale of the world position.
        assert_eq!(scale(28, "t_terrain_desert_fine_gravel_02_d"), None);
        assert_eq!(scale(3, "t_terrain_sand_wave_01_d"), None);
        assert_eq!(scale(3, "t_terrain_sand_footprints_01_d"), Some(0.2));
        // Every scaled diffuse map's normal map is its name with `_n`, sampled at the
        // same scale.
        assert_eq!(scale(1, "t_mega_grass_01_n"), Some(0.2));
        let mut paired = 0;
        for (layer, maps) in &diffuse {
            for d in maps
                .iter()
                .filter(|d| d.name.ends_with("_d") && d.scale.is_some())
            {
                let normal = format!("{}_n", d.name.strip_suffix("_d").unwrap());
                match maps.iter().find(|n| n.name == normal) {
                    Some(n) => {
                        let (a, b) = (d.scale.unwrap(), n.scale.unwrap());
                        assert!(
                            (a - b).abs() < 1e-6,
                            "layer {layer}: {normal} at {b}, not {a}"
                        );
                        paired += 1;
                    }
                    None => panic!("layer {layer}: no {normal}"),
                }
            }
        }
        assert_eq!(paired, 31);
    }
}
