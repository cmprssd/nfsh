//! Bundles stored in a superbundle's `.sb`, at the offset its `.toc` gives.
//!
//! A bundle starts with eight big-endian words: the offset of the binary bundle, the
//! offset of the location records (relative to the binary bundle), the offset of the
//! location flags, the entry count, then three copies of the end of the binary bundle
//! and a zero. The binary bundle (magic `9D798ED6`) lists the entries: SHA-1s, then
//! ebx, res and chunk records, a metadata DbObject and a name table. The location
//! records then say where each entry's stored bytes are, in entry order.

use crate::reader::{Reader, cstr_at};
use crate::{CasRef, Location, Result, bail};

const BUNDLE_MAGIC: u32 = 0x9d79_8ed6;

/// An ebx (asset description) or res (resource) entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub sha1: [u8; 20],
    /// Size once decompressed.
    pub original_size: u32,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Res {
    pub asset: Asset,
    /// Hash of the lowercased resource type name (see [`res_type_hash`]).
    pub res_type: u32,
}

/// A chunk (bulk data such as texture mips) listed in a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub id: [u8; 16],
    pub sha1: [u8; 20],
    /// The part of the chunk this bundle uses, in decompressed bytes.
    pub logical_offset: u32,
    pub logical_size: u32,
    pub location: Location,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bundle {
    pub ebx: Vec<Asset>,
    pub res: Vec<Res>,
    pub chunks: Vec<Chunk>,
}

/// The hash Heat uses for resource type names: `h = h * 33 ^ byte` from 5381, over the
/// lowercased name.
pub fn res_type_hash(name: &str) -> u32 {
    name.bytes().fold(5381u32, |h, b| {
        h.wrapping_mul(33) ^ u32::from(b.to_ascii_lowercase())
    })
}

/// Reads one bundle from its bytes in the `.sb`.
pub fn parse(data: &[u8]) -> Result<Bundle> {
    let mut r = Reader::new(data);
    let bundle_at = r.u32_be()? as usize;
    let locs_rel = r.u32_be()? as usize;
    let flags_at = r.u32_be()? as usize;
    let count = r.u32_be()? as usize;

    let mut r = Reader::at(data, bundle_at)?;
    let size = r.u32_be()? as usize;
    let base = r.pos();
    let Some(binary) = data.get(base..).and_then(|rest| rest.get(..size)) else {
        bail!("binary bundle ({size} bytes at {base:#x}) runs past the end");
    };
    let mut r = Reader::new(binary);
    let magic = r.u32_be()?;
    if magic != BUNDLE_MAGIC {
        bail!("bundle magic is {magic:#010x}, expected {BUNDLE_MAGIC:#010x}");
    }
    let total = r.u32_be()? as usize;
    let ebx_count = r.u32_be()? as usize;
    let res_count = r.u32_be()? as usize;
    let chunk_count = r.u32_be()? as usize;
    let strings_at = r.u32_be()? as usize;
    let _meta_at = r.u32_be()?;
    let _meta_size = r.u32_be()?;
    if ebx_count
        .checked_add(res_count)
        .and_then(|n| n.checked_add(chunk_count))
        != Some(total)
        || total != count
    {
        bail!(
            "bundle counts disagree: {ebx_count} ebx + {res_count} res + {chunk_count} chunks, total {total}, {count} locations"
        );
    }
    let strings = binary.get(strings_at..).unwrap_or_default();
    let mut sha1s = Vec::with_capacity(total.min(binary.len() / 20));
    for _ in 0..total {
        sha1s.push(r.array::<20>()?);
    }
    let mut names = |n: usize| -> Result<Vec<(String, u32)>> {
        (0..n)
            .map(|_| {
                let name = cstr_at(strings, r.u32_be()? as usize)?.to_owned();
                Ok((name, r.u32_be()?))
            })
            .collect()
    };
    let ebx_names = names(ebx_count)?;
    let res_names = names(res_count)?;
    let mut res_types = Vec::with_capacity(res_count);
    for _ in 0..res_count {
        res_types.push(r.u32_be()?);
    }
    r.bytes(res_count * 16)?; // per-resource metadata
    r.bytes(res_count * 8)?; // resource ids
    let mut chunk_heads = Vec::with_capacity(chunk_count);
    for _ in 0..chunk_count {
        chunk_heads.push((r.array::<16>()?, r.u32_be()?, r.u32_be()?));
    }

    let locations = read_locations(data, bundle_at + locs_rel, flags_at, total)?;
    let mut sha1s = sha1s.into_iter();
    let mut locations = locations.into_iter();
    let mut next = || {
        (
            sha1s.next().unwrap_or_default(),
            locations.next().unwrap_or(EMPTY),
        )
    };
    let mut asset = |(name, original_size): (String, u32)| {
        let (sha1, location) = next();
        Asset {
            name,
            sha1,
            original_size,
            location,
        }
    };
    let ebx = ebx_names.into_iter().map(&mut asset).collect();
    let res = res_names
        .into_iter()
        .zip(res_types)
        .map(|(name, res_type)| Res {
            asset: asset(name),
            res_type,
        })
        .collect();
    let chunks = chunk_heads
        .into_iter()
        .map(|(id, logical_offset, logical_size)| {
            let (sha1, location) = next();
            Chunk {
                id,
                sha1,
                logical_offset,
                logical_size,
                location,
            }
        })
        .collect();
    Ok(Bundle { ebx, res, chunks })
}

