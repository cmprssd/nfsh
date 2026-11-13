//! `nfsh replay`: reads again what a read trace (`NFSH_READ_TRACE`) recorded, past
//! Windows' file cache, and times it: what those reads cost a cold disk, run after
//! run. With a gap, the reads are first sorted by where they are and merged when no
//! more than the gap lies between them, as a reader that coalesces would ask.

use std::fs::File;
use std::path::Path;
use std::time::Instant;

use frostbite::{CasRef, Location};

/// One read: a pack's number, an offset and a size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read {
    pub file: u8,
    pub offset: u32,
    pub size: u32,
}

/// The reads of a trace, one `file offset size` line each, of a store's packs.
pub fn parse(text: &str) -> Result<Vec<Read>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(n, line)| {
            let mut f = line.split_whitespace();
            match (
                f.next().map(str::parse),
                f.next().map(str::parse),
                f.next().map(str::parse),
                f.next(),
            ) {
                (Some(Ok(file)), Some(Ok(offset)), Some(Ok(size)), None) => {
                    Ok(Read { file, offset, size })
                }
                _ => Err(format!("line {}: not FILE OFFSET SIZE: {line}", n + 1)),
            }
        })
        .collect()
}

/// The reads to make: the trace's own without a gap; otherwise sorted and merged as
/// nfsh's read-ahead does ([`frostbite::cas::spans`]).
pub fn plan(reads: &[Read], gap: Option<u32>) -> Vec<Read> {
    let Some(gap) = gap else {
        return reads.to_vec();
    };
    let locations = reads.iter().map(|r| Location {
        cas: CasRef {
            patch: false,
            package: 0,
            cas: r.file,
        },
        offset: r.offset,
        size: r.size,
    });
    frostbite::cas::spans(locations, gap)
        .into_iter()
        .map(|(l, _)| Read {
            file: l.cas.cas,
            offset: l.offset,
            size: l.size,
        })
        .collect()
}

/// What reading the plan cost.
#[derive(Debug)]
pub struct Timed {
    pub reads: usize,
    pub bytes: u64,
    pub seconds: f64,
}

/// Reads `TRACE` again from the store in `store`, past the file cache, as is or
/// merged across `gap` bytes.
pub fn run(store: &Path, trace: &Path, gap: Option<u32>) -> Result<String, String> {
    let text = std::fs::read_to_string(trace).map_err(|e| format!("{}: {e}", trace.display()))?;
    let reads = parse(&text)?;
    let planned = plan(&reads, gap);
    let timed = time(store, &planned)?;
    let asked: u64 = reads.iter().map(|r| u64::from(r.size)).sum();
    Ok(format!(
        "{} reads of {:.1} MiB in the trace; read as {} reads of {:.1} MiB{} in {:.2} s, \
         {:.2} ms a read, {}\n",
        reads.len(),
        mib(asked),
        timed.reads,
        mib(timed.bytes),
        match gap {
            Some(g) => format!(" (sorted, merged across up to {g} bytes)"),
            None => " (as traced)".into(),
        },
        timed.seconds,
        timed.seconds * 1000.0 / timed.reads.max(1) as f64,
        if cfg!(windows) {
            "unbuffered"
        } else {
            "through the file cache"
        }
    ))
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / f64::from(1 << 20)
}

