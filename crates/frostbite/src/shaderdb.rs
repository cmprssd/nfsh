//! A level's shader database: its parameter blocks (the textures each shader binds
//! and its named constants) and its shader permutations (see docs/FORMATS.md,
//! "Terrain layers"), and what the textures a program reads but its block does not
//! bind hold.
//! Little-endian.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::dxbc::{self, Instruction};
use crate::ebx::name_hash;
use crate::reader::Reader;
use crate::store::Store;
use crate::{Error, Result, bail};

/// Palm City's shader database, a res.
pub const MAINLEVEL: &str = "levels/mainlevel/mainlevel/shaderdb";

/// The parameter blocks' count, and where the first starts.
const BLOCK_COUNT_AT: usize = 0x18;
const BLOCKS: usize = 0x1C;
/// A block's header: its size, then six offsets from its start; its textures lie
/// between the second and third.
const BLOCK_HEADER: usize = 56;
const TEXTURE: usize = 16;
/// A constant: its name (32 bytes, NUL-padded), its name hash, a slot word, a count
/// and a default `Vec4`; they lie between the block's third and fourth offsets.
const CONSTANT: usize = 60;
const CONSTANT_NAME: usize = 32;
/// Where a constant's default `Vec4` starts: after its name, hash, slot word and count.
const CONSTANT_DEFAULT: usize = CONSTANT_NAME + 12;
/// Where Palm City's file holds the programs and their count, then the permutations
/// and theirs; the sections between the blocks and them are not worked out, so these
/// are not found from the file.
const PROGRAM_COUNT_AT: usize = 0x39_8338;
/// A program's record: 16 bytes, its GUID, then 16 bytes.
const PROGRAM: usize = 48;
const PROGRAM_GUID: usize = 16;
const PERMUTATION_COUNT_AT: usize = 0x3D_0808;
const PERMUTATIONS: usize = 0x3D_0810;
/// A permutation's record, and where in it its parameter block and its program
/// (bytecode) are; after all of them, a second record of each, starting with its
/// shader's name hash.
const PERMUTATION: usize = 176;
const BLOCK_AT: usize = 68;
const PROGRAM_AT: usize = 28;
const PERMUTATION_NAME: usize = 32;

/// A texture a parameter block binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// The texture's name hash ([`crate::ebx::name_hash`] of its lowercase ebx name).
    pub texture: u32,
    pub slot: u32,
}

/// A constant a parameter block names.
#[derive(Debug, Clone, PartialEq)]
pub struct Constant {
    pub name: String,
    /// Its register in the program's constant buffer 1: its place in the block's
    /// list.
    pub register: u32,
    /// Its value where a material sets none.
    pub default: [f32; 4],
}

/// What a texture holds that the engine supplies to a terrain or decal program (one
/// its parameter block does not bind), told by the constant its reads' coordinates
/// are mapped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Supplied {
    /// The terrain's heightfield (`worldToHeightfieldUvTransform`).
    Heightfield,
    /// The terrain's global colour (`worldToGlobalColorUvTransform`).
    GlobalColor,
    /// The layers' masks (`worldToMask<n>UvTransform`).
    Mask,
    /// None of these.
    Unknown,
}

/// The slots program `code` reads that its block, binding `bound` and naming
/// `constants`, does not bind, with what each holds.
pub fn supplied(
    code: &[Instruction],
    bound: &[Binding],
    constants: &[Constant],
) -> BTreeMap<u32, Supplied> {
    dxbc::read_slots(code)
        .into_iter()
        .filter(|s| bound.iter().all(|b| b.slot != *s))
        .map(|slot| {
            let names: Vec<&str> = dxbc::read_constants(code, slot)
                .into_iter()
                .filter(|&(buffer, _)| buffer == 1)
                .filter_map(|(_, r)| constants.iter().find(|c| c.register == r))
                .map(|c| c.name.as_str())
                .collect();
            let has = |n: &str| names.contains(&n);
            let kind = if has("worldToHeightfieldUvTransform") {
                Supplied::Heightfield
            } else if has("worldToGlobalColorUvTransform") {
                Supplied::GlobalColor
            } else if names
                .iter()
                .any(|n| n.starts_with("worldToMask") && n.ends_with("UvTransform"))
            {
                Supplied::Mask
            } else {
                Supplied::Unknown
            };
            (slot, kind)
        })
        .collect()
}

