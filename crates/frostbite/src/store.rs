//! nfsh's own copy of an install's data, a *store*: the entries the patched tables
//! still point at, recompressed into a few packs (`pack_NN.nfsh`), and one index
//! (`index.nfsh`) holding the superbundles with their bundles and chunks. The dead
//! `Data/` bytes the patch replaced are left behind. Each entry is one zstd frame (see
//! [`crate::frames`]), so a store reads anywhere, without the install's Oodle library.
//!
//! In a store, the archive a [`Location`] names is a pack: [`CasRef::cas`] is the pack
//! number, with `patch` false and `package` 0. The index is our own format (see
//! `docs/FORMATS.md`), big-endian like Heat's tables. It also lists each ebx's
//! partition GUID, so a reference into another partition is found without reading
//! every ebx.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cas::{Archives, Decoder};
use crate::ebx::Partition;
use crate::frames::{self, FrameDecoder, FrameEncoder};
use crate::install::{BundleEntry, ChunkEntry, Install, Layer, Superbundle};
use crate::reader::Reader;
use crate::sb::{Asset, Bundle, Chunk, Res};
use crate::toc::TocChunk;
use crate::{CasRef, Location, Result, bail};

pub const INDEX: &str = "index.nfsh";
const MAGIC: &[u8; 8] = b"NFSHSTOR";
const VERSION: u32 = 3;
/// The version before the partition GUIDs, still read: such a store has none until
/// [`reindex`] adds them.
const VERSION_WITHOUT_GUIDS: u32 = 2;

/// A pack is closed once the next entry would take it past this size, so packs stay
/// easy to copy and their offsets fit the 32-bit [`Location::offset`].
pub const PACK_LIMIT: u64 = 1 << 30;

/// The import works through the entries in batches of about this many stored bytes,
/// holding one batch's frames in memory at a time.
const BATCH: u64 = 256 << 20;

/// The most ebx the dictionary is trained on, spread evenly over the install's.
const DICTIONARY_SAMPLES: usize = 8000;

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
    /// The ebx frames' dictionary; empty for none.
    pub dictionary: Vec<u8>,
    /// Every ebx's partition GUID with where it is listed, sorted by GUID; empty for
    /// a store not indexed yet.
    by_guid: Vec<([u8; 16], EbxRef)>,
}

/// Where an ebx is listed in a store: indices into [`Store::superbundles`], its
/// bundles, and their ebx.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EbxRef {
    pub superbundle: u32,
    pub bundle: u32,
    pub ebx: u32,
}

/// Every ebx of `superbundles` with where it is listed, in index order.
fn each_ebx(superbundles: &[Superbundle]) -> impl Iterator<Item = (EbxRef, &Asset)> {
    superbundles.iter().zip(0..).flat_map(|(sb, s)| {
        sb.bundles.iter().zip(0..).flat_map(move |(b, n)| {
            b.bundle.ebx.iter().zip(0..).map(move |(a, e)| {
                let at = EbxRef {
                    superbundle: s,
                    bundle: n,
                    ebx: e,
                };
                (at, a)
            })
        })
    })
}

