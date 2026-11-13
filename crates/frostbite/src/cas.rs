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
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

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

/// Reads stored bytes from archives (an install's, or a store's packs), keeping each
/// archive open once read.
pub struct Archives<'a> {
    path: Box<dyn Fn(CasRef) -> Option<PathBuf> + 'a>,
    open: HashMap<CasRef, (PathBuf, File)>,
}

impl<'a> Archives<'a> {
    pub fn new(install: &'a Install) -> Self {
        Self::with_paths(|cas| install.cas_path(cas))
    }

    /// Archives found by `path`, which gives `None` for a reference it doesn't know.
    pub fn with_paths(path: impl Fn(CasRef) -> Option<PathBuf> + 'a) -> Self {
        Self {
            path: Box::new(path),
            open: HashMap::new(),
        }
    }

    /// The stored (still compressed) bytes at `location`.
    pub fn stored(&mut self, location: Location) -> Result<Vec<u8>> {
        self.span(location)
    }

    /// The bytes of `span`, which may cover several entries (see [`spans`]).
    pub fn span(&mut self, span: Location) -> Result<Vec<u8>> {
        if !self.open.contains_key(&span.cas) {
            let Some(path) = (self.path)(span.cas) else {
                bail!("{:?} names an unknown archive", span.cas);
            };
            let file =
                File::open(&path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
            self.open.insert(span.cas, (path, file));
        }
        let (path, file) = &self.open[&span.cas];
        let mut out = vec![0; span.size as usize];
        read_at(
            file,
            u16::from(span.cas.cas),
            &mut out,
            u64::from(span.offset),
        )
        .map_err(|e| {
            crate::Error(format!(
                "{}: {} bytes at {:#x}: {e}",
                path.display(),
                span.size,
                span.offset
            ))
        })?;
        Ok(out)
    }

    /// The decompressed bytes at `location`.
    pub fn read(&mut self, location: Location, decoder: &dyn Decoder) -> Result<Vec<u8>> {
        decompress(&self.stored(location)?, decoder)
    }
}

/// Entries closer than this in an archive are read in one go by [`spans`]: on the
/// owner's USB hard disk a seek costs about 7 to 9 ms, as long as reading 0.7 MB, and
/// merging across 4 KiB to 1 MiB gaps all read zone 494's meshes in about the same
/// time (docs/BASELINE.md, "Where zone 494's load goes").
pub const SPAN_GAP: u32 = 64 * 1024;

/// The reads that cover `locations`: sorted by archive and offset, each merged with
/// the next when at most `gap` bytes lie between them; each span with the locations it
/// covers, once each.
pub fn spans(
    locations: impl IntoIterator<Item = Location>,
    gap: u32,
) -> Vec<(Location, Vec<Location>)> {
    let mut sorted: Vec<Location> = locations.into_iter().collect();
    sorted.sort_by_key(|l| (l.cas, l.offset, l.size));
    sorted.dedup();
    let mut out: Vec<(Location, Vec<Location>)> = Vec::new();
    for l in sorted {
        let end = u64::from(l.offset) + u64::from(l.size);
        match out.last_mut() {
            Some((span, members))
                if span.cas == l.cas
                    && u64::from(l.offset)
                        <= u64::from(span.offset) + u64::from(span.size) + u64::from(gap)
                    && end - u64::from(span.offset) <= u64::from(u32::MAX) =>
            {
                span.size = span.size.max((end - u64::from(span.offset)) as u32);
                members.push(l);
            }
            _ => out.push((l, vec![l])),
        }
    }
    out
}

/// Bytes and reads of stored entries since the process started, over every thread.
static READ_BYTES: AtomicU64 = AtomicU64::new(0);
static READS: AtomicU64 = AtomicU64::new(0);

/// How many stored bytes, in how many reads, the archives (packs or an install's
/// files) have been read for so far in this process: what a load costs the disk.
pub fn read_so_far() -> (u64, u64) {
    (
        READ_BYTES.load(Ordering::Relaxed),
        READS.load(Ordering::Relaxed),
    )
}

/// Where each read is written as `file offset size` (decimal) when `NFSH_READ_TRACE`
/// names a file: what a load asks of the disk, in order. `file` is a pack's number,
/// or an install file's index.
fn trace() -> Option<&'static Mutex<BufWriter<File>>> {
    static TRACE: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();
    TRACE
        .get_or_init(|| {
            let path = std::env::var_os("NFSH_READ_TRACE")?;
            match File::create(&path) {
                Ok(f) => Some(Mutex::new(BufWriter::new(f))),
                Err(e) => {
                    eprintln!("NFSH_READ_TRACE {}: {e}", Path::new(&path).display());
                    None
                }
            }
        })
        .as_ref()
}

/// Writes out what the read trace holds so far; nothing without a trace.
pub fn flush_trace() {
    if let Some(t) = trace()
        && let Ok(mut w) = t.lock()
    {
        let _ = w.flush();
    }
}

/// Reads exactly `buf.len()` bytes of `file` (numbered `id` in the trace) at `offset`,
/// and counts them.
pub(crate) fn read_at(file: &File, id: u16, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    READ_BYTES.fetch_add(buf.len() as u64, Ordering::Relaxed);
    READS.fetch_add(1, Ordering::Relaxed);
    if let Some(t) = trace()
        && let Ok(mut w) = t.lock()
    {
        let _ = writeln!(w, "{id} {offset} {}", buf.len());
    }
    read_exact_at(file, buf, offset)
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
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

    fn at(cas: u8, offset: u32, size: u32) -> Location {
        Location {
            cas: CasRef {
                patch: false,
                package: 0,
                cas,
            },
            offset,
            size,
        }
    }

    #[test]
    fn spans_merge_neighbours_within_the_gap_in_each_archive() {
        let got = spans(
            [
                at(1, 100, 10), // 100..110
                at(0, 0, 10),
                at(1, 0, 50),  // 50 short of the entry at 100
                at(1, 105, 2), // inside 100..110
                at(1, 300, 5), // 190 past 110
                at(1, 0, 50),  // asked twice
            ],
            50,
        );
        assert_eq!(
            got,
            vec![
                (at(0, 0, 10), vec![at(0, 0, 10)]),
                (
                    at(1, 0, 110),
                    vec![at(1, 0, 50), at(1, 100, 10), at(1, 105, 2)]
                ),
                (at(1, 300, 5), vec![at(1, 300, 5)]),
            ]
        );
        // Without a gap, only entries that touch or overlap share a read.
        let got: Vec<Location> = spans([at(0, 0, 10), at(0, 10, 5), at(0, 16, 1)], 0)
            .into_iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(got, vec![at(0, 0, 15), at(0, 16, 1)]);
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
