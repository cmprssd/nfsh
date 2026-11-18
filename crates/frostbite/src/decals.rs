//! Palm City's terrain decals: tyre tracks, dirt roads, rivers, crop fields, road
//! cracks and the like, as meshes laid on the ground, from the res the terrain
//! entity's `DecalsResource` names (see docs/FORMATS.md, "Terrain decals").
//! Little-endian, packed with no alignment.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ebx::name_hash;
use crate::mesh::half;
use crate::reader::Reader;
use crate::shaderdb::{self, ShaderDb};
use crate::store::Store;
use crate::{Error, Result, bail};

/// The res holding the decals (in `Win32/levels/mainlevel/mainlevel`); the
/// `TerrainEntityData` in `levels/mainlevel/default` names it by id as its
/// `DecalsResource`.
pub const DECALS: &str = "levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048_levels/mainlevel/mainlevel/decals";

/// The header before the first decal: an f32 (120), a u32 (0), a u32 (1) and a byte
/// (0), not worked out, then the decal count (u32).
const HEADER: usize = 13;
/// A decal's texture slots.
pub const SLOTS: usize = 5;
/// The slot of the decal's colour texture (`…_2D`), set on every decal.
pub const COLOUR: usize = 0;
/// The slot of its scatter mask (`…_ScatterMask`), when it has one.
pub const SCATTER_MASK: usize = 2;
/// The slot of its displacement texture (`…_Displacement`, `…_Disp`), when it has one.
pub const DISPLACEMENT: usize = 3;
/// A decal's bytes after its texture names.
const RECORD: usize = 4 * 17 + 3;
/// A vertex's bytes with and without a normal and a tangent.
const FRAMED: usize = 40;
const PLAIN: usize = 24;

/// Every terrain decal.
#[derive(Debug, Clone, PartialEq)]
pub struct Decals {
    pub decals: Vec<Decal>,
}

/// One decal: a mesh of triangles over the ground in one grid square, and its
/// textures. Its vertices have no height: the ground under them gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Decal {
    /// Shader asset names by slot ([`COLOUR`], [`SCATTER_MASK`], [`DISPLACEMENT`]);
    /// empty for none. Slots 1 and 4 are empty on every decal.
    pub textures: [String; SLOTS],
    /// Its vertices' bounds: least x, y, z, then greatest. The y range is the ground's
    /// under the vertices; on decals without normals it is empty (1e20 to −1e20).
    pub bounds: [f32; 6],
    /// The grid square holding it, 128 m (most), 256 or 512 m, as the same six floats
    /// with the bounds' y range.
    pub square: [f32; 6],
    /// 1/(n + 1) for an n of 100 on most decals (10 to 500 on others); likely a draw
    /// order or priority, not worked out.
    pub order: f32,
    /// The last two of its three flag bytes (always 1 in Palm City), not worked out.
    pub flags: [u8; 2],
    pub vertices: Vec<Vertex>,
    /// Triangles, three indices each into `vertices`.
    pub indices: Vec<u32>,
}

/// A texture a decal's shader binds: its slot (the `t` register its program samples
/// it from) and its ebx name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bound {
    pub slot: u32,
    pub name: String,
}

/// A decal vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// Its x and z; the ground's height there is its y.
    pub xz: [f32; 2],
    /// The ground's normal and the tangent (along decreasing u), unit length, on the
    /// decals that carry them (all but the parking lot plots).
    pub frame: Option<[[f32; 3]; 2]>,
    pub uv: [f32; 2],
    /// Four halves, all equal on most decals (0 to 1): likely its opacity, fading the
    /// tracks' ends.
    pub fade: [f32; 4],
}

