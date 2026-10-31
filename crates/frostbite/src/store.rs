//! nfsh's own copy of an install's data, a *store*: the stored entries the patched
//! tables still point at, copied into a few packs (`pack_NN.nfsh`), and one index
//! (`index.nfsh`) holding the superbundles with their bundles and chunks. The dead
//! `Data/` bytes the patch replaced are left behind. Entries keep Heat's stored blocks
//! as they are, so [`crate::cas::decompress`] reads them unchanged.
//!
//! In a store, the archive a [`Location`] names is a pack: [`CasRef::cas`] is the pack
//! number, with `patch` false and `package` 0. The index is our own format (see
//! `docs/FORMATS.md`), big-endian like Heat's tables.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::cas::Archives;
use crate::install::{BundleEntry, ChunkEntry, Install, Layer, Superbundle};
use crate::reader::Reader;
use crate::sb::{Asset, Bundle, Chunk, Res};
use crate::toc::TocChunk;
use crate::{CasRef, Location, Result, bail};

pub const INDEX: &str = "index.nfsh";
const MAGIC: &[u8; 8] = b"NFSHSTOR";
const VERSION: u32 = 1;

/// A pack is closed once the next entry would take it past this size, so packs stay
/// easy to copy and their offsets fit the 32-bit [`Location::offset`].
pub const PACK_LIMIT: u64 = 1 << 30;

pub fn pack_name(n: u8) -> String {
    format!("pack_{n:02}.nfsh")
}

fn pack_ref(n: u8) -> CasRef {
    CasRef {
        patch: false,
        package: 0,
        cas: n,
    }
}

fn io_error(path: &Path, e: std::io::Error) -> crate::Error {
    crate::Error(format!("{}: {e}", path.display()))
}

#[derive(Debug)]
pub struct Store {
    pub dir: PathBuf,
    pub superbundles: Vec<Superbundle>,
    /// Each pack's size in bytes.
    pub packs: Vec<u64>,
}

impl Store {
    /// Opens the store in `dir`, checking that every pack the index names is there and
    /// whole.
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join(INDEX);
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
        let (superbundles, packs) =
            decode(&bytes).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
        for (n, &size) in packs.iter().enumerate() {
            let path = dir.join(pack_name(n as u8));
            let len = std::fs::metadata(&path)
                .map_err(|e| io_error(&path, e))?
                .len();
            if len != size {
                bail!("{}: {len} bytes, the index says {size}", path.display());
            }
        }
        Ok(Self {
            dir: dir.to_owned(),
            superbundles,
            packs,
        })
    }

    pub fn pack_path(&self, cas: CasRef) -> Option<PathBuf> {
        let known = !cas.patch && cas.package == 0 && usize::from(cas.cas) < self.packs.len();
        known.then(|| self.dir.join(pack_name(cas.cas)))
    }

    pub fn archives(&self) -> Archives<'_> {
        Archives::with_paths(|cas| self.pack_path(cas))
    }
}

/// Calls `f` on every location a superbundle lists.
pub fn each_location(sb: &mut Superbundle, f: &mut impl FnMut(&mut Location)) {
    for b in &mut sb.bundles {
        let Bundle {
            ebx, res, chunks, ..
        } = &mut b.bundle;
        ebx.iter_mut().for_each(|a| f(&mut a.location));
        res.iter_mut().for_each(|r| f(&mut r.asset.location));
        chunks.iter_mut().for_each(|c| f(&mut c.location));
    }
    sb.chunks.iter_mut().for_each(|c| f(&mut c.chunk.location));
}

/// Places each distinct stored entry in the packs, in the order given (archive order,
/// so the copy reads each archive front to back). A pack is closed when the next entry
/// would take a non-empty pack past `limit`.
pub fn plan(entries: &BTreeSet<Location>, limit: u64) -> Result<BTreeMap<Location, Location>> {
    let mut out = BTreeMap::new();
    let (mut pack, mut pos) = (0u8, 0u64);
    for &source in entries {
        let size = u64::from(source.size);
        if pos > 0 && pos + size > limit {
            let Some(next) = pack.checked_add(1) else {
                bail!("the entries need more than 256 packs");
            };
            (pack, pos) = (next, 0);
        }
        let target = Location {
            cas: pack_ref(pack),
            offset: pos as u32,
            size: source.size,
        };
        out.insert(source, target);
        pos += size;
    }
    Ok(out)
}

/// What an import wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub superbundles: usize,
    pub bundles: usize,
    pub entries: usize,
    pub packs: Vec<u64>,
    pub index: u64,
}