/// Never used: the counts were checked to match before any entry is built.
const EMPTY: Location = Location {
    cas: CasRef {
        patch: false,
        package: 0,
        cas: 0,
    },
    offset: 0,
    size: 0,
};

/// One flag byte per entry at `flags_at`; flag 1 means the record starts with a new
/// cas reference, flag 0 that it is in the same archive as the entry before. Each
/// record then has the offset and the stored size.
fn read_locations(
    data: &[u8],
    locs_at: usize,
    flags_at: usize,
    count: usize,
) -> Result<Vec<Location>> {
    let Some(flags) = data.get(flags_at..).and_then(|rest| rest.get(..count)) else {
        bail!("{count} location flags at {flags_at:#x} run past the end");
    };
    let mut r = Reader::at(data, locs_at)?;
    let mut cas = None;
    let mut out = Vec::with_capacity(count);
    for (i, &flag) in flags.iter().enumerate() {
        match flag {
            1 => cas = Some(CasRef::from_word(r.u32_be()?)?),
            0 => {}
            other => bail!("location {i} has unknown flag {other}"),
        }
        let Some(cas) = cas else {
            bail!("location {i} names no archive");
        };
        out.push(Location {
            cas,
            offset: r.u32_be()?,
            size: r.u32_be()?,
        });
    }
    if r.pos() != flags_at {
        bail!(
            "location records end at {:#x}, but the flags start at {flags_at:#x}",
            r.pos()
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(out: &mut Vec<u8>, words: &[u32]) {
        out.extend(words.iter().flat_map(|w| w.to_be_bytes()));
    }

    /// A bundle with one ebx, one res and one chunk; the res and chunk share the
    /// ebx's archive.
    fn sample() -> Vec<u8> {
        let mut bin = Vec::new();
        // Header with string/meta offsets filled below; tables are 0x24 + 3*20 +
        // 8 + 8 + 4 + 16 + 8 + 24 = 0xa0 bytes long.
        words(&mut bin, &[BUNDLE_MAGIC, 3, 1, 1, 1, 0xa0, 0xa0, 0]);
        for i in 0..3 {
            bin.extend_from_slice(&[i; 20]);
        }
        words(&mut bin, &[0, 400, 2, 14164, res_type_hash("Texture")]);
        bin.extend_from_slice(&[0; 24]);
        bin.extend_from_slice(&[7; 16]);
        words(&mut bin, &[0, 65536]);
        bin.extend_from_slice(b"a\0bc\0");
        let mut data = Vec::new();
        let locs_rel = 4 + bin.len() as u32;
        let flags_at = 0x20 + locs_rel + 12 + 8 + 8;
        words(&mut data, &[0x20, locs_rel, flags_at, 3, 0, 0, 0, 0]);
        words(&mut data, &[bin.len() as u32]);
        data.extend(bin);
        words(
            &mut data,
            &[0x0000_0002, 0x100, 0x129, 0x229, 0x40, 0x269, 0x1000],
        );
        data.extend_from_slice(&[1, 0, 0]);
        data
    }

    #[test]
    fn reads_entries_and_their_locations() {
        let bundle = parse(&sample()).unwrap();
        let cas = CasRef {
            patch: false,
            package: 0,
            cas: 2,
        };
        assert_eq!(bundle.ebx.len(), 1);
        assert_eq!(bundle.ebx[0].name, "a");
        assert_eq!(bundle.ebx[0].original_size, 400);
        assert_eq!(
            bundle.ebx[0].location,
            Location {
                cas,
                offset: 0x100,
                size: 0x129
            }
        );
        assert_eq!(bundle.res[0].asset.name, "bc");
        assert_eq!(bundle.res[0].asset.sha1, [1; 20]);
        assert_eq!(bundle.res[0].res_type, 0x6bde_20ba);
        assert_eq!(bundle.chunks[0].id, [7; 16]);
        assert_eq!(bundle.chunks[0].logical_size, 65536);
        assert_eq!(
            bundle.chunks[0].location,
            Location {
                cas,
                offset: 0x269,
                size: 0x1000
            }
        );
    }

    #[test]
    fn rejects_inconsistent_bundles() {
        let good = sample();
        let mut bad = good.clone();
        let flags = bad.len() - 3;
        bad[flags] = 0; // first entry without an archive
        assert!(parse(&bad).is_err());
        let mut bad = good.clone();
        bad[0x24 + 4..0x24 + 8].copy_from_slice(&4u32.to_be_bytes()); // total
        assert!(parse(&bad).is_err());
        for len in [0, 10, 0x40, good.len() - 1] {
            assert!(parse(&good[..len]).is_err(), "truncated to {len}");
        }
    }

    #[test]
    fn res_type_hashes_match_heats_names() {
        // Hashes seen in Heat's bundles next to assets of the named kind.
        assert_eq!(res_type_hash("Texture"), 0x6bde_20ba);
        assert_eq!(res_type_hash("MeshSet"), 0x49b1_56d4);
    }
}
