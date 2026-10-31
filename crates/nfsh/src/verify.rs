//! `nfsh verify`: decompresses every entry the install's current bundles and chunk
//! tables point at, or every entry of an nfsh store, and checks each against the
//! sizes the tables give.
//!
//! An ebx or res entry must decompress to exactly its listed size. A chunk listed in a
//! bundle stores the chunk's blocks that cover its logical range (see
//! [`bundle_chunk_size`]); a chunk only in a `.toc` has no size in the tables and must
//! just decompress. Entries shared by several bundles are read once, in archive
//! order.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use frostbite::cas::{Archives, MAX_BLOCK};
use frostbite::install::{Install, Superbundle};
use frostbite::oodle::Oodle;
use frostbite::store::{self, Store};
use frostbite::{Location, Result};

/// At most this many failures are listed; all are counted.
const LISTED_FAILURES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Ebx,
    Res,
    Chunk,
}

/// What an entry's decompressed size must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Exactly(u64),
    AtLeast(u64),
}

impl Expect {
    /// Combines what two bundles say about the same stored bytes; `None` when they
    /// name different exact sizes.
    pub fn merge(self, other: Expect) -> Option<Expect> {
        use Expect::*;
        match (self, other) {
            (Exactly(a), Exactly(b)) => (a == b).then_some(Exactly(a)),
            (Exactly(a), AtLeast(b)) | (AtLeast(b), Exactly(a)) => (a >= b).then_some(Exactly(a)),
            (AtLeast(a), AtLeast(b)) => Some(AtLeast(a.max(b))),
        }
    }

    pub fn check(self, len: u64) -> std::result::Result<(), String> {
        match self {
            Expect::Exactly(n) if len != n => Err(format!("{len} bytes, listed as {n}")),
            Expect::AtLeast(n) if len < n => Err(format!("{len} bytes, bundles use {n}")),
            _ => Ok(()),
        }
    }
}

struct Entry {
    kind: Kind,
    name: String,
    /// `None` once two bundles disagree about the size.
    expect: Option<Expect>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    pub entries: u64,
    pub stored: u64,
    pub decompressed: u64,
    pub failed: u64,
}

#[derive(Debug, Default)]
pub struct Report {
    pub kinds: BTreeMap<Kind, Counts>,
    pub failures: Vec<String>,
}

impl Report {
    pub fn failed(&self) -> u64 {
        self.kinds.values().map(|c| c.failed).sum()
    }

    fn fail(&mut self, kind: Kind, message: String) {
        self.kinds.entry(kind).or_default().failed += 1;
        if self.failures.len() < LISTED_FAILURES {
            self.failures.push(message);
        }
    }

    pub fn markdown(&self) -> String {
        let mut out = String::from(
            "| entries | count | stored | decompressed | failed |\n\
             |---------|------:|-------:|-------------:|-------:|\n",
        );
        for (kind, c) in &self.kinds {
            let _ = writeln!(
                out,
                "| {kind:?} | {} | {} | {} | {} |",
                c.entries, c.stored, c.decompressed, c.failed
            );
        }
        if !self.failures.is_empty() {
            let _ = writeln!(out, "\nFailures (first {LISTED_FAILURES}):\n");
            for f in &self.failures {
                let _ = writeln!(out, "- {f}");
            }
        }
        out
    }
}

fn add(entries: &mut BTreeMap<Location, Entry>, l: Location, kind: Kind, name: &str, e: Expect) {
    let entry = entries.entry(l).or_insert_with(|| Entry {
        kind,
        name: name.to_owned(),
        expect: Some(e),
    });
    entry.expect = entry.expect.and_then(|x| x.merge(e));
}

/// Every distinct stored entry the superbundles list, with what it must decompress to.
fn entries(superbundles: &[Superbundle]) -> BTreeMap<Location, Entry> {
    let mut out = BTreeMap::new();
    for sb in superbundles {
        for b in sb.bundles.iter().map(|b| &b.bundle) {
            for a in &b.ebx {
                let e = Expect::Exactly(a.original_size.into());
                add(&mut out, a.location, Kind::Ebx, &a.name, e);
            }
            for r in &b.res {
                let a = &r.asset;
                let e = Expect::Exactly(a.original_size.into());
                add(&mut out, a.location, Kind::Res, &a.name, e);
            }
            for c in &b.chunks {
                let e = Expect::Exactly(bundle_chunk_size(c.logical_offset, c.logical_size));
                add(&mut out, c.location, Kind::Chunk, &hex(&c.id), e);
            }
        }
        for c in &sb.chunks {
            let id = hex(&c.chunk.id);
            add(
                &mut out,
                c.chunk.location,
                Kind::Chunk,
                &id,
                Expect::AtLeast(0),
            );
        }
    }
    out
}

