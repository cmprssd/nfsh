//! `nfsh verify`: decompresses every entry the install's current bundles and chunk
//! tables point at, or every entry of an nfsh store, and checks each against the
//! sizes the tables give.
//!
//! In an install, every entry a bundle lists must first match the SHA-1 it lists for
//! its stored bytes (a chunk only in a `.toc` has none); a store's frames carry their
//! own checksums instead. An ebx or res entry must decompress to exactly its listed size. A chunk listed in a
//! bundle stores the chunk's blocks that cover its logical range (see
//! [`bundle_chunk_size`]); a chunk only in a `.toc` has no size in the tables and must
//! just decompress. Entries shared by several bundles are read once, in archive
//! order. In a store, a bundle's copy of a chunk's end may be read from the whole chunk,
//! so it only has to be at least that long; the store's parts are then checked too. An
//! install ([`frostbite::copy`]) is checked as its store is, but for the entries it
//! leaves out some or all of: what it keeps of them must decompress, and they are
//! counted apart.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use frostbite::cas::{self, Archives};
use frostbite::install::{Install, Superbundle};
use frostbite::oodle::Oodle;
use frostbite::sb::bundle_chunk_size;
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
    /// The SHA-1s bundles list for its stored bytes, each once.
    sha1s: Vec<[u8; 20]>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    pub entries: u64,
    pub stored: u64,
    pub decompressed: u64,
    /// The entries whose stored bytes matched their listed SHA-1s.
    pub hashed: u64,
    /// In an install, the entries it leaves out some or all of: what it keeps of them
    /// is decompressed but can't be held to the listed size.
    pub cut: u64,
    pub failed: u64,
}

#[derive(Debug, Default)]
pub struct Report {
    pub kinds: BTreeMap<Kind, Counts>,
    pub failures: Vec<String>,
    /// For a store, whether its parts find what they refer to.
    pub parts: Option<store::PartsCheck>,
}

impl Report {
    pub fn failed(&self) -> u64 {
        let parts = self.parts.as_ref().map_or(0, |p| p.failures.len() as u64);
        self.kinds.values().map(|c| c.failed).sum::<u64>() + parts
    }

    fn fail(&mut self, kind: Kind, message: String) {
        self.kinds.entry(kind).or_default().failed += 1;
        if self.failures.len() < LISTED_FAILURES {
            self.failures.push(message);
        }
    }

    pub fn markdown(&self) -> String {
        let mut out = String::from(
            "| entries | count | stored | decompressed | SHA-1 checked | left out | failed |\n\
             |---------|------:|-------:|-------------:|--------------:|---------:|-------:|\n",
        );
        for (kind, c) in &self.kinds {
            let _ = writeln!(
                out,
                "| {kind:?} | {} | {} | {} | {} | {} | {} |",
                c.entries, c.stored, c.decompressed, c.hashed, c.cut, c.failed
            );
        }
        if !self.failures.is_empty() {
            let _ = writeln!(out, "\nFailures (first {LISTED_FAILURES}):\n");
            for f in &self.failures {
                let _ = writeln!(out, "- {f}");
            }
        }
        if let Some(parts) = &self.parts {
            out.push('\n');
            out.push_str(&parts.markdown());
        }
        out
    }
}

/// Lists the entry at `l`, with the SHA-1 a bundle gives it (none from a `.toc`).
fn add(
    entries: &mut BTreeMap<Location, Entry>,
    l: Location,
    kind: Kind,
    name: &str,
    e: Expect,
    sha1: Option<[u8; 20]>,
) {
    let entry = entries.entry(l).or_insert_with(|| Entry {
        kind,
        name: name.to_owned(),
        expect: Some(e),
        sha1s: Vec::new(),
    });
    entry.expect = entry.expect.and_then(|x| x.merge(e));
    if let Some(sha1) = sha1.filter(|s| !entry.sha1s.contains(s)) {
        entry.sha1s.push(sha1);
    }
}