/// Copies the superbundles `names` of `install` into a store in `dir`, then reopens
/// the store and checks that it lists the same superbundles and holds the same stored
/// bytes, entry for entry. `dir` may be missing, empty, or an earlier store, which is
/// replaced; anything else in it is refused. The index is written last, so an import
/// that stops early leaves no index behind.
pub fn import(install: &Install, names: &[String], dir: &Path) -> Result<Imported> {
    clear(dir)?;
    let mut superbundles = Vec::new();
    for name in names {
        superbundles.extend(install.superbundle(name)?);
    }
    let mut entries = BTreeSet::new();
    for sb in &mut superbundles {
        each_location(sb, &mut |l| {
            entries.insert(*l);
        });
    }
    let plan = plan(&entries, PACK_LIMIT)?;

    let mut packs: Vec<u64> = Vec::new();
    let mut out: Option<(PathBuf, BufWriter<File>)> = None;
    let mut source = Archives::new(install);
    for (&from, to) in &plan {
        if usize::from(to.cas.cas) == packs.len() {
            if let Some((path, file)) = out.take() {
                finish(&path, file)?;
            }
            let path = dir.join(pack_name(to.cas.cas));
            let file = File::create(&path).map_err(|e| io_error(&path, e))?;
            out = Some((path, BufWriter::with_capacity(1 << 20, file)));
            packs.push(0);
        }
        let (Some((path, file)), Some(len)) = (&mut out, packs.last_mut()) else {
            bail!("{from:?} is placed before the first pack");
        };
        let bytes = source.stored(from)?;
        file.write_all(&bytes).map_err(|e| io_error(path, e))?;
        *len += bytes.len() as u64;
    }
    if let Some((path, file)) = out {
        finish(&path, file)?;
    }

    for sb in &mut superbundles {
        each_location(sb, &mut |l| *l = plan[l]);
    }
    let index = encode(&superbundles, &packs);
    let tmp = dir.join(format!("{INDEX}.tmp"));
    std::fs::write(&tmp, &index).map_err(|e| io_error(&tmp, e))?;
    let path = dir.join(INDEX);
    std::fs::rename(&tmp, &path).map_err(|e| io_error(&path, e))?;

    let store = Store::open(dir)?;
    if store.superbundles != superbundles {
        bail!("{}: the index reads back differently", path.display());
    }
    let mut copy = store.archives();
    for (&from, &to) in &plan {
        if source.stored(from)? != copy.stored(to)? {
            bail!("{from:?}: the copy at {to:?} differs");
        }
    }
    Ok(Imported {
        superbundles: superbundles.len(),
        bundles: superbundles.iter().map(|s| s.bundles.len()).sum(),
        entries: plan.len(),
        packs,
        index: index.len() as u64,
    })
}

fn finish(path: &Path, file: BufWriter<File>) -> Result<()> {
    let file = file
        .into_inner()
        .map_err(|e| io_error(path, e.into_error()))?;
    file.sync_all().map_err(|e| io_error(path, e))
}

/// Makes `dir` an empty folder, removing an earlier store's files; refuses if it holds
/// anything else.
fn clear(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    let mut ours = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| io_error(dir, e))? {
        let path = entry.map_err(|e| io_error(dir, e))?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let is_pack = name.starts_with("pack_") && name.ends_with(".nfsh");
        if !(is_pack || name == INDEX || name == format!("{INDEX}.tmp")) || !path.is_file() {
            bail!(
                "{}: not an nfsh store (holds {name}); refusing to write into it",
                dir.display()
            );
        }
        ours.push(path);
    }
    // The index goes first, so a store is never left with an index and missing packs.
    ours.sort_by_key(|p| !p.ends_with(INDEX));
    for path in ours {
        std::fs::remove_file(&path).map_err(|e| io_error(&path, e))?;
    }
    Ok(())
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_be_bytes());
    }
    fn raw(&mut self, v: &[u8]) {
        self.0.extend(v);
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.raw(v);
    }
    fn count<T>(&mut self, items: &[T]) {
        self.u32(items.len() as u32);
    }
    fn layer(&mut self, layer: Layer) {
        self.u8(u8::from(layer == Layer::Patch));
    }
    fn location(&mut self, l: Location) {
        self.u8(l.cas.cas);
        self.u32(l.offset);
        self.u32(l.size);
    }
    fn asset(&mut self, a: &Asset) {
        self.bytes(a.name.as_bytes());
        self.raw(&a.sha1);
        self.u32(a.original_size);
        self.location(a.location);
    }
}

