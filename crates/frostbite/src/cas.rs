//! Stored entries in the `cas_NN.cas` archives. An entry's stored bytes are a chain of
//! blocks, each an 8-byte big-endian header (decompressed size u32, codec u16, stored
//! size u16) and its payload, with no gap between them. A block holds at most 64 KiB
//! decompressed.
//!
//! Codecs seen in Heat's files: `0x1170` is Oodle, and a high byte of zero means the
//! payload is stored as is (`0x0070`, and `0x0071`, whose stored size field wraps to 0
//! for a full 64 KiB block). Raw payloads are as long as the decompressed size.

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;

use crate::install::Install;
use crate::reader::Reader;
use crate::{CasRef, Location, Result, bail};

/// The largest block, decompressed.
pub const MAX_BLOCK: usize = 0x10000;

const OODLE: u16 = 0x1170;

/// Decompresses one Oodle block into `dst`, which is exactly the decompressed size.
pub trait Decoder {
    fn decompress(&self, src: &[u8], dst: &mut [u8]) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Raw,
    Oodle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub codec: Codec,
    /// Where the payload starts in the stored bytes.
    pub offset: usize,
    pub stored_size: usize,
    pub size: usize,
}

/// The blocks of one entry's stored bytes, which must end exactly at the last block.
pub fn blocks(stored: &[u8]) -> Result<Vec<Block>> {
    let mut r = Reader::new(stored);
    let mut out = Vec::new();
    while r.pos() < stored.len() {
        let start = r.pos();
        let size = r.u32_be()? as usize;
        let codec = u16::from_be_bytes(r.array()?);
        let stored_size = usize::from(u16::from_be_bytes(r.array()?));
        if size == 0 || size > MAX_BLOCK {
            bail!("block at {start:#x} has decompressed size {size:#x}");
        }
        let (codec, stored_size) = match codec {
            0x0070 | 0x0071 if stored_size == size % MAX_BLOCK => (Codec::Raw, size),
            OODLE if stored_size != 0 => (Codec::Oodle, stored_size),
            _ => bail!(
                "block at {start:#x} has codec {codec:#06x} with stored size \
                 {stored_size:#x} for {size:#x} bytes"
            ),
        };
        let offset = r.pos();
        r.bytes(stored_size)?;
        out.push(Block {
            codec,
            offset,
            stored_size,
            size,
        });
    }
    Ok(out)
}

/// Decompresses one entry's stored bytes.
pub fn decompress(stored: &[u8], decoder: &dyn Decoder) -> Result<Vec<u8>> {
    let blocks = blocks(stored)?;
    let mut out = vec![0; blocks.iter().map(|b| b.size).sum()];
    let mut pos = 0;
    for b in blocks {
        let src = &stored[b.offset..b.offset + b.stored_size];
        let dst = &mut out[pos..pos + b.size];
        match b.codec {
            Codec::Raw => dst.copy_from_slice(src),
            Codec::Oodle => decoder
                .decompress(src, dst)
                .map_err(|e| crate::Error(format!("block at {:#x}: {e}", b.offset - 8)))?,
        }
        pos += b.size;
    }
    Ok(out)
}

/// Reads stored bytes from an install's archives, keeping each archive open once read.
pub struct Archives<'a> {
    install: &'a Install,
    open: HashMap<CasRef, (PathBuf, File)>,
}

impl<'a> Archives<'a> {
    pub fn new(install: &'a Install) -> Self {
        Self {
            install,
            open: HashMap::new(),
        }
    }