/// A bundle's copy of a chunk holds the chunk's whole blocks from the one containing
/// the logical offset to the end of the logical range (blocks are [`MAX_BLOCK`]
/// decompressed), so its size is that range widened back to a block boundary.
pub fn bundle_chunk_size(logical_offset: u32, logical_size: u32) -> u64 {
    let block = MAX_BLOCK as u64;
    let offset = u64::from(logical_offset);
    offset % block + u64::from(logical_size)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks the superbundles' entries, decompressed by `read`, in archive order.
fn check(
    superbundles: &[Superbundle],
    mut read: impl FnMut(Location) -> Result<Vec<u8>>,
) -> Report {
    let mut report = Report::default();
    for (location, entry) in entries(superbundles) {
        let c = report.kinds.entry(entry.kind).or_default();
        c.entries += 1;
        c.stored += u64::from(location.size);
        let Some(expect) = entry.expect else {
            let message = format!("{}: bundles list different sizes", entry.name);
            report.fail(entry.kind, message);
            continue;
        };
        let result = read(location).map_err(|e| e.0);
        let result = result.and_then(|data| {
            report.kinds.entry(entry.kind).or_default().decompressed += data.len() as u64;
            expect.check(data.len() as u64)
        });
        if let Err(e) = result {
            report.fail(entry.kind, format!("{:?} {}: {e}", entry.kind, entry.name));
        }
    }
    report
}

/// Verifies the store in `path` if it holds one, otherwise the install there.
pub fn run(path: &Path) -> Result<Report> {
    if path.join(store::INDEX).is_file() {
        let store = Store::open(path)?;
        let mut entries = store.entries()?;
        return Ok(check(&store.superbundles, |l| entries.read(l)));
    }
    let install = Install::open(path)?;
    let oodle = Oodle::load(path)?;
    let mut superbundles = Vec::new();
    for name in &install.superbundles {
        superbundles.extend(install.superbundle(name)?);
    }
    let mut archives = Archives::new(&install);
    Ok(check(&superbundles, |l| archives.read(l, &oodle)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_sizes_win_over_lower_bounds_and_must_agree() {
        use Expect::*;
        assert_eq!(Exactly(5).merge(AtLeast(3)), Some(Exactly(5)));
        assert_eq!(AtLeast(3).merge(Exactly(5)), Some(Exactly(5)));
        assert_eq!(AtLeast(6).merge(Exactly(5)), None);
        assert_eq!(Exactly(5).merge(Exactly(6)), None);
        assert_eq!(AtLeast(2).merge(AtLeast(9)), Some(AtLeast(9)));
    }

    #[test]
    fn bundle_chunks_start_at_the_block_holding_the_logical_offset() {
        // A 2048x2048 texture's mip chain ends at 5592432; the bundle names its last
        // 1392 bytes and stores the 21872 from the 64 KiB boundary before them.
        assert_eq!(bundle_chunk_size(5_591_040, 1392), 21_872);
        assert_eq!(bundle_chunk_size(0, 1392), 1392);
        assert_eq!(bundle_chunk_size(0x20000, 5), 5);
    }

    #[test]
    fn checks_report_the_mismatch() {
        assert_eq!(Expect::Exactly(4).check(4), Ok(()));
        assert_eq!(
            Expect::Exactly(4).check(3),
            Err("3 bytes, listed as 4".into())
        );
        assert_eq!(Expect::AtLeast(4).check(9), Ok(()));
        assert_eq!(
            Expect::AtLeast(4).check(3),
            Err("3 bytes, bundles use 4".into())
        );
    }

    #[test]
    fn failures_are_counted_but_only_the_first_are_listed() {
        let mut report = Report::default();
        for i in 0..25 {
            report.fail(Kind::Res, format!("res {i}"));
        }
        assert_eq!(report.failed(), 25);
        assert_eq!(report.failures.len(), LISTED_FAILURES);
        assert_eq!(report.failures[0], "res 0");
    }
}