/// The index for `superbundles` (locations already in the packs) and the pack sizes.
pub fn encode(superbundles: &[Superbundle], packs: &[u64]) -> Vec<u8> {
    let mut w = Writer(MAGIC.to_vec());
    w.u32(VERSION);
    w.count(packs);
    packs.iter().for_each(|&p| w.u64(p));
    w.count(superbundles);
    for sb in superbundles {
        w.bytes(sb.name.as_bytes());
        w.count(&sb.bundles);
        for b in &sb.bundles {
            w.bytes(b.name.as_bytes());
            w.layer(b.layer);
            w.bytes(&b.bundle.meta);
            w.count(&b.bundle.ebx);
            b.bundle.ebx.iter().for_each(|a| w.asset(a));
            w.count(&b.bundle.res);
            for r in &b.bundle.res {
                w.asset(&r.asset);
                w.u32(r.res_type);
                w.raw(&r.meta);
                w.u64(r.id);
            }
            w.count(&b.bundle.chunks);
            for c in &b.bundle.chunks {
                w.raw(&c.id);
                w.raw(&c.sha1);
                w.u32(c.logical_offset);
                w.u32(c.logical_size);
                w.location(c.location);
            }
        }
        w.count(&sb.chunks);
        for c in &sb.chunks {
            w.raw(&c.chunk.id);
            w.layer(c.layer);
            w.location(c.chunk.location);
        }
    }
    w.0
}

struct IndexReader<'a> {
    r: Reader<'a>,
    packs: Vec<u64>,
}

impl IndexReader<'_> {
    /// Reads `count`, then that many items. Nothing is reserved up front, so a damaged
    /// count fails at the end of the data rather than in the allocator.
    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        let count = self.r.u32_be()?;
        (0..count).map(|_| item(self)).collect()
    }
    fn string(&mut self) -> Result<String> {
        let len = self.r.u32_be()? as usize;
        match std::str::from_utf8(self.r.bytes(len)?) {
            Ok(s) => Ok(s.to_owned()),
            Err(_) => bail!("a name is not UTF-8"),
        }
    }
    fn layer(&mut self) -> Result<Layer> {
        match self.r.u8()? {
            0 => Ok(Layer::Data),
            1 => Ok(Layer::Patch),
            other => bail!("unknown layer {other}"),
        }
    }
    /// A location, which must lie inside its pack.
    fn location(&mut self) -> Result<Location> {
        let pack = self.r.u8()?;
        let (offset, size) = (self.r.u32_be()?, self.r.u32_be()?);
        let end = u64::from(offset) + u64::from(size);
        match self.packs.get(usize::from(pack)) {
            Some(&len) if end <= len => Ok(Location {
                cas: pack_ref(pack),
                offset,
                size,
            }),
            _ => bail!("{size} bytes at {offset:#x} in pack {pack} are outside the packs"),
        }
    }
    fn asset(&mut self) -> Result<Asset> {
        Ok(Asset {
            name: self.string()?,
            sha1: self.r.array()?,
            original_size: self.r.u32_be()?,
            location: self.location()?,
        })
    }
    fn bundle(&mut self) -> Result<BundleEntry> {
        let name = self.string()?;
        let layer = self.layer()?;
        let meta_len = self.r.u32_be()? as usize;
        let meta = self.r.bytes(meta_len)?.to_vec();
        let ebx = self.list(Self::asset)?;
        let res = self.list(|i| {
            Ok(Res {
                asset: i.asset()?,
                res_type: i.r.u32_be()?,
                meta: i.r.array()?,
                id: i.r.u64_be()?,
            })
        })?;
        let chunks = self.list(|i| {
            Ok(Chunk {
                id: i.r.array()?,
                sha1: i.r.array()?,
                logical_offset: i.r.u32_be()?,
                logical_size: i.r.u32_be()?,
                location: i.location()?,
            })
        })?;
        Ok(BundleEntry {
            name,
            layer,
            bundle: Bundle {
                ebx,
                res,
                chunks,
                meta,
            },
        })
    }
    fn superbundle(&mut self) -> Result<Superbundle> {
        let name = self.string()?;
        let bundles = self.list(Self::bundle)?;
        let chunks = self.list(|i| {
            let id = i.r.array()?;
            let layer = i.layer()?;
            let location = i.location()?;
            Ok(ChunkEntry {
                chunk: TocChunk { id, location },
                layer,
            })
        })?;
        Ok(Superbundle {
            name,
            bundles,
            chunks,
        })
    }
}