    /// The stored (still compressed) bytes at `location`.
    pub fn stored(&mut self, location: Location) -> Result<Vec<u8>> {
        if !self.open.contains_key(&location.cas) {
            let Some(path) = self.install.cas_path(location.cas) else {
                bail!("{:?} names an unknown install package", location.cas);
            };
            let file =
                File::open(&path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
            self.open.insert(location.cas, (path, file));
        }
        let (path, file) = &self.open[&location.cas];
        let mut out = vec![0; location.size as usize];
        read_at(file, &mut out, u64::from(location.offset)).map_err(|e| {
            crate::Error(format!(
                "{}: {} bytes at {:#x}: {e}",
                path.display(),
                location.size,
                location.offset
            ))
        })?;
        Ok(out)
    }

    /// The decompressed bytes at `location`.
    pub fn read(&mut self, location: Location, decoder: &dyn Decoder) -> Result<Vec<u8>> {
        decompress(&self.stored(location)?, decoder)
    }
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset)? {
            0 => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            n => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands in for Oodle: each "compressed" byte expands to two copies of itself.
    struct Doubler;

    impl Decoder for Doubler {
        fn decompress(&self, src: &[u8], dst: &mut [u8]) -> Result<()> {
            if dst.len() != src.len() * 2 {
                bail!("bad sizes");
            }
            for (i, &b) in src.iter().enumerate() {
                dst[2 * i] = b;
                dst[2 * i + 1] = b;
            }
            Ok(())
        }
    }

    fn block(size: u32, codec: u16, stored: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = size.to_be_bytes().to_vec();
        out.extend(codec.to_be_bytes());
        out.extend(stored.to_be_bytes());
        out.extend(payload);
        out
    }

    #[test]
    fn decompresses_raw_and_oodle_blocks_in_order() {
        let mut stored = block(3, 0x0070, 3, b"abc");
        stored.extend(block(4, OODLE, 2, b"xy"));
        assert_eq!(decompress(&stored, &Doubler), Ok(b"abcxxyy".to_vec()));
    }

    #[test]
    fn a_full_raw_block_has_stored_size_zero() {
        let payload = vec![7; MAX_BLOCK];
        let stored = block(MAX_BLOCK as u32, 0x0071, 0, &payload);
        assert_eq!(decompress(&stored, &Doubler), Ok(payload));
    }

    #[test]
    fn rejects_malformed_chains() {
        // Payload runs past the end.
        assert!(blocks(&block(4, OODLE, 9, b"xy")).is_err());
        // Trailing bytes too short for a header.
        let mut stored = block(3, 0x0070, 3, b"abc");
        stored.push(0);
        assert!(blocks(&stored).is_err());
        // Raw payload shorter than the decompressed size.
        assert!(blocks(&block(4, 0x0070, 2, b"ab")).is_err());
        // Unknown codec, oversized and empty blocks.
        assert!(blocks(&block(2, 0x0f70, 2, b"ab")).is_err());
        assert!(blocks(&block(0x10001, OODLE, 2, b"ab")).is_err());
        assert!(blocks(&block(0, 0x0070, 0, b"")).is_err());
        // The decoder's own failure is reported with the block.
        let err = decompress(&block(5, OODLE, 2, b"xy"), &Doubler).unwrap_err();
        assert_eq!(err.0, "block at 0x0: bad sizes");
    }

    /// Decompresses a sample of the owner's install through its own Oodle library,
    /// when `NFSH_INSTALL` points at it (local-ci sets it on the Windows side).
    /// `nfsh verify` checks every entry.
    #[test]
    fn real_entries_decompress_to_their_listed_size() {
        let Some(root) = std::env::var_os("NFSH_INSTALL") else {
            eprintln!("NFSH_INSTALL not set; skipped");
            return;
        };
        let root = std::path::Path::new(&root);
        let install = Install::open(root).unwrap();
        let oodle = crate::oodle::Oodle::load(root).unwrap();
        let sb = install.superbundle("Win32/globals").unwrap().unwrap();
        let mut archives = Archives::new(&install);
        let mut checked = 0;
        for b in sb.bundles.iter().take(50).map(|b| &b.bundle) {
            let res = b.res.iter().map(|r| &r.asset);
            for a in b.ebx.iter().chain(res) {
                let data = archives.read(a.location, &oodle).unwrap();
                assert_eq!(data.len(), a.original_size as usize, "{}", a.name);
                checked += 1;
            }
        }
        assert!(checked > 100, "only {checked} entries");
    }
}