impl Decals {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::at(data, HEADER)?;
        let count = r.u32_le()? as usize;
        // A decal takes at least its record and an empty name in each slot.
        if count > data.len() / (RECORD + SLOTS) {
            bail!("{count} decals can't fit");
        }
        let mut heads = Vec::with_capacity(count);
        for i in 0..count {
            heads.push(head(&mut r).map_err(|e| Error(format!("decal {i}: {e}")))?);
        }
        let vertex_bytes = r.u32_le()? as usize;
        let index_bytes = r.u32_le()? as usize;
        let vertices = r.bytes(vertex_bytes)?;
        let indices = r.bytes(index_bytes)?;
        // Eight bytes end it (zero in Palm City), not worked out.
        r.bytes(8)?;
        if r.pos() != data.len() {
            bail!("{} bytes after the indices", data.len() - r.pos());
        }
        let decals = heads
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                h.mesh(vertices, indices)
                    .map_err(|e| Error(format!("decal {i}: {e}")))
            })
            .collect::<Result<_>>()?;
        Ok(Self { decals })
    }

    /// Every shader the decals name, in name order, with the textures the shader
    /// database binds for it in slot order, each once. A shader the database lacks
    /// is left out, and so is a texture `names` cannot name.
    pub fn bindings(
        &self,
        db: &ShaderDb,
        names: &HashMap<u32, String>,
    ) -> BTreeMap<String, Vec<Bound>> {
        self.shaders()
            .into_iter()
            .filter(|s| {
                db.permutations_of(name_hash(&s.to_lowercase()))
                    .next()
                    .is_some()
            })
            .map(|s| {
                let bound: BTreeSet<Bound> = db
                    .textures(name_hash(&s.to_lowercase()))
                    .filter_map(|b| {
                        Some(Bound {
                            slot: b.slot,
                            name: names.get(&b.texture)?.clone(),
                        })
                    })
                    .collect();
                (s.to_string(), bound.into_iter().collect())
            })
            .collect()
    }

    /// [`Decals::bindings`] from Palm City's shader database, the textures named
    /// by the store's ebx.
    pub fn textures(&self, store: &Store) -> Result<BTreeMap<String, Vec<Bound>>> {
        let db = ShaderDb::read(store)?;
        let wanted: BTreeSet<u32> = self
            .shaders()
            .iter()
            .flat_map(|s| db.textures(name_hash(&s.to_lowercase())))
            .map(|b| b.texture)
            .collect();
        Ok(self.bindings(&db, &shaderdb::texture_names(store, &wanted)))
    }

    /// The shaders the decals name, each once.
    fn shaders(&self) -> BTreeSet<&str> {
        self.decals
            .iter()
            .flat_map(|d| &d.textures)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .collect()
    }

    /// Reads Palm City's decals from `store`.
    pub fn read(store: &Store) -> Result<Self> {
        let asset = store
            .res(DECALS)
            .ok_or_else(|| Error(format!("{DECALS}: not in the store")))?;
        let data = store.entries()?.read(asset.location)?;
        Self::parse(&data).map_err(|e| Error(format!("{DECALS}: {e}")))
    }
}