/// Reads an index back: the superbundles and the pack sizes.
pub fn decode(bytes: &[u8]) -> Result<(Vec<Superbundle>, Vec<u64>)> {
    let mut r = Reader::new(bytes);
    if r.bytes(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
        bail!("not an nfsh store index");
    }
    let version = r.u32_be()?;
    if version != VERSION {
        bail!("store version {version}; this nfsh reads version {VERSION}");
    }
    let mut i = IndexReader {
        r,
        packs: Vec::new(),
    };
    i.packs = i.list(|i| i.r.u64_be())?;
    if i.packs.len() > 256 {
        bail!("{} packs; at most 256", i.packs.len());
    }
    let superbundles = i.list(IndexReader::superbundle)?;
    if i.r.pos() != bytes.len() {
        bail!("{} bytes after the index", bytes.len() - i.r.pos());
    }
    Ok((superbundles, i.packs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(cas: u8, offset: u32, size: u32) -> Location {
        Location {
            cas: pack_ref(cas),
            offset,
            size,
        }
    }

    fn sample() -> Vec<Superbundle> {
        let asset = |name: &str, location| Asset {
            name: name.into(),
            sha1: [3; 20],
            original_size: 900,
            location,
        };
        let bundle = Bundle {
            ebx: vec![asset("cars/a", at(0, 0, 10))],
            res: vec![Res {
                asset: asset("cars/a/mesh", at(0, 10, 20)),
                res_type: 0x49b1_56d4,
                meta: [9; 16],
                id: 77,
            }],
            chunks: vec![Chunk {
                id: [4; 16],
                sha1: [5; 20],
                logical_offset: 0x10000,
                logical_size: 12,
                location: at(1, 0, 30),
            }],
            meta: vec![0x82, 0],
        };
        vec![Superbundle {
            name: "Win32/globals".into(),
            bundles: vec![BundleEntry {
                name: "win32/cars".into(),
                layer: Layer::Patch,
                bundle,
            }],
            chunks: vec![ChunkEntry {
                chunk: TocChunk {
                    id: [6; 16],
                    location: at(1, 30, 5),
                },
                layer: Layer::Data,
            }],
        }]
    }

    #[test]
    fn the_index_reads_back_as_written() {
        let packs = vec![30, 35];
        let bytes = encode(&sample(), &packs);
        assert_eq!(decode(&bytes), Ok((sample(), packs)));
    }

    #[test]
    fn damaged_indexes_are_refused() {
        let bytes = encode(&sample(), &[30, 35]);
        for len in [0, 8, 20, bytes.len() - 1] {
            assert!(decode(&bytes[..len]).is_err(), "truncated to {len}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_err());
        // The second pack is too short for the toc chunk at 30..35.
        assert!(decode(&encode(&sample(), &[30, 34])).is_err());
        let mut version = bytes.clone();
        version[11] = 2;
        assert!(decode(&version).unwrap_err().0.contains("version 2"));
    }

    #[test]
    fn packs_close_before_the_limit_and_keep_entry_order() {
        let src = |offset, size| Location {
            cas: CasRef {
                patch: true,
                package: 1,
                cas: 3,
            },
            offset,
            size,
        };
        let entries = BTreeSet::from([src(0, 60), src(100, 40), src(200, 10), src(300, 150)]);
        let placed = plan(&entries, 100).unwrap();
        assert_eq!(
            placed.values().copied().collect::<Vec<_>>(),
            // 60 + 40 fills the first pack exactly; an entry larger than the limit
            // gets a pack of its own size.
            [at(0, 0, 60), at(0, 60, 40), at(1, 0, 10), at(2, 0, 150)]
        );
        assert!(plan(&BTreeSet::new(), 100).unwrap().is_empty());
    }

    #[test]
    fn each_location_visits_bundle_and_toc_entries() {
        let mut sb = sample().remove(0);
        let mut seen = Vec::new();
        each_location(&mut sb, &mut |l| seen.push(l.offset));
        assert_eq!(seen, [0, 10, 0, 30]);
    }

    #[test]
    fn imports_only_into_an_empty_folder_or_a_store() {
        let dir = std::env::temp_dir().join(format!("nfsh-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(INDEX), b"old").unwrap();
        std::fs::write(dir.join(pack_name(0)), b"old").unwrap();
        clear(&dir).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
        assert!(clear(&dir).is_err());
        assert!(dir.join("notes.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Imports one superbundle of the owner's install when `NFSH_INSTALL` points at it
    /// (local-ci sets it on the Windows side); `nfsh import` does them all.
    #[test]
    fn imports_a_real_superbundle() {
        let Some(root) = std::env::var_os("NFSH_INSTALL") else {
            eprintln!("NFSH_INSTALL not set; skipped");
            return;
        };
        let install = Install::open(Path::new(&root)).unwrap();
        let dir = std::env::temp_dir().join(format!("nfsh-import-test-{}", std::process::id()));
        let names = ["Win32/globals".to_owned()];
        let imported = import(&install, &names, &dir).unwrap();
        let store = Store::open(&dir).unwrap();
        assert_eq!(store.superbundles.len(), 1);
        assert_eq!(store.packs, imported.packs);
        assert!(imported.entries > 100, "only {} entries", imported.entries);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