impl Store {
    /// Opens the store in `dir`, checking that every pack the index names is there and
    /// whole.
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join(INDEX);
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
        let index = decode(&bytes).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
        for (n, &size) in index.packs.iter().enumerate() {
            let path = dir.join(pack_name(n as u8));
            let len = std::fs::metadata(&path)
                .map_err(|e| io_error(&path, e))?
                .len();
            if len != size {
                bail!("{}: {len} bytes, the index says {size}", path.display());
            }
        }
        let mut by_guid: Vec<([u8; 16], EbxRef)> = index
            .partitions
            .iter()
            .zip(each_ebx(&index.superbundles))
            .map(|(&guid, (at, _))| (guid, at))
            .collect();
        by_guid.sort_unstable();
        Ok(Self {
            dir: dir.to_owned(),
            superbundles: index.superbundles,
            packs: index.packs,
            dictionary: index.dictionary,
            by_guid,
        })
    }

    /// Whether the index lists the partition GUIDs (see [`reindex`]).
    pub fn has_guids(&self) -> bool {
        !self.by_guid.is_empty()
    }

    /// Every ebx holding the partition `guid`, in index order: a partition is often
    /// copied into several bundles. An error for a store without GUIDs.
    pub fn partitions(&self, guid: &[u8; 16]) -> Result<impl Iterator<Item = EbxRef> + '_> {
        let guid = *guid;
        if !self.has_guids() {
            bail!(
                "{} has no partition GUIDs; run `nfsh reindex` on it",
                self.dir.display()
            );
        }
        let start = self.by_guid.partition_point(|(g, _)| *g < guid);
        Ok(self.by_guid[start..]
            .iter()
            .take_while(move |(g, _)| *g == guid)
            .map(|&(_, at)| at))
    }

    /// The bundle and the ebx at `at`, if the store lists it.
    pub fn ebx_at(&self, at: EbxRef) -> Option<(&BundleEntry, &Asset)> {
        let sb = self.superbundles.get(at.superbundle as usize)?;
        let b = sb.bundles.get(at.bundle as usize)?;
        Some((b, b.bundle.ebx.get(at.ebx as usize)?))
    }

    pub fn pack_path(&self, cas: CasRef) -> Option<PathBuf> {
        let known = !cas.patch && cas.package == 0 && usize::from(cas.cas) < self.packs.len();
        known.then(|| self.dir.join(pack_name(cas.cas)))
    }

    /// The first ebx entry named `name` (ignoring case), in any bundle.
    pub fn ebx(&self, name: &str) -> Option<&Asset> {
        self.superbundles
            .iter()
            .flat_map(|sb| &sb.bundles)
            .flat_map(|b| &b.bundle.ebx)
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Reads the frames as stored.
    pub fn archives(&self) -> Archives<'_> {
        Archives::with_paths(|cas| self.pack_path(cas))
    }

    /// Reads the entries, decompressed.
    pub fn entries(&self) -> Result<Entries<'_>> {
        Ok(Entries {
            archives: self.archives(),
            frames: FrameDecoder::new(&self.dictionary)?,
        })
    }
}

/// A store's entries, read and decompressed; one per thread.
pub struct Entries<'a> {
    archives: Archives<'a>,
    frames: FrameDecoder,
}

impl Entries<'_> {
    /// The decompressed entry at `location`.
    pub fn read(&mut self, location: Location) -> Result<Vec<u8>> {
        let frame = self.archives.stored(location)?;
        self.frames
            .decode(&frame)
            .map_err(|e| crate::Error(format!("{location:?}: {e}")))
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

/// Places entries in the packs one after another, in the order given (archive order,
/// so the import reads each archive front to back). A pack is closed when the next
/// entry would take a non-empty pack past the limit.
pub struct Packer {
    limit: u64,
    pack: u8,
    pos: u64,
}

impl Packer {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            pack: 0,
            pos: 0,
        }
    }

    /// Where the next entry, `size` bytes, goes.
    pub fn place(&mut self, size: u32) -> Result<Location> {
        let len = u64::from(size);
        if self.pos > 0 && self.pos + len > self.limit {
            let Some(next) = self.pack.checked_add(1) else {
                bail!("the entries need more than 256 packs");
            };
            (self.pack, self.pos) = (next, 0);
        }
        let Ok(offset) = u32::try_from(self.pos) else {
            bail!("pack {} is past 4 GiB", self.pack);
        };
        self.pos += len;
        Ok(Location {
            cas: pack_ref(self.pack),
            offset,
            size,
        })
    }
}

