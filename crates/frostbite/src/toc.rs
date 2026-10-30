//! Superbundle tables of contents (`*.toc`) and the signed header shared with
//! `layout.toc` and `initfs_Win32`.
//!
//! The body of a `.toc` starts with twelve big-endian words; the ones we use are:
//!
//! | word | meaning |
//! |------|---------|
//! | 1, 2 | offset of the bundle records, bundle count |
//! | 4, 5 | offset of the chunk records, chunk count |
//! | 8    | offset of the name table |
//! | 9, 10 | offset and length (in words) of the chunk locations |
//!
//! Words 0 and 3 point at hash tables for lookup by name or id, which a full listing
//! does not need. Offsets count from the start of the body.

use crate::reader::{Reader, cstr_at};
use crate::{CasRef, Location, Result, bail};

/// Signed files start with this magic, then a signature, then the plain body.
const SIGNED_MAGIC: [u8; 4] = [0x00, 0xd1, 0xce, 0x01];
const SIGNED_HEADER_LEN: usize = 0x22c;

/// The plain body of a signed file (everything after its 0x22C-byte header).
pub fn signed_body(file: &[u8]) -> Result<&[u8]> {
    if file.get(..4) != Some(&SIGNED_MAGIC[..]) {
        bail!(
            "not a signed Frostbite file (magic {:02x?})",
            file.get(..4).unwrap_or(file)
        );
    }
    match file.get(SIGNED_HEADER_LEN..) {
        Some(body) => Ok(body),
        None => bail!("signed file is shorter than its {SIGNED_HEADER_LEN:#x}-byte header"),
    }
}

/// A bundle listed in a `.toc`: its bytes are `size` bytes at `offset` in the `.sb`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocBundle {
    pub name: String,
    pub offset: u64,
    pub size: u32,
}

/// A chunk stored directly under a superbundle rather than inside a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocChunk {
    pub id: [u8; 16],
    pub location: Location,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toc {
    pub bundles: Vec<TocBundle>,
    pub chunks: Vec<TocChunk>,
}

/// Reads a whole `.toc` file.
pub fn parse(file: &[u8]) -> Result<Toc> {
    parse_body(signed_body(file)?)
}

pub fn parse_body(body: &[u8]) -> Result<Toc> {
    let mut header = Reader::new(body);
    let mut word = [0usize; 12];
    for w in &mut word {
        *w = header.u32_be()? as usize;
    }
    let [
        _,
        bundles_at,
        bundle_count,
        _,
        chunks_at,
        chunk_count,
        _,
        _,
        names_at,
        locs_at,
        locs_len,
        _,
    ] = word;
    let names = body.get(names_at..).unwrap_or_default();

    let mut r = Reader::at(body, bundles_at)?;
    let mut bundles = Vec::with_capacity(bundle_count.min(body.len() / 16));
    for _ in 0..bundle_count {
        let name_at = r.u32_be()? as usize;
        let size = r.u32_be()?;
        let offset = r.u64_be()?;
        let name = cstr_at(names, name_at)?.to_owned();
        bundles.push(TocBundle { name, offset, size });
    }

    let locs = match locs_len
        .checked_mul(4)
        .and_then(|len| body.get(locs_at..)?.get(..len))
    {
        Some(locs) => locs,
        None => bail!("chunk locations ({locs_len} words at {locs_at:#x}) run past the end"),
    };
    let mut r = Reader::at(body, chunks_at)?;
    let mut chunks = Vec::with_capacity(chunk_count.min(body.len() / 20));
    for _ in 0..chunk_count {
        let id = r.array()?;
        // The top byte is a flag (always 1 in Heat); the rest indexes the location words.
        let index = (r.u32_be()? & 0x00ff_ffff) as usize;
        let mut l = Reader::at(locs, index * 4)?;
        let location = Location {
            cas: CasRef::from_word(l.u32_be()?)?,
            offset: l.u32_be()?,
            size: l.u32_be()?,
        };
        chunks.push(TocChunk { id, location });
    }
    Ok(Toc { bundles, chunks })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body with one bundle and one chunk, laid out as Heat's files are: header,
    /// bundle hash table, bundle records, chunk hash table, chunk records, names
    /// (padded to 4 bytes), chunk locations.
    fn sample_body() -> Vec<u8> {
        let words: [u32; 12] = [0x30, 0x34, 1, 0x44, 0x48, 1, 0, 0, 0x5c, 0x6c, 3, 0];
        let mut body: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
        body.extend_from_slice(&[0xff; 4]);
        body.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0x48, 0, 0, 0, 0, 0, 0x10, 0xd5, 0x98]);
        body.extend_from_slice(&[0xff; 4]);
        body.extend_from_slice(&[0xab; 16]);
        body.extend_from_slice(&0x0100_0000u32.to_be_bytes());
        body.extend_from_slice(b"Win32/globals\0\0\0");
        for w in [0x0000_0105u32, 0x08d3_e77d, 0x8459] {
            body.extend_from_slice(&w.to_be_bytes());
        }
        assert_eq!(body.len(), 0x78);
        body
    }

    #[test]
    fn lists_bundles_and_chunks() {
        let toc = parse_body(&sample_body()).unwrap();
        assert_eq!(
            toc.bundles,
            [TocBundle {
                name: "Win32/globals".into(),
                offset: 0x10_d598,
                size: 0x48
            }]
        );
        assert_eq!(toc.chunks.len(), 1);
        assert_eq!(toc.chunks[0].id, [0xab; 16]);
        assert_eq!(
            toc.chunks[0].location,
            Location {
                cas: CasRef {
                    patch: false,
                    package: 1,
                    cas: 5
                },
                offset: 0x08d3_e77d,
                size: 0x8459
            }
        );
    }

    #[test]
    fn rejects_out_of_range_tables() {
        let mut body = sample_body();
        body[0x14..0x18].copy_from_slice(&1000u32.to_be_bytes()); // chunk count
        assert!(parse_body(&body).is_err());
        assert!(parse_body(&body[..0x20]).is_err());
    }

    #[test]
    fn signed_header_is_skipped_and_checked() {
        let mut file = vec![0; SIGNED_HEADER_LEN];
        file[..4].copy_from_slice(&SIGNED_MAGIC);
        file.extend_from_slice(b"body");
        assert_eq!(signed_body(&file), Ok(&b"body"[..]));
        assert!(signed_body(&file[..0x100]).is_err());
        assert!(signed_body(b"\0\xd1\xce\0rest").is_err());
    }
}