fn time(store: &Path, planned: &[Read]) -> Result<Timed, String> {
    let mut open: Vec<Option<File>> = Vec::new();
    let mut buf = Aligned::default();
    let mut bytes = 0;
    let start = Instant::now();
    for r in planned {
        let i = usize::from(r.file);
        if open.len() <= i {
            open.resize_with(i + 1, || None);
        }
        if open[i].is_none() {
            let path = store.join(frostbite::store::pack_name(r.file));
            open[i] = Some(open_uncached(&path).map_err(|e| format!("{}: {e}", path.display()))?);
        }
        let Some(file) = open[i].as_ref() else {
            return Err("pack not open".into());
        };
        bytes += read_aligned(file, &mut buf, r.offset.into(), r.size.into())
            .map_err(|e| format!("pack {}: {} bytes at {}: {e}", r.file, r.size, r.offset))?;
    }
    Ok(Timed {
        reads: planned.len(),
        bytes,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// A file read past the cache needs sector-aligned offsets, sizes and buffers.
const SECTOR: u64 = 4096;

/// A buffer whose usable part starts on a sector.
#[derive(Default)]
struct Aligned(Vec<u8>);

impl Aligned {
    fn get(&mut self, len: usize) -> &mut [u8] {
        let want = len + SECTOR as usize;
        if self.0.len() < want {
            self.0.resize(want, 0);
        }
        let skew = self.0.as_ptr().align_offset(SECTOR as usize);
        &mut self.0[skew..skew + len]
    }
}

/// Reads the sectors that cover `size` bytes at `offset`; returns the bytes read
/// (fewer at the end of the file).
fn read_aligned(file: &File, buf: &mut Aligned, offset: u64, size: u64) -> std::io::Result<u64> {
    let from = offset / SECTOR * SECTOR;
    let to = (offset + size).div_ceil(SECTOR) * SECTOR;
    let buf = buf.get((to - from) as usize);
    let mut done = 0;
    while done < buf.len() {
        let n = read_at(file, &mut buf[done..], from + done as u64)?;
        done += n;
        // Short of a whole sector, the file has ended: the next offset would not be
        // aligned, which a read past the cache refuses.
        if n == 0 || n % SECTOR as usize != 0 {
            break;
        }
    }
    Ok(done as u64)
}

#[cfg(windows)]
fn open_uncached(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_NO_BUFFERING)
        .open(path)
}

#[cfg(not(windows))]
fn open_uncached(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(not(windows))]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(file: u8, offset: u32, size: u32) -> Read {
        Read { file, offset, size }
    }

    #[test]
    fn parses_a_trace_and_rejects_bad_lines() {
        assert_eq!(
            parse("3 4096 10\n\n0 0 1\n"),
            Ok(vec![r(3, 4096, 10), r(0, 0, 1)])
        );
        assert_eq!(
            parse("1 2\n"),
            Err("line 1: not FILE OFFSET SIZE: 1 2".into())
        );
        assert!(parse("256 0 1\n").is_err());
        assert!(parse("0 5000000000 1\n").is_err());
        assert!(parse("1 2 3 4\n").is_err());
    }

    #[test]
    fn without_a_gap_the_trace_is_read_as_is() {
        let reads = [r(1, 500, 10), r(0, 0, 10), r(1, 0, 10)];
        assert_eq!(plan(&reads, None), reads.to_vec());
    }

    #[test]
    fn merges_neighbours_within_the_gap_in_each_file() {
        let reads = [
            r(1, 100, 10), // 100..110
            r(0, 0, 10),   // 0..10
            r(1, 0, 50),   // 0..50, 50 short of the read at 100
            r(1, 105, 2),  // inside 100..110
            r(1, 300, 5),  // 190 past 110
            r(2, 10, 5),
        ];
        assert_eq!(
            plan(&reads, Some(50)),
            vec![r(0, 0, 10), r(1, 0, 110), r(1, 300, 5), r(2, 10, 5)]
        );
        // With no gap, only reads that touch or overlap merge.
        assert_eq!(
            plan(&[r(0, 0, 10), r(0, 10, 5), r(0, 16, 1)], Some(0)),
            vec![r(0, 0, 15), r(0, 16, 1)]
        );
    }

    #[test]
    fn aligned_reads_cover_whole_sectors() {
        let path = std::env::temp_dir().join(format!("nfsh-replay-{}", std::process::id()));
        std::fs::write(&path, vec![1u8; 10_000]).unwrap();
        let file = File::open(&path).unwrap();
        let mut buf = Aligned::default();
        // Bytes 4000..4200 span sectors 0 and 1: 8,192 bytes.
        assert_eq!(read_aligned(&file, &mut buf, 4000, 200).unwrap(), 8192);
        // The last sector is short: the file ends at 10,000.
        assert_eq!(
            read_aligned(&file, &mut buf, 9000, 100).unwrap(),
            10_000 - 8192
        );
        std::fs::remove_file(&path).unwrap();
    }
}