/// Maps `f` over `items` on every core, each thread with its own state from `init`
/// and taking the next item when it is free; the results keep the items' order. Any
/// error fails the whole map.
pub fn in_parallel<I: Sync, S, T: Send>(
    items: &[I],
    init: impl Fn() -> Result<S> + Sync,
    f: impl Fn(&mut S, &I) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let next = AtomicUsize::new(0);
    let worker = || -> Result<Vec<(usize, T)>> {
        let mut state = init()?;
        let mut out = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else {
                return Ok(out);
            };
            out.push((i, f(&mut state, item)?));
        }
    };
    let mut done = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.min(items.len()))
            .map(|_| s.spawn(worker))
            .collect();
        let mut done = Vec::with_capacity(items.len());
        for w in workers {
            match w.join() {
                Ok(results) => done.extend(results?),
                Err(_) => bail!("a worker thread panicked"),
            }
        }
        Ok(done)
    })?;
    done.sort_unstable_by_key(|&(i, _)| i);
    Ok(done.into_iter().map(|(_, t)| t).collect())
}

/// Splits `items` into runs of about `limit` bytes as `size` counts them.
fn batches<T>(items: &[T], size: impl Fn(&T) -> u64, limit: u64) -> Vec<&[T]> {
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (i, item) in items.iter().enumerate() {
        if bytes >= limit {
            out.push(&items[start..i]);
            (start, bytes) = (i, 0);
        }
        bytes += size(item);
    }
    if start < items.len() {
        out.push(&items[start..]);
    }
    out
}

/// A dictionary for the ebx frames, trained on up to [`DICTIONARY_SAMPLES`] of the
/// ebx `entries` (flagged `true`), spread evenly over them.
fn train_dictionary(
    install: &Install,
    decoder: &(dyn Decoder + Sync),
    entries: &[(Location, bool)],
) -> Result<Vec<u8>> {
    let ebx: Vec<Location> = entries.iter().filter(|e| e.1).map(|e| e.0).collect();
    let step = ebx.len().div_ceil(DICTIONARY_SAMPLES).max(1);
    let picked: Vec<Location> = ebx.into_iter().step_by(step).collect();
    let samples = in_parallel(
        &picked,
        || Ok(Archives::new(install)),
        |a, &l| read(a, l, decoder),
    )?;
    Ok(frames::train(&samples))
}

/// An install entry, decompressed, with its location in any error.
fn read(archives: &mut Archives, l: Location, decoder: &dyn Decoder) -> Result<Vec<u8>> {
    archives
        .read(l, decoder)
        .map_err(|e| crate::Error(format!("{l:?}: {e}")))
}

/// What an import wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub superbundles: usize,
    pub bundles: usize,
    pub entries: usize,
    pub packs: Vec<u64>,
    /// The index's size, the dictionary included.
    pub index: u64,
    pub dictionary: u64,
}