/// One compiled variant of a shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permutation {
    /// Its shader's name hash ([`crate::ebx::name_hash`] of its lowercase name).
    pub shader: u32,
    /// Its parameter block, an index into [`ShaderDb::blocks`].
    pub block: u32,
    /// Its program, an index into [`ShaderDb::programs`].
    pub program: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShaderDb {
    /// Each parameter block's textures, in the order it lists them.
    pub blocks: Vec<Vec<Binding>>,
    /// Each parameter block's constants, in its order.
    pub constants: Vec<Vec<Constant>>,
    /// Each program's GUID, as stored: its bytecode is the res [`program_name`] names.
    pub programs: Vec<[u8; 16]>,
    pub permutations: Vec<Permutation>,
}

impl ShaderDb {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let count = Reader::at(data, BLOCK_COUNT_AT)?.u32_le()? as usize;
        let mut blocks = Vec::with_capacity(count.min(data.len() / BLOCK_HEADER));
        let mut constants = Vec::with_capacity(blocks.capacity());
        let mut at = BLOCKS;
        for b in 0..count {
            let block = read_block(data, at).map_err(|e| Error(format!("block {b}: {e}")))?;
            blocks.push(block.1);
            constants.push(block.2);
            at += block.0;
        }
        if at > PROGRAM_COUNT_AT {
            bail!("the blocks end at {at:#x}, past the programs");
        }
        let mut r = Reader::at(data, PROGRAM_COUNT_AT)?;
        let count = r.u32_le()? as usize;
        if count > (PERMUTATION_COUNT_AT - r.pos()) / PROGRAM {
            bail!("{count} programs run past the permutations");
        }
        let programs = (0..count)
            .map(|_| {
                Ok(r.bytes(PROGRAM)?[PROGRAM_GUID..][..16]
                    .try_into()
                    .expect("16"))
            })
            .collect::<Result<Vec<[u8; 16]>>>()?;
        let count = Reader::at(data, PERMUTATION_COUNT_AT)?.u32_le()? as usize;
        let names = count
            .checked_mul(PERMUTATION)
            .and_then(|n| n.checked_add(PERMUTATIONS))
            .filter(|&end| end <= data.len())
            .ok_or_else(|| Error(format!("{count} permutations run past the end")))?;
        let mut first = Reader::at(data, PERMUTATIONS)?;
        let mut second = Reader::at(data, names)?;
        let mut permutations = Vec::with_capacity(count);
        for p in 0..count {
            let record = first.bytes(PERMUTATION)?;
            let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().expect("4"));
            let shader = Reader::new(second.bytes(PERMUTATION_NAME)?).u32_le()?;
            let block = word(BLOCK_AT);
            if block as usize >= blocks.len() {
                bail!("permutation {p}: block {block} of {}", blocks.len());
            }
            let program = word(PROGRAM_AT);
            if program as usize >= programs.len() {
                bail!("permutation {p}: program {program} of {}", programs.len());
            }
            permutations.push(Permutation {
                shader,
                block,
                program,
            });
        }
        Ok(Self {
            blocks,
            constants,
            programs,
            permutations,
        })
    }

    /// Reads Palm City's shader database from `store`.
    pub fn read(store: &Store) -> Result<Self> {
        let asset = store
            .res(MAINLEVEL)
            .ok_or_else(|| Error(format!("{MAINLEVEL}: not in the store")))?;
        let data = store.entries()?.read(asset.location)?;
        Self::parse(&data).map_err(|e| Error(format!("{MAINLEVEL}: {e}")))
    }

    /// The permutations of the shader whose name hash is `shader`.
    pub fn permutations_of(&self, shader: u32) -> impl Iterator<Item = &Permutation> {
        self.permutations.iter().filter(move |p| p.shader == shader)
    }

    /// The textures the shader whose name hash is `shader` binds, in every
    /// permutation of it; empty when it has none.
    pub fn textures(&self, shader: u32) -> impl Iterator<Item = &Binding> {
        self.permutations_of(shader)
            .flat_map(|p| &self.blocks[p.block as usize])
    }
}