/// Every distinct stored entry the superbundles list, with what it must decompress to.
fn entries(superbundles: &[Superbundle], store: bool) -> BTreeMap<Location, Entry> {
    let mut out = BTreeMap::new();
    for sb in superbundles {
        for b in sb.bundles.iter().map(|b| &b.bundle) {
            for a in &b.ebx {
                let e = Expect::Exactly(a.original_size.into());
                add(&mut out, a.location, Kind::Ebx, &a.name, e, Some(a.sha1));
            }
            for r in &b.res {
                let a = &r.asset;
                let e = Expect::Exactly(a.original_size.into());
                add(&mut out, a.location, Kind::Res, &a.name, e, Some(a.sha1));
            }
            for c in &b.chunks {
                let size = bundle_chunk_size(c.logical_offset, c.logical_size);
                let e = match store && c.logical_offset > 0 {
                    true => Expect::AtLeast(size),
                    false => Expect::Exactly(size),
                };
                add(
                    &mut out,
                    c.location,
                    Kind::Chunk,
                    &hex(&c.id),
                    e,
                    Some(c.sha1),
                );
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
                None,
            );
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks the superbundles' entries in archive order. `read` decompresses one, given
/// its listed SHA-1s, and says whether an install left out any of it and whether it
/// checked them.
fn check(
    superbundles: &[Superbundle],
    store: bool,
    mut read: impl FnMut(Location, &[[u8; 20]]) -> Result<Read>,
) -> Report {
    let mut report = Report::default();
    for (location, entry) in entries(superbundles, store) {
        let c = report.kinds.entry(entry.kind).or_default();
        c.entries += 1;
        c.stored += u64::from(location.size);
        let Some(expect) = entry.expect else {
            let message = format!("{}: bundles list different sizes", entry.name);
            report.fail(entry.kind, message);
            continue;
        };
        let result = read(location, &entry.sha1s).map_err(|e| e.0);
        let result = result.and_then(|Read { data, cut, hashed }| {
            let c = report.kinds.entry(entry.kind).or_default();
            c.decompressed += data.len() as u64;
            c.hashed += u64::from(hashed);
            c.cut += u64::from(cut);
            match cut {
                true => Ok(()),
                false => expect.check(data.len() as u64),
            }
        });
        if let Err(e) = result {
            report.fail(entry.kind, format!("{:?} {}: {e}", entry.kind, entry.name));
        }
    }
    report
}

/// One entry as [`check`]'s reader gives it.
struct Read {
    data: Vec<u8>,
    /// An install left out some or all of it.
    cut: bool,
    /// Its stored bytes matched its listed SHA-1s.
    hashed: bool,
}

/// An install's entry at `l`, checked against its listed `sha1s` and decompressed.
fn read_install(
    archives: &mut Archives,
    oodle: &Oodle,
    l: Location,
    sha1s: &[[u8; 20]],
) -> Result<Read> {
    let stored = archives.stored(l)?;
    cas::check_sha1(&stored, sha1s)?;
    let data = cas::decompress(&stored, oodle)?;
    let hashed = !sha1s.is_empty();
    Ok(Read {
        data,
        cut: false,
        hashed,
    })
}

/// Verifies the store in `path` if it holds one, otherwise the install there.
pub fn run(path: &Path) -> Result<Report> {
    if path.join(store::INDEX).is_file() {
        let store = Store::open(path)?;
        let mut entries = store.entries()?;
        let mut report = check(&store.superbundles, true, |l, _| {
            let (data, cut) = entries.read_end(l)?;
            Ok(Read {
                data,
                cut,
                hashed: false,
            })
        });
        report.parts = Some(store::check_parts(&store)?);
        return Ok(report);
    }
    let install = Install::open(path)?;
    let oodle = Oodle::load(path)?;
    let mut superbundles = Vec::new();
    for name in &install.superbundles {
        superbundles.extend(install.superbundle(name)?);
    }
    let mut archives = Archives::new(&install);
    Ok(check(&superbundles, false, |l, sha1s| {
        read_install(&mut archives, &oodle, l, sha1s)
    }))
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