/// Copies the superbundles `names` of `install` into a store in `dir`: every distinct
/// entry they point at is decompressed with `decoder` (the install's Oodle) and written
/// as one frame, on every core. Then it reopens the store and checks that it lists the
/// same superbundles and that every entry reads back as the install's, byte for byte.
/// `dir` may be missing, empty, or an earlier store, which is replaced; anything else
/// in it is refused. The index is written last, so an import that stops early leaves
/// no index behind. `progress` hears of each pack written and of the check.
pub fn import(
    install: &Install,
    decoder: &(dyn Decoder + Sync),
    names: &[String],
    dir: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<Imported> {
    clear(dir)?;
    let mut superbundles = Vec::new();
    for name in names {
        superbundles.extend(install.superbundle(name)?);
    }
    // Every distinct entry, in archive order, and whether it is an ebx.
    let mut entries = BTreeMap::new();
    for sb in &mut superbundles {
        each_location(sb, &mut |l| {
            entries.entry(*l).or_insert(false);
        });
        for a in sb.bundles.iter().flat_map(|b| &b.bundle.ebx) {
            entries.insert(a.location, true);
        }
    }
    let entries: Vec<(Location, bool)> = entries.into_iter().collect();
    let dictionary = train_dictionary(install, decoder, &entries)?;
    let prepared = frames::prepare(&dictionary);

    let total: u64 = entries.iter().map(|e| u64::from(e.0.size)).sum();
    let (mut done, mut packer) = (0, Packer::new(PACK_LIMIT));
    let mut placed = BTreeMap::new();
    let mut packs: Vec<u64> = Vec::new();
    let mut out: Option<(PathBuf, BufWriter<File>)> = None;
    for batch in batches(&entries, |e| e.0.size.into(), BATCH) {
        let frames = in_parallel(
            batch,
            || {
                Ok((
                    Archives::new(install),
                    FrameEncoder::new(prepared.as_ref())?,
                ))
            },
            |(archives, encoder), &(l, ebx)| encoder.encode(&read(archives, l, decoder)?, ebx),
        )?;
        for (&(from, _), frame) in batch.iter().zip(frames) {
            let Ok(size) = u32::try_from(frame.len()) else {
                bail!("{from:?}: a frame of {} bytes", frame.len());
            };
            let to = packer.place(size)?;
            if usize::from(to.cas.cas) == packs.len() {
                if let Some((path, file)) = out.take() {
                    finish(&path, file)?;
                    progress(&format!(
                        "{} written ({}%)",
                        path.display(),
                        done * 100 / total
                    ));
                }
                let path = dir.join(pack_name(to.cas.cas));
                let file = File::create(&path).map_err(|e| io_error(&path, e))?;
                out = Some((path, BufWriter::with_capacity(1 << 20, file)));
                packs.push(0);
            }
            let (Some((path, file)), Some(len)) = (&mut out, packs.last_mut()) else {
                bail!("{from:?} is placed before the first pack");
            };
            file.write_all(&frame).map_err(|e| io_error(path, e))?;
            *len += frame.len() as u64;
            done += u64::from(from.size);
            placed.insert(from, to);
        }
    }
    if let Some((path, file)) = out {
        finish(&path, file)?;
        progress(&format!("{} written (100%)", path.display()));
    }

    for sb in &mut superbundles {
        each_location(sb, &mut |l| *l = placed[l]);
    }
    let index = Index {
        packs,
        dictionary,
        superbundles,
        partitions: Vec::new(),
    };
    let path = dir.join(INDEX);
    write_index(dir, &index)?;

    progress("checking the copy");
    let store = Store::open(dir)?;
    if store.superbundles != index.superbundles || store.dictionary != index.dictionary {
        bail!("{}: the index reads back differently", path.display());
    }
    let pairs: Vec<(Location, Location)> = placed.into_iter().collect();
    for batch in batches(&pairs, |p| p.0.size.into(), BATCH) {
        in_parallel(
            batch,
            || Ok((Archives::new(install), store.entries()?)),
            |(source, copy), &(from, to)| {
                if read(source, from, decoder)? != copy.read(to)? {
                    bail!("{from:?}: the copy at {to:?} differs");
                }
                Ok(())
            },
        )?;
    }
    progress("listing the partition GUIDs");
    let index_size = reindex(dir)?;
    Ok(Imported {
        superbundles: store.superbundles.len(),
        bundles: store.superbundles.iter().map(|s| s.bundles.len()).sum(),
        entries: pairs.len(),
        packs: store.packs,
        index: index_size,
        dictionary: store.dictionary.len() as u64,
    })
}

/// Writes `index` as the index of the store in `dir`, under a temporary name renamed
/// at the end, so a reader never sees half an index. Returns its size.
fn write_index(dir: &Path, index: &Index) -> Result<u64> {
    let bytes = encode(index);
    let tmp = dir.join(format!("{INDEX}.tmp"));
    std::fs::write(&tmp, &bytes).map_err(|e| io_error(&tmp, e))?;
    let path = dir.join(INDEX);
    std::fs::rename(&tmp, &path).map_err(|e| io_error(&path, e))?;
    Ok(bytes.len() as u64)
}

/// Reads the partition GUID of every ebx in the store in `dir`, on every core, and
/// rewrites its index with them (replacing any it had). Every ebx is parsed on the
/// way, so a damaged one fails here. Returns the new index's size. Running it again
/// writes the same index.
pub fn reindex(dir: &Path) -> Result<u64> {
    let store = Store::open(dir)?;
    // Entries shared by several bundles are read once.
    let mut guids: BTreeMap<Location, [u8; 16]> = each_ebx(&store.superbundles)
        .map(|(_, a)| (a.location, [0; 16]))
        .collect();
    let locations: Vec<Location> = guids.keys().copied().collect();
    let found = in_parallel(
        &locations,
        || store.entries(),
        |entries, &l| {
            let data = entries.read(l)?;
            let partition =
                Partition::parse(&data).map_err(|e| crate::Error(format!("{l:?}: {e}")))?;
            Ok(partition.header.guid)
        },
    )?;
    for (l, guid) in locations.into_iter().zip(found) {
        guids.insert(l, guid);
    }
    let partitions = each_ebx(&store.superbundles)
        .map(|(_, a)| guids[&a.location])
        .collect();
    let Store {
        superbundles,
        packs,
        dictionary,
        ..
    } = store;
    write_index(
        dir,
        &Index {
            packs,
            dictionary,
            superbundles,
            partitions,
        },
    )
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

/// What `index.nfsh` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// Each pack's size in bytes.
    pub packs: Vec<u64>,
    /// The ebx frames' dictionary; empty for none.
    pub dictionary: Vec<u8>,
    /// With their locations in the packs.
    pub superbundles: Vec<Superbundle>,
    /// Each ebx's partition GUID, in the order the superbundles list them; empty for
    /// none.
    pub partitions: Vec<[u8; 16]>,
}

pub fn encode(index: &Index) -> Vec<u8> {
    let mut w = Writer(MAGIC.to_vec());
    w.u32(VERSION);
    w.count(&index.packs);
    index.packs.iter().for_each(|&p| w.u64(p));
    w.bytes(&index.dictionary);
    w.count(&index.superbundles);
    for sb in &index.superbundles {
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
    w.count(&index.partitions);
    index.partitions.iter().for_each(|g| w.raw(g));
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

/// Reads an index back.
pub fn decode(bytes: &[u8]) -> Result<Index> {
    let mut r = Reader::new(bytes);
    if r.bytes(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
        bail!("not an nfsh store index");
    }
    let version = r.u32_be()?;
    if version != VERSION && version != VERSION_WITHOUT_GUIDS {
        bail!(
            "store version {version}; this nfsh reads versions {VERSION_WITHOUT_GUIDS} and {VERSION}"
        );
    }
    let mut i = IndexReader {
        r,
        packs: Vec::new(),
    };
    i.packs = i.list(|i| i.r.u64_be())?;
    if i.packs.len() > 256 {
        bail!("{} packs; at most 256", i.packs.len());
    }
    let len = i.r.u32_be()? as usize;
    let dictionary = i.r.bytes(len)?.to_vec();
    let superbundles = i.list(IndexReader::superbundle)?;
    let partitions = match version {
        VERSION => i.list(|i| i.r.array())?,
        _ => Vec::new(),
    };
    let ebx = each_ebx(&superbundles).count();
    if !partitions.is_empty() && partitions.len() != ebx {
        bail!("{} partition GUIDs for {ebx} ebx", partitions.len());
    }
    if i.r.pos() != bytes.len() {
        bail!("{} bytes after the index", bytes.len() - i.r.pos());
    }
    Ok(Index {
        packs: i.packs,
        dictionary,
        superbundles,
        partitions,
    })
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

    fn index(packs: Vec<u64>) -> Index {
        Index {
            packs,
            dictionary: b"dict".to_vec(),
            superbundles: sample(),
            partitions: vec![[8; 16]],
        }
    }

    #[test]
    fn the_index_reads_back_as_written() {
        let bytes = encode(&index(vec![30, 35]));
        assert_eq!(decode(&bytes), Ok(index(vec![30, 35])));
    }

    #[test]
    fn damaged_indexes_are_refused() {
        let bytes = encode(&index(vec![30, 35]));
        for len in [0, 8, 20, bytes.len() - 1] {
            assert!(decode(&bytes[..len]).is_err(), "truncated to {len}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_err());
        // The second pack is too short for the toc chunk at 30..35.
        assert!(decode(&encode(&index(vec![30, 34]))).is_err());
        // A version 1 store kept Heat's Oodle blocks; this nfsh can't read it.
        let mut version = bytes.clone();
        version[11] = 1;
        assert!(decode(&version).unwrap_err().0.contains("version 1"));
        // One GUID per ebx, or none.
        let mut two = index(vec![30, 35]);
        two.partitions.push([9; 16]);
        assert!(
            decode(&encode(&two))
                .unwrap_err()
                .0
                .contains("2 partition GUIDs")
        );
    }

    #[test]
    fn a_version_2_index_reads_without_guids() {
        let mut old = index(vec![30, 35]);
        old.partitions.clear();
        let mut bytes = encode(&old);
        // Version 2 ended after the superbundles: drop the empty GUID list.
        bytes.truncate(bytes.len() - 4);
        bytes[11] = 2;
        assert_eq!(decode(&bytes), Ok(old));
    }

    #[test]
    fn packs_close_before_the_limit_and_keep_entry_order() {
        let mut packer = Packer::new(100);
        let placed: Vec<_> = [60, 40, 10, 150]
            .map(|size| packer.place(size).unwrap())
            .into();
        assert_eq!(
            placed,
            // 60 + 40 fills the first pack exactly; an entry larger than the limit
            // gets a pack of its own size.
            [at(0, 0, 60), at(0, 60, 40), at(1, 0, 10), at(2, 0, 150)]
        );
        let mut full = Packer::new(1);
        for _ in 0..256 {
            full.place(1).unwrap();
        }
        assert!(full.place(1).is_err());
    }

    #[test]
    fn parallel_maps_keep_order_and_fail_on_any_error() {
        let items: Vec<u32> = (0..1000).collect();
        let doubled = in_parallel(&items, || Ok(()), |_, &i| Ok(i * 2)).unwrap();
        assert_eq!(doubled, items.iter().map(|i| i * 2).collect::<Vec<_>>());
        let failed = in_parallel(
            &items,
            || Ok(()),
            |_, &i| if i == 700 { bail!("item {i}") } else { Ok(i) },
        );
        assert_eq!(failed, Err(crate::Error("item 700".into())));
        let none: Vec<u32> = in_parallel(&[], || Ok(()), |_, &i: &u32| Ok(i)).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn batches_split_by_size() {
        let sizes = [5u64, 5, 1, 20, 3];
        let runs = batches(&sizes, |&s| s, 10);
        assert_eq!(runs, [&[5, 5][..], &[1, 20], &[3]]);
        assert!(batches(&[] as &[u64], |&s| s, 10).is_empty());
    }

    /// Writes a store by hand: two frames in one pack, one of them with the dictionary.
    #[test]
    fn a_store_reads_its_entries_back() {
        let dir = std::env::temp_dir().join(format!("nfsh-entries-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let samples: Vec<Vec<u8>> = (0..300u32)
            .map(|i| format!("ebx partition {i}; RaceVehicleBlueprint; car_{}", i % 37).into())
            .collect();
        let dictionary = frames::train(&samples);
        let prepared = frames::prepare(&dictionary);
        let mut encoder = FrameEncoder::new(prepared.as_ref()).unwrap();
        let ebx = encoder.encode(&samples[3], true).unwrap();
        let chunk = encoder.encode(&[7; 5000], false).unwrap();
        std::fs::write(
            dir.join(pack_name(0)),
            [ebx.clone(), chunk.clone()].concat(),
        )
        .unwrap();
        let mut sb = sample();
        let (e, c) = (ebx.len() as u32, chunk.len() as u32);
        sb[0].bundles[0].bundle.ebx[0].location = at(0, 0, e);
        sb[0].bundles[0].bundle.res.clear();
        sb[0].bundles[0].bundle.chunks.clear();
        sb[0].chunks[0].chunk.location = at(0, e, c);
        let packs = vec![u64::from(e + c)];
        std::fs::write(
            dir.join(INDEX),
            encode(&Index {
                packs,
                dictionary,
                superbundles: sb,
                partitions: Vec::new(),
            }),
        )
        .unwrap();
        let store = Store::open(&dir).unwrap();
        let mut entries = store.entries().unwrap();
        assert_eq!(entries.read(at(0, 0, e)).unwrap(), samples[3]);
        assert_eq!(entries.read(at(0, e, c)).unwrap(), [7; 5000]);
        // A location that isn't one whole frame is an error, as is one past the pack.
        assert!(entries.read(at(0, 0, e + c)).is_err());
        assert!(entries.read(at(0, e, c + 1)).is_err());
        assert!(entries.read(at(1, 0, 4)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A store by hand whose one ebx is the ebx tests' sample partition, listed by two
    /// bundles: `reindex` finds its GUID, and the store finds both copies by it.
    #[test]
    fn reindex_lists_every_copy_of_a_partition() {
        let dir = std::env::temp_dir().join(format!("nfsh-reindex-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let frame = FrameEncoder::new(None)
            .unwrap()
            .encode(&crate::ebx::tests::sample(), true)
            .unwrap();
        std::fs::write(dir.join(pack_name(0)), &frame).unwrap();
        let mut sb = sample();
        let bundle = &mut sb[0].bundles[0];
        bundle.bundle.ebx[0].location = at(0, 0, frame.len() as u32);
        bundle.bundle.res.clear();
        bundle.bundle.chunks.clear();
        let copy = bundle.clone();
        sb[0].bundles.push(copy);
        sb[0].chunks.clear();
        let old = Index {
            packs: vec![frame.len() as u64],
            dictionary: Vec::new(),
            superbundles: sb,
            partitions: Vec::new(),
        };
        std::fs::write(dir.join(INDEX), encode(&old)).unwrap();
        let store = Store::open(&dir).unwrap();
        assert!(!store.has_guids());
        assert!(store.partitions(&[0x44; 16]).is_err());

        let size = reindex(&dir).unwrap();
        let bytes = std::fs::read(dir.join(INDEX)).unwrap();
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(decode(&bytes).unwrap().partitions, [[0x44; 16]; 2]);
        let store = Store::open(&dir).unwrap();
        let found: Vec<EbxRef> = store.partitions(&[0x44; 16]).unwrap().collect();
        let at = |bundle| EbxRef {
            superbundle: 0,
            bundle,
            ebx: 0,
        };
        assert_eq!(found, [at(0), at(1)]);
        assert_eq!(store.ebx_at(at(1)).unwrap().1.name, "cars/a");
        assert!(store.ebx_at(at(2)).is_none());
        assert_eq!(store.partitions(&[0x45; 16]).unwrap().count(), 0);
        // Again: the same index.
        reindex(&dir).unwrap();
        assert_eq!(std::fs::read(dir.join(INDEX)).unwrap(), bytes);
        std::fs::remove_dir_all(&dir).unwrap();
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
        let root = Path::new(&root);
        let install = Install::open(root).unwrap();
        let oodle = crate::oodle::Oodle::load(root).unwrap();
        let dir = std::env::temp_dir().join(format!("nfsh-import-test-{}", std::process::id()));
        let names = ["Win32/globals".to_owned()];
        let mut said = Vec::new();
        let imported = import(&install, &oodle, &names, &dir, &mut |s| {
            said.push(s.to_owned())
        });
        let imported = imported.unwrap();
        let store = Store::open(&dir).unwrap();
        assert_eq!(store.superbundles.len(), 1);
        assert_eq!(store.packs, imported.packs);
        assert!(imported.entries > 100, "only {} entries", imported.entries);
        assert!(imported.dictionary > 0, "no dictionary");
        assert_eq!(
            said.last().map(String::as_str),
            Some("listing the partition GUIDs")
        );
        assert!(store.has_guids());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