/// The ebx names of the textures whose name hashes are in `wanted`, as the store
/// spells them; a hash no ebx has is left out.
pub fn texture_names(store: &Store, wanted: &BTreeSet<u32>) -> HashMap<u32, String> {
    let mut names = HashMap::new();
    for e in store
        .superbundles
        .iter()
        .flat_map(|sb| &sb.bundles)
        .flat_map(|b| &b.bundle.ebx)
    {
        let hash = name_hash(&e.name.to_lowercase());
        if wanted.contains(&hash) {
            names.entry(hash).or_insert_with(|| e.name.clone());
        }
    }
    names
}

/// The res holding the bytecode of the program whose GUID is `guid`:
/// `shaders/bytecode/` and the GUID in its usual text form (its first three fields
/// little-endian).
pub fn program_name(guid: &[u8; 16]) -> String {
    let g = crate::store::guid_order(*guid);
    let hex =
        |r: std::ops::Range<usize>| -> String { g[r].iter().map(|b| format!("{b:02x}")).collect() };
    format!(
        "shaders/bytecode/{}-{}-{}-{}-{}",
        hex(0..4),
        hex(4..6),
        hex(6..8),
        hex(8..10),
        hex(10..16)
    )
}

/// Reads the parameter block at `at`: its size, its textures and its constants.
fn read_block(data: &[u8], at: usize) -> Result<(usize, Vec<Binding>, Vec<Constant>)> {
    let mut r = Reader::at(data, at)?;
    let size = r.u64_le()?;
    let mut offsets = [0; 6];
    for o in &mut offsets {
        *o = r.u64_le()?;
    }
    let (from, to, end) = (offsets[1], offsets[2], offsets[3]);
    if size < BLOCK_HEADER as u64 || !(from <= to && to <= end && end <= size) {
        bail!("size {size}, textures from {from} to {to}, constants to {end}");
    }
    let (from, to) = (from as usize, to as usize);
    let mut r = Reader::at(data, at + from)?;
    let mut textures = Vec::with_capacity((to - from) / TEXTURE);
    for _ in 0..(to - from) / TEXTURE {
        let texture = r.u32_le()?;
        let slot = r.u32_le()?;
        r.bytes(8)?;
        textures.push(Binding { texture, slot });
    }
    let end = end as usize;
    let mut r = Reader::at(data, at + to)?;
    let mut constants = Vec::with_capacity((end - to) / CONSTANT);
    for register in 0..((end - to) / CONSTANT) as u32 {
        let c = r.bytes(CONSTANT)?;
        let name = &c[..CONSTANT_NAME];
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(CONSTANT_NAME)];
        let mut default = [0.0; 4];
        for (k, v) in default.iter_mut().enumerate() {
            let at = CONSTANT_DEFAULT + 4 * k;
            *v = f32::from_le_bytes([c[at], c[at + 1], c[at + 2], c[at + 3]]);
        }
        constants.push(Constant {
            name: String::from_utf8_lossy(name).into_owned(),
            register,
            default,
        });
    }
    Reader::at(data, at)?.bytes(size as usize)?;
    Ok((size as usize, textures, constants))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
        d[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A block binding `textures` (name hash, slot), with 8 bytes after.
    fn block(textures: &[(u32, u32)]) -> Vec<u8> {
        block_with(textures, &[])
    }

    /// A block binding `textures` (name hash, slot) and naming `constants`, with 8
    /// bytes after.
    fn block_with(textures: &[(u32, u32)], constants: &[&str]) -> Vec<u8> {
        let to = (BLOCK_HEADER + TEXTURE * textures.len()) as u64;
        let end = to + (CONSTANT * constants.len()) as u64;
        let mut d = Vec::new();
        for v in [end + 8, 0, BLOCK_HEADER as u64, to, end, end + 8, end + 8] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        for &(name, slot) in textures {
            d.extend_from_slice(&name.to_le_bytes());
            d.extend_from_slice(&slot.to_le_bytes());
            d.extend_from_slice(&[0; 8]);
        }
        for (i, name) in constants.iter().enumerate() {
            let mut c = [0; CONSTANT];
            c[..name.len()].copy_from_slice(name.as_bytes());
            put(&mut c, CONSTANT_NAME, &name_hash(name).to_le_bytes());
            put(
                &mut c,
                CONSTANT_NAME + 4,
                &(0x1_0001 + i as u32).to_le_bytes(),
            );
            put(&mut c, CONSTANT_NAME + 8, &4u32.to_le_bytes());
            put(&mut c, CONSTANT_DEFAULT, &(i as f32 + 0.5).to_le_bytes());
            d.extend_from_slice(&c);
        }
        d.extend_from_slice(&[0xCC; 8]);
        d
    }

    /// A database of `blocks` and `permutations` (shader hash, block).
    pub(crate) fn sample(blocks: &[&[(u32, u32)]], permutations: &[(u32, u32)]) -> Vec<u8> {
        let mut d = vec![0; BLOCKS];
        put(&mut d, BLOCK_COUNT_AT, &(blocks.len() as u32).to_le_bytes());
        for b in blocks {
            d.extend_from_slice(&block(b));
        }
        d.resize(PROGRAM_COUNT_AT, 0);
        let programs = 100 + permutations.len();
        d.extend_from_slice(&(programs as u32).to_le_bytes());
        for i in 0..programs {
            let mut record = [0; PROGRAM];
            record[PROGRAM_GUID] = i as u8;
            record[PROGRAM_GUID + 15] = 0xEE;
            d.extend_from_slice(&record);
        }
        d.resize(PERMUTATIONS, 0);
        put(
            &mut d,
            PERMUTATION_COUNT_AT,
            &(permutations.len() as u32).to_le_bytes(),
        );
        for (i, &(_, block)) in permutations.iter().enumerate() {
            let mut record = [0; PERMUTATION];
            put(&mut record, BLOCK_AT, &block.to_le_bytes());
            put(&mut record, PROGRAM_AT, &(100 + i as u32).to_le_bytes());
            d.extend_from_slice(&record);
        }
        for &(shader, _) in permutations {
            let mut record = [0; PERMUTATION_NAME];
            put(&mut record, 0, &shader.to_le_bytes());
            d.extend_from_slice(&record);
        }
        d
    }

    #[test]
    fn reads_blocks_and_permutations() {
        let d = sample(
            &[&[(7, 3), (8, 4)], &[], &[(9, 3)]],
            &[(0xA, 2), (0xB, 0), (0xA, 0)],
        );
        let db = ShaderDb::parse(&d).unwrap();
        let t = |name, slot| Binding {
            texture: name,
            slot,
        };
        assert_eq!(db.blocks, [vec![t(7, 3), t(8, 4)], vec![], vec![t(9, 3)]]);
        assert_eq!(
            db.permutations[0],
            Permutation {
                shader: 0xA,
                block: 2,
                program: 100
            }
        );
        let a: Vec<_> = db.textures(0xA).map(|b| b.texture).collect();
        assert_eq!(a, [9, 7, 8]);
        assert_eq!(db.textures(0xC).count(), 0);
        assert_eq!(db.programs.len(), 103);
        assert_eq!(db.programs[101][0], 101);
        assert_eq!(db.permutations_of(0xA).count(), 2);
    }

    #[test]
    fn reads_a_blocks_constants_in_order() {
        let d = block_with(&[(7, 1)], &["heightfieldPixelSizeAndAspect", "isOpaque"]);
        let (size, textures, constants) = read_block(&d, 0).unwrap();
        assert_eq!(size, d.len());
        assert_eq!(
            textures,
            [Binding {
                texture: 7,
                slot: 1
            }]
        );
        let c = |name: &str, register| Constant {
            name: name.into(),
            register,
            default: [register as f32 + 0.5, 0.0, 0.0, 0.0],
        };
        assert_eq!(
            constants,
            [c("heightfieldPixelSizeAndAspect", 0), c("isOpaque", 1)]
        );
        let mut crossed = d;
        put(&mut crossed, 32, &(1u64 << 20).to_le_bytes());
        assert!(read_block(&crossed, 0).is_err(), "constants past the block");
    }

    /// A program fetching t5 at a heightfield transform and sampling t6 at a global
    /// colour one, t1 bound: t5 and t6 are supplied, each told by its transform.
    #[test]
    fn tells_supplied_textures_by_their_coordinates_transform() {
        use crate::dxbc::tests::{ins, op, res, sample};
        let mad = |dest, constant| {
            ins(
                0x32,
                &[
                    op::dest(dest, 3),
                    op::input(2, [0, 1, 0, 0]),
                    op::constant(1, constant, [0, 1, 0, 0]),
                    op::constant(1, constant, [2, 3, 2, 2]),
                ],
            )
        };
        let code = dxbc::parse(&res(&[
            mad(0, 2),
            ins(
                0x2D,
                &[op::dest(14, 1), op::temp(0, [0, 1, 0, 0]), op::texture(5)],
            ),
            mad(1, 4),
            sample(1, 6),
            mad(2, 3),
            sample(2, 7),
            sample(1, 1),
        ]))
        .unwrap();
        let constants: Vec<Constant> = [
            "heightfieldPixelSizeAndAspect",
            "clipToWorldSpaceXzTransform",
            "worldToHeightfieldUvTransform",
            "worldToMask0UvTransform",
            "worldToGlobalColorUvTransform",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| Constant {
            name: n.to_string(),
            register: i as u32,
            default: [0.0; 4],
        })
        .collect();
        let bound = [Binding {
            texture: 9,
            slot: 1,
        }];
        assert_eq!(
            supplied(&code, &bound, &constants),
            BTreeMap::from([
                (5, Supplied::Heightfield),
                (6, Supplied::GlobalColor),
                (7, Supplied::Mask)
            ])
        );
        assert_eq!(
            supplied(&code, &bound, &[])[&5],
            Supplied::Unknown,
            "no names"
        );
    }

    #[test]
    fn names_a_program_by_its_guid() {
        let guid = [
            0x89, 0x3e, 0x21, 0x62, 0xca, 0x77, 0xa2, 0x0f, 0xfb, 0x1c, 0x28, 0xcb, 0xac, 0xed,
            0xee, 0x24,
        ];
        assert_eq!(
            program_name(&guid),
            "shaders/bytecode/62213e89-77ca-0fa2-fb1c-28cbacedee24"
        );
    }

    #[test]
    fn a_damaged_database_is_an_error() {
        let d = sample(&[&[(7, 3)]], &[(0xA, 0)]);
        assert!(ShaderDb::parse(&d[..d.len() - 1]).is_err());
        let mut far = d.clone();
        put(&mut far, PERMUTATIONS + BLOCK_AT, &1u32.to_le_bytes());
        assert!(ShaderDb::parse(&far).is_err(), "a block out of range");
        let mut program = d.clone();
        put(
            &mut program,
            PERMUTATIONS + PROGRAM_AT,
            &9999u32.to_le_bytes(),
        );
        assert!(ShaderDb::parse(&program).is_err(), "a program out of range");
        let mut many = d.clone();
        put(&mut many, PROGRAM_COUNT_AT, &u32::MAX.to_le_bytes());
        assert!(
            ShaderDb::parse(&many).is_err(),
            "programs past the permutations"
        );
        let mut big = d.clone();
        put(&mut big, BLOCKS, &(1u64 << 40).to_le_bytes());
        assert!(ShaderDb::parse(&big).is_err(), "a block past the end");
        let mut crossed = d;
        put(&mut crossed, BLOCKS + 16, &(1u64 << 20).to_le_bytes());
        assert!(
            ShaderDb::parse(&crossed).is_err(),
            "textures past the block"
        );
    }

    /// The road decals' programs (`rd_dcl_transparentdcl_…`): the 29 writing a colour
    /// paint their colour texture's (t3) red by `lineColor`, darkened when wet, and
    /// cover by its blue and alpha, weighed by `alphaSelector`, never its red or green.
    #[test]
    fn the_road_decals_paint_red_by_line_color_and_cover_by_blue_or_alpha() {
        use crate::dxbc::{Kind, Source};
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(std::path::Path::new(&dir)).unwrap();
        let db = ShaderDb::read(&store).unwrap();
        let mut entries = store.entries().unwrap();
        let t3 = |channel| Source::Texture { slot: 3, channel };
        let mut painting = BTreeMap::new();
        let mut code_of = HashMap::new();
        for shader in ["lerp", "lerpnsmultiply", "lerpbasecolor"] {
            let name = format!("shaders/world/roads/rd_dcl_transparentdcl_{shader}");
            let programs: BTreeMap<u32, u32> = db
                .permutations_of(name_hash(&name))
                .map(|p| (p.program, p.block))
                .collect();
            for (program, block) in programs {
                let asset = store
                    .res(&program_name(&db.programs[program as usize]))
                    .unwrap();
                let code = dxbc::parse(&entries.read(asset.location).unwrap()).unwrap();
                if !dxbc::outputs(&code).contains(&1) {
                    continue;
                }
                let constants = &db.constants[block as usize];
                let register = |name: &str| {
                    let c = constants.iter().find(|c| c.name == name).unwrap();
                    (1, c.register)
                };
                let paint = dxbc::output_sources(&code, 1, 0);
                assert!(paint.contains(&t3(0)), "{program}: {paint:?}");
                let paint = dxbc::output_constants(&code, 1, 0);
                assert!(paint.contains(&register("lineColor")), "{program}");
                assert!(paint.contains(&register("WaterCycle")), "{program}");
                let cover = dxbc::output_sources(&code, 1, 3);
                assert!(
                    cover.contains(&t3(2)) && cover.contains(&t3(3)),
                    "{program}"
                );
                assert!(
                    !cover.contains(&t3(0)) && !cover.contains(&t3(1)),
                    "{program}"
                );
                let cover = dxbc::output_constants(&code, 1, 3);
                for name in ["alphaSelector", "Opacity", "diffuseRGBAOpacity"] {
                    assert!(cover.contains(&register(name)), "{program}: {name}");
                }
                let default =
                    |name: &str| constants.iter().find(|c| c.name == name).unwrap().default;
                assert_eq!(default("lineColor"), [1.0, 1.0, 1.0, 0.0]);
                assert_eq!(default("alphaSelector"), [1.0, 0.0, 0.0, 0.0]);
                assert_eq!(default("diffuseRGBAOpacity"), [0.0; 4]);
                assert_eq!(default("WaterCycle"), [0.0; 4]);
                painting
                    .entry(shader)
                    .or_insert_with(Vec::new)
                    .push(program);
                code_of.insert(program, (code, block));
            }
        }
        let count: Vec<(&str, usize)> = painting.iter().map(|(s, p)| (*s, p.len())).collect();
        assert_eq!(
            count,
            [("lerp", 12), ("lerpbasecolor", 8), ("lerpnsmultiply", 9)]
        );
        // Only `…_lerp` mixes toward a water colour, a dark warm grey.
        for (program, (_, block)) in &code_of {
            let water = db.constants[*block as usize]
                .iter()
                .find(|c| c.name == "WaterColor");
            assert_eq!(water.is_some(), painting["lerp"].contains(program));
            if let Some(w) = water {
                assert_eq!(w.default, [0.143, 0.134, 0.118, 0.0]);
            }
        }
        // 433 and 442 are one program but for t7, the engine's wetness and dirt
        // texture: 433 samples it at a fixed point, 442 at the mesh's first
        // coordinates (its own textures at its second).
        let t7_at = |program: u32| {
            let (code, _) = &code_of[&program];
            let read = code
                .iter()
                .find(|i| {
                    i.operands
                        .iter()
                        .any(|o| o.kind == Kind::Texture && o.register == 7)
                })
                .unwrap();
            read.operands[1].kind
        };
        assert_eq!(t7_at(433), Kind::Literal([0.0; 4]));
        assert_eq!(t7_at(442), Kind::Temp);
    }
}
