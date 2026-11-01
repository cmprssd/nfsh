//! A level's shader database: its parameter blocks (the textures each shader binds)
//! and its shader permutations (see docs/FORMATS.md, "Terrain layers").
//! Little-endian.

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
/// Where Palm City's file holds the permutations and their count; the section
/// between the blocks and them is not worked out, so this is not found from the file.
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

/// One compiled variant of a shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permutation {
    /// Its shader's name hash ([`crate::ebx::name_hash`] of its lowercase name).
    pub shader: u32,
    /// Its parameter block, an index into [`ShaderDb::blocks`].
    pub block: u32,
    /// Its program's index among the bytecode res; not read yet.
    pub program: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShaderDb {
    /// Each parameter block's textures, in the order it lists them.
    pub blocks: Vec<Vec<Binding>>,
    pub permutations: Vec<Permutation>,
}

impl ShaderDb {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let count = Reader::at(data, BLOCK_COUNT_AT)?.u32_le()? as usize;
        let mut blocks = Vec::with_capacity(count.min(data.len() / BLOCK_HEADER));
        let mut at = BLOCKS;
        for b in 0..count {
            let block = read_block(data, at).map_err(|e| Error(format!("block {b}: {e}")))?;
            blocks.push(block.1);
            at += block.0;
        }
        if at > PERMUTATION_COUNT_AT {
            bail!("the blocks end at {at:#x}, past the permutations");
        }
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
            permutations.push(Permutation {
                shader,
                block,
                program: word(PROGRAM_AT),
            });
        }
        Ok(Self {
            blocks,
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

    /// The textures the shader whose name hash is `shader` binds, in every
    /// permutation of it; empty when it has none.
    pub fn textures(&self, shader: u32) -> impl Iterator<Item = &Binding> {
        self.permutations
            .iter()
            .filter(move |p| p.shader == shader)
            .flat_map(|p| &self.blocks[p.block as usize])
    }
}

/// Reads the parameter block at `at`: its size and its textures.
fn read_block(data: &[u8], at: usize) -> Result<(usize, Vec<Binding>)> {
    let mut r = Reader::at(data, at)?;
    let size = r.u64_le()?;
    let mut offsets = [0; 6];
    for o in &mut offsets {
        *o = r.u64_le()?;
    }
    let (from, to) = (offsets[1], offsets[2]);
    if size < BLOCK_HEADER as u64 || !(from <= to && to <= size) {
        bail!("size {size}, textures from {from} to {to}");
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
    Reader::at(data, at)?.bytes(size as usize)?;
    Ok((size as usize, textures))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
        d[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A block binding `textures` (name hash, slot), with 8 bytes of constants after.
    fn block(textures: &[(u32, u32)]) -> Vec<u8> {
        let end = (BLOCK_HEADER + TEXTURE * textures.len()) as u64;
        let mut d = Vec::new();
        for v in [
            end + 8,
            0,
            BLOCK_HEADER as u64,
            end,
            end + 8,
            end + 8,
            end + 8,
        ] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        for &(name, slot) in textures {
            d.extend_from_slice(&name.to_le_bytes());
            d.extend_from_slice(&slot.to_le_bytes());
            d.extend_from_slice(&[0; 8]);
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
    }

    #[test]
    fn a_damaged_database_is_an_error() {
        let d = sample(&[&[(7, 3)]], &[(0xA, 0)]);
        assert!(ShaderDb::parse(&d[..d.len() - 1]).is_err());
        let mut far = d.clone();
        put(&mut far, PERMUTATIONS + BLOCK_AT, &1u32.to_le_bytes());
        assert!(ShaderDb::parse(&far).is_err(), "a block out of range");
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
}