/// A decal's record before its mesh is read.
struct Head {
    textures: [String; SLOTS],
    /// Where its indices start (in indices) and its triangle count.
    first_index: usize,
    triangles: usize,
    bounds: [f32; 6],
    square: [f32; 6],
    order: f32,
    /// Where its vertices start and their length, in bytes.
    vertex_start: usize,
    vertex_bytes: usize,
    framed: bool,
    flags: [u8; 2],
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

fn halves<const N: usize>(r: &mut Reader) -> Result<[f32; N]> {
    let mut v = [0.0; N];
    for x in &mut v {
        *x = half(r.u16_le()?);
    }
    Ok(v)
}

fn head(r: &mut Reader) -> Result<Head> {
    let mut textures: [String; SLOTS] = Default::default();
    for t in &mut textures {
        *t = r.cstr()?.to_owned();
    }
    let first_index = r.u32_le()? as usize;
    let triangles = r.u32_le()? as usize;
    let bounds = f32s::<6>(r)?;
    let square = f32s::<6>(r)?;
    let order = f32_le(r)?;
    let vertex_start = r.u32_le()? as usize;
    let vertex_bytes = r.u32_le()? as usize;
    let framed = match r.u8()? {
        0 => false,
        1 => true,
        f => bail!("vertex flag {f}"),
    };
    let flags = r.array::<2>()?;
    Ok(Head {
        textures,
        first_index,
        triangles,
        bounds,
        square,
        order,
        vertex_start,
        vertex_bytes,
        framed,
        flags,
    })
}

impl Head {
    /// The decal, with its vertices and indices out of the res's two buffers.
    fn mesh(self, vertices: &[u8], indices: &[u8]) -> Result<Decal> {
        let stride = if self.framed { FRAMED } else { PLAIN };
        if !self.vertex_bytes.is_multiple_of(stride) {
            bail!(
                "{} vertex bytes, not a multiple of {stride}",
                self.vertex_bytes
            );
        }
        let mut r = Reader::at(vertices, self.vertex_start)?;
        let count = self.vertex_bytes / stride;
        let mut list = Vec::with_capacity(count);
        for _ in 0..count {
            let xz = f32s::<2>(&mut r)?;
            let frame = if self.framed {
                let normal = halves::<4>(&mut r)?;
                let tangent = halves::<4>(&mut r)?;
                Some([
                    [normal[0], normal[1], normal[2]],
                    [tangent[0], tangent[1], tangent[2]],
                ])
            } else {
                None
            };
            let uv = f32s::<2>(&mut r)?;
            let fade = halves::<4>(&mut r)?;
            list.push(Vertex {
                xz,
                frame,
                uv,
                fade,
            });
        }
        let start = self
            .first_index
            .checked_mul(4)
            .ok_or_else(|| Error("index offset overflows".into()))?;
        let length = self
            .triangles
            .checked_mul(12)
            .ok_or_else(|| Error("index count overflows".into()))?;
        let mut r = Reader::at(indices, start)?;
        let mut idx = Vec::with_capacity(self.triangles * 3);
        for _ in 0..length / 4 {
            let i = r.u32_le()?;
            if i as usize >= count {
                bail!("index {i} past its {count} vertices");
            }
            idx.push(i);
        }
        Ok(Decal {
            textures: self.textures,
            bounds: self.bounds,
            square: self.square,
            order: self.order,
            flags: self.flags,
            vertices: list,
            indices: idx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn put_f32s(out: &mut Vec<u8>, v: &[f32]) {
        v.iter()
            .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    }

    /// A half's bits, for the few values the sample uses.
    fn half_bits(x: f32) -> u16 {
        match x {
            0.0 => 0,
            0.5 => 0x3800,
            1.0 => 0x3C00,
            -1.0 => 0xBC00,
            _ => panic!("no bits for {x}"),
        }
    }

    fn put_halves(out: &mut Vec<u8>, v: &[f32]) {
        v.iter()
            .for_each(|&x| out.extend_from_slice(&half_bits(x).to_le_bytes()));
    }

    /// One decal's record.
    fn record(
        names: [&str; SLOTS],
        first: u32,
        triangles: u32,
        vertex: (u32, u32),
        framed: bool,
    ) -> Vec<u8> {
        let mut d = Vec::new();
        for n in names {
            d.extend_from_slice(n.as_bytes());
            d.push(0);
        }
        d.extend_from_slice(&first.to_le_bytes());
        d.extend_from_slice(&triangles.to_le_bytes());
        put_f32s(&mut d, &[10.0, 1.0, 20.0, 12.0, 2.0, 22.0]);
        put_f32s(&mut d, &[0.0, 1.0, 0.0, 128.0, 2.0, 128.0]);
        put_f32s(&mut d, &[1.0 / 101.0]);
        d.extend_from_slice(&vertex.0.to_le_bytes());
        d.extend_from_slice(&vertex.1.to_le_bytes());
        d.extend_from_slice(&[u8::from(framed), 1, 1]);
        d
    }

    /// Two decals: a framed one of one triangle, a plain one of two (a quad).
    fn sample() -> Vec<u8> {
        let mut d = Vec::new();
        put_f32s(&mut d, &[120.0]);
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.push(0);
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend(record(
            ["a_2D", "", "a_ScatterMask", "", ""],
            0,
            1,
            (0, 120),
            true,
        ));
        d.extend(record(["b_2D", "", "", "", ""], 3, 2, (120, 96), false));
        let mut v = Vec::new();
        for k in 0..3 {
            put_f32s(&mut v, &[10.0 + k as f32, 20.0]);
            put_halves(&mut v, &[0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0]);
            put_f32s(&mut v, &[k as f32, 0.5]);
            put_halves(&mut v, &[1.0, 1.0, 1.0, 1.0]);
        }
        for k in 0..4 {
            put_f32s(&mut v, &[k as f32, 5.0, 0.0, 1.0]);
            put_halves(&mut v, &[0.5, 0.5, 0.5, 0.5]);
        }
        d.extend_from_slice(&(v.len() as u32).to_le_bytes());
        let idx: Vec<u32> = vec![0, 1, 2, 0, 1, 2, 2, 1, 3];
        d.extend_from_slice(&(4 * idx.len() as u32).to_le_bytes());
        d.extend(v);
        idx.iter()
            .for_each(|i| d.extend_from_slice(&i.to_le_bytes()));
        d.extend_from_slice(&[0; 8]);
        d
    }

    #[test]
    fn reads_each_decals_textures_and_mesh() {
        let s = Decals::parse(&sample()).unwrap();
        assert_eq!(s.decals.len(), 2);
        let [a, b] = &s.decals[..] else { panic!() };
        assert_eq!(a.textures[COLOUR], "a_2D");
        assert_eq!(a.textures[SCATTER_MASK], "a_ScatterMask");
        assert_eq!(b.textures[DISPLACEMENT], "");
        assert_eq!(a.bounds, [10.0, 1.0, 20.0, 12.0, 2.0, 22.0]);
        assert_eq!(a.square[3], 128.0);
        assert_eq!(a.flags, [1, 1]);
        assert_eq!(a.vertices.len(), 3);
        assert_eq!(
            a.vertices[2],
            Vertex {
                xz: [12.0, 20.0],
                frame: Some([[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]]),
                uv: [2.0, 0.5],
                fade: [1.0; 4],
            }
        );
        assert_eq!(a.indices, [0, 1, 2]);
        assert_eq!(b.vertices.len(), 4);
        assert_eq!(b.vertices[3].frame, None);
        assert_eq!(b.vertices[3].xz, [3.0, 5.0]);
        assert_eq!(b.vertices[3].fade, [0.5; 4]);
        assert_eq!(b.indices, [0, 1, 2, 2, 1, 3]);
    }

    #[test]
    fn a_damaged_res_is_an_error_not_a_panic() {
        let good = sample();
        for cut in 0..good.len() {
            assert!(Decals::parse(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut long = good.clone();
        long.push(0);
        assert!(Decals::parse(&long).is_err());
        // An index past the decal's vertices.
        let mut bad = good.clone();
        let at = bad.len() - 8 - 4;
        bad[at..at + 4].copy_from_slice(&9u32.to_le_bytes());
        assert!(Decals::parse(&bad).is_err());
        // A huge decal count.
        let mut many = good;
        many[HEADER..HEADER + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Decals::parse(&many).is_err());
    }

    #[test]
    fn each_shader_binds_its_blocks_textures_in_slot_order_once() {
        use crate::shaderdb::{Binding, Permutation};
        let s = Decals::parse(&sample()).unwrap();
        let bind = |name: &str, slot| Binding {
            texture: name_hash(name),
            slot,
        };
        let permutation = |shader: &str, block| Permutation {
            shader: name_hash(shader),
            block,
            program: 0,
        };
        // "a_2D" in two permutations binding the same pair; "a_ScatterMask" binds a
        // texture no ebx names; "b_2D" is not in the database.
        let db = ShaderDb {
            blocks: vec![
                vec![bind("t_n", 2), bind("t_d", 1)],
                vec![bind("t_d", 1), bind("t_n", 2)],
                vec![bind("nameless", 1)],
            ],
            programs: vec![[0; 16]],
            permutations: vec![
                permutation("a_2d", 0),
                permutation("a_scattermask", 2),
                permutation("a_2d", 1),
            ],
        };
        let names: HashMap<u32, String> = ["T_D", "T_N"]
            .map(|n| (name_hash(&n.to_lowercase()), n.to_string()))
            .into();
        let bound = |slot, name: &str| Bound {
            slot,
            name: name.to_string(),
        };
        assert_eq!(
            s.bindings(&db, &names).into_iter().collect::<Vec<_>>(),
            [
                ("a_2D".to_string(), vec![bound(1, "T_D"), bound(2, "T_N")]),
                ("a_ScatterMask".to_string(), vec![]),
            ]
        );
    }

    /// Bilinear ground height at `xz` from the smallest tile with heights holding it.
    fn ground(
        tree: &crate::terrain::Tree,
        chunks: &mut crate::terrain::Chunks,
        cache: &mut std::collections::HashMap<usize, crate::terrain::Heightfield>,
        xz: [f32; 2],
    ) -> Option<f32> {
        use crate::terrain::{BORDER, Heightfield, QUADRANTS};
        let mut i = 0;
        let mut best = None;
        loop {
            let t = &tree.tiles[i];
            if t.has_heights() {
                best = Some(i);
            }
            let Some(children) = t.children else { break };
            let half = t.size / 2.0;
            let q = [
                u32::from(xz[0] >= t.min[0] + half),
                u32::from(xz[1] >= t.min[1] + half),
            ];
            i = children[QUADRANTS.iter().position(|&c| c == q)?] as usize;
        }
        let i = best?;
        let t = &tree.tiles[i];
        let h = match cache.entry(i) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let chunk = chunks.read(&t.chunk?.0).ok()?;
                e.insert(Heightfield::parse(&chunk).ok()?)
            }
        };
        let c = (xz[0] - t.min[0]) * 128.0 / t.size + BORDER as f32;
        let r = (xz[1] - t.min[1]) * 128.0 / t.size + BORDER as f32;
        let (c0, r0) = (c.floor() as usize, r.floor() as usize);
        let (fc, fr) = (c - c0 as f32, r - r0 as f32);
        Some(
            h.at(r0, c0)? * (1.0 - fc) * (1.0 - fr)
                + h.at(r0, c0 + 1)? * fc * (1.0 - fr)
                + h.at(r0 + 1, c0)? * (1.0 - fc) * fr
                + h.at(r0 + 1, c0 + 1)? * fc * fr,
        )
    }

    /// Every shader Palm City's decals name is in its shader database, with its
    /// textures in slots from 1 on, every one an ebx the store names.
    #[test]
    fn the_decals_shaders_bind_named_textures() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let s = Decals::read(&store).unwrap();
        let t = s.textures(&store).unwrap();
        assert_eq!(t.len(), s.shaders().len());
        assert_eq!(t.len(), 49);
        let bare: Vec<&str> = t
            .iter()
            .filter(|(_, b)| b.is_empty())
            .map(|(s, _)| s.as_str())
            .collect();
        assert_eq!(bare.len(), 1);
        assert!(bare[0].ends_with("TS_Decal_Lawnmower_TireTracks_ScatterMask_01_2D"));
        for (shader, bound) in &t {
            let slots: Vec<u32> = bound.iter().map(|b| b.slot).collect();
            assert_eq!(
                slots,
                (1..=bound.len() as u32).collect::<Vec<_>>(),
                "{shader}"
            );
        }
        let distinct: BTreeSet<&str> = t.values().flatten().map(|b| b.name.as_str()).collect();
        assert_eq!(distinct.len(), 65);
        let short = |shader: &str| -> Vec<String> {
            let (_, b) = t.iter().find(|(s, _)| s.ends_with(shader)).unwrap();
            b.iter()
                .map(|b| b.name.rsplit('/').next().unwrap().to_lowercase())
                .collect()
        };
        assert_eq!(
            short("/TS_Decal_TireTracks_01_2D"),
            [
                "t_tiretracks_01_nha",
                "t_terrain_desert_medium_gravel_02_n",
                "t_terrain_desert_medium_gravel_02_d",
                "t_tiretracks_01_a"
            ]
        );
        assert_eq!(
            short("/TS_Decal_TireTracks_01_Displacement"),
            ["t_tiretracks_01_nha"]
        );
        assert_eq!(short("/TS_Decal_Plot_ParkingLots"), ["parkinglots"]);
        assert_eq!(
            short("/TS_Road_Cracks_01_2D"),
            [
                "drv_cracks_decal_01_d",
                "drv_cracks_decal_01_n",
                "ut_naturalnoises_d"
            ]
        );
    }

    /// Reads Palm City's decals from the store `NFSH_STORE` points at and checks them:
    /// the terrain entity names this res, every vertex lies in its decal's bounds and
    /// square, and on a sample of decals the bounds' y range is the ground's under the
    /// vertices and their normals the ground's.
    #[test]
    fn the_decals_lie_on_the_ground() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let s = Decals::read(&store).unwrap();
        let vertices: usize = s.decals.iter().map(|d| d.vertices.len()).sum();
        let triangles: usize = s.decals.iter().map(|d| d.indices.len() / 3).sum();
        assert_eq!(
            (s.decals.len(), vertices, triangles),
            (2_678, 168_657, 153_347)
        );
        let count = |slot: usize| {
            s.decals
                .iter()
                .filter(|d| !d.textures[slot].is_empty())
                .count()
        };
        assert_eq!(
            (0..SLOTS).map(count).collect::<Vec<_>>(),
            [2_678, 0, 557, 1_650, 0]
        );
        let plain = s.decals.iter().filter(|d| d.vertices[0].frame.is_none());
        assert_eq!(plain.clone().count(), 84);
        assert!(plain.clone().all(|d| d.bounds[1] > d.bounds[4]));
        assert!(
            plain
                .clone()
                .all(|d| d.textures[COLOUR].contains("Decal_Plot_Parking"))
        );

        // The terrain entity's DecalsResource is this res.
        let mut entries = store.entries().unwrap();
        let data = entries
            .read(store.ebx(crate::terrain::GROUND_HEIGHTS).unwrap().location)
            .unwrap();
        let p = crate::ebx::Partition::parse(&data).unwrap();
        let entity = p
            .instances
            .iter()
            .map(|i| p.object(i))
            .find(|o| p.type_name(o.type_index) == "TerrainEntityData")
            .unwrap();
        let Ok(Some(crate::ebx::Value::ResourceRef(id))) =
            crate::level::field(&p, entity, "DecalsResource")
        else {
            panic!("no DecalsResource");
        };
        let named = store
            .superbundles
            .iter()
            .flat_map(|sb| &sb.bundles)
            .flat_map(|b| &b.bundle.res)
            .find(|r| r.id == id)
            .unwrap();
        assert!(named.asset.name.eq_ignore_ascii_case(DECALS));

        for (i, d) in s.decals.iter().enumerate() {
            let b = d.bounds;
            for v in &d.vertices {
                let [x, z] = v.xz;
                assert!(
                    x >= b[0] && x <= b[3] && z >= b[2] && z <= b[5],
                    "decal {i}"
                );
                // The square's edges are rounded: a vertex may sit a hair outside.
                let q = d.square;
                let e = 1e-3;
                assert!(
                    x >= q[0] - e && x <= q[3] + e && z >= q[2] - e && z <= q[5] + e,
                    "decal {i}"
                );
                if let Some([n, _]) = v.frame {
                    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    assert!((l - 1.0).abs() < 0.002, "decal {i}: normal {n:?}");
                }
            }
        }

        // Every 20th framed decal: its y range is the ground's at its vertices (to
        // 5 cm) and its normals the ground's (median within 3 degrees).
        let tree = crate::terrain::Tree::read(&store).unwrap();
        let mut chunks = crate::terrain::Chunks::new(&store).unwrap();
        let mut cache = std::collections::HashMap::new();
        let mut angles = Vec::new();
        let mut checked = 0;
        for d in s
            .decals
            .iter()
            .filter(|d| d.vertices[0].frame.is_some())
            .step_by(20)
        {
            // Decals over a hole in the ground are left out.
            let Some(ys) = d
                .vertices
                .iter()
                .map(|v| ground(&tree, &mut chunks, &mut cache, v.xz))
                .collect::<Option<Vec<f32>>>()
            else {
                continue;
            };
            let lo = ys.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert!((lo - d.bounds[1]).abs() < 0.05, "{lo} vs {:?}", d.bounds);
            assert!((hi - d.bounds[4]).abs() < 0.05, "{hi} vs {:?}", d.bounds);
            for v in &d.vertices {
                let [x, z] = v.xz;
                let e = 0.25;
                let mut at = |p: [f32; 2]| ground(&tree, &mut chunks, &mut cache, p);
                let (Some(xp), Some(xm), Some(zp), Some(zm)) = (
                    at([x + e, z]),
                    at([x - e, z]),
                    at([x, z + e]),
                    at([x, z - e]),
                ) else {
                    continue;
                };
                let dx = (xp - xm) / (2.0 * e);
                let dz = (zp - zm) / (2.0 * e);
                let l = (dx * dx + 1.0 + dz * dz).sqrt();
                let n = v.frame.unwrap()[0];
                let cos = (-dx * n[0] + n[1] - dz * n[2]) / l;
                angles.push(cos.clamp(-1.0, 1.0).acos().to_degrees());
            }
            checked += 1;
        }
        assert!(checked > 100);
        angles.sort_by(f32::total_cmp);
        let median = angles[angles.len() / 2];
        assert!(median < 3.0, "median normal angle {median}");
    }
}
