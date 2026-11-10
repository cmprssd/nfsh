//! nfsh's own copy of an install's data, a *store*: the entries the patched tables
//! still point at, recompressed into a few packs (`pack_NN.nfsh`), and one index
//! (`index.nfsh`) holding the superbundles with their bundles and chunks. The dead
//! `Data/` bytes the patch replaced are left behind. An entry is zstd frames (see
//! [`crate::frames`]), so a store reads anywhere, without the install's Oodle library.
//! What the store keeps of them, and how, is [`crate::plan`]'s decision; every frame is
//! tagged in the index with its part (the base, or one language) and tier.
//!
//! In a store, the archive a [`Location`] names is a pack: [`CasRef::cas`] is the pack
//! number, with `patch` false and `package` 0. The index is our own format (see
//! `docs/FORMATS.md`), big-endian like Heat's tables. It also lists each ebx's
//! partition GUID, so a reference into another partition is found without reading
//! every ebx.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cas::{Archives, Decoder};
use crate::ebx::Partition;
use crate::frames::{self, FrameDecoder, FrameEncoder};
use crate::install::{BundleEntry, ChunkEntry, Install, Layer, Superbundle};
use crate::mesh::MeshSet;
use crate::plan::{self, Fate, Finder, Listing};
use crate::reader::Reader;
use crate::sb::{Asset, Bundle, Chunk, Res, bundle_chunk_size, res_type_hash};
use crate::texture::{self, Texture};
use crate::toc::TocChunk;
use crate::{CasRef, Location, Result, bail};

pub const INDEX: &str = "index.nfsh";
const MAGIC: &[u8; 8] = b"NFSHSTOR";
const VERSION: u32 = 4;
/// The oldest version still read: such a store has no partition GUIDs until [`reindex`]
/// adds them. Version 3 added the GUIDs, and version 4 the frames' table and parts.
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

pub(crate) fn io_error(path: &Path, e: std::io::Error) -> crate::Error {
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
    /// The name of each part; empty for a store made before them.
    pub parts: Vec<String>,
    /// Every frame, in the packs' order; empty for a store made before parts.
    pub frames: Vec<Frame>,
    /// For an install ([`crate::copy`]), where its frames are; `None` for a store.
    pub placement: Option<crate::copy::Placement>,
}

/// One zstd frame of a pack, with what it is for. A pack holds frames back to back, so
/// a frame's place follows from those before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub size: u32,
    /// An index into [`Store::parts`].
    pub part: u8,
    /// See [`plan::tier`].
    pub tier: u8,
}

/// Where an ebx is listed in a store: indices into [`Store::superbundles`], its
/// bundles, and their ebx.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EbxRef {
    pub superbundle: u32,
    pub bundle: u32,
    pub ebx: u32,
}

/// A GUID as stored in memory (a res, an ebx) in the order a bundle's chunk list
/// stores it: its first three fields byte-swapped.
pub fn guid_order(mut id: [u8; 16]) -> [u8; 16] {
    id[0..4].reverse();
    id[4..6].reverse();
    id[6..8].reverse();
    id
}

/// The chunk id a `.toc` lists in the order a bundle's chunk list stores it: a toc
/// stores the in-memory GUID with all 16 bytes reversed.
pub fn toc_chunk_id(mut id: [u8; 16]) -> [u8; 16] {
    id.reverse();
    guid_order(id)
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
        let mut by_guid: Vec<([u8; 16], EbxRef)> = index
            .partitions
            .iter()
            .zip(each_ebx(&index.superbundles))
            .map(|(&guid, (at, _))| (guid, at))
            .collect();
        by_guid.sort_unstable();
        let mut store = Self {
            dir: dir.to_owned(),
            superbundles: index.superbundles,
            packs: index.packs,
            dictionary: index.dictionary,
            by_guid,
            parts: index.parts,
            frames: index.frames,
            placement: None,
        };
        // An install has its own files in place of the packs.
        store.placement = crate::copy::Placement::open(dir, &store)?;
        if store.placement.is_none() {
            for (n, &size) in store.packs.iter().enumerate() {
                let path = dir.join(pack_name(n as u8));
                let len = std::fs::metadata(&path)
                    .map_err(|e| io_error(&path, e))?
                    .len();
                if len != size {
                    bail!("{}: {len} bytes, the index says {size}", path.display());
                }
            }
        }
        Ok(store)
    }

    /// Each part's stored bytes in each tier.
    pub fn part_sizes(&self) -> BTreeMap<(u8, u8), u64> {
        let mut sizes = BTreeMap::new();
        for f in &self.frames {
            *sizes.entry((f.part, f.tier)).or_default() += u64::from(f.size);
        }
        sizes
    }

    /// The part of the frame at `location`'s start, given [`Self::frame_starts`].
    fn part_at(&self, starts: &[(u8, u32)], location: Location) -> Option<u8> {
        let at = (location.cas.cas, location.offset);
        let n = starts.binary_search(&at).ok()?;
        Some(self.frames[n].part)
    }

    /// Where each frame starts, as (pack, offset), sorted because the frames are in the
    /// packs' order.
    pub(crate) fn frame_starts(&self) -> Vec<(u8, u32)> {
        let mut starts = Vec::with_capacity(self.frames.len());
        let (mut pack, mut offset, mut packs) = (0u8, 0u64, self.packs.iter());
        let mut size = packs.next().copied().unwrap_or(0);
        for f in &self.frames {
            while offset >= size {
                (pack, offset) = (pack.wrapping_add(1), 0);
                size = packs.next().copied().unwrap_or(u64::MAX);
            }
            starts.push((pack, offset as u32));
            offset += u64::from(f.size);
        }
        starts
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

    /// The name of the partition `guid`, if the store holds it.
    pub fn partition_name(&self, guid: &[u8; 16]) -> Option<&str> {
        let at = self.partitions(guid).ok()?.next()?;
        Some(&self.ebx_at(at)?.1.name)
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

    /// The first res named `name` (ignoring case), in any bundle.
    pub fn res(&self, name: &str) -> Option<&Asset> {
        self.superbundles
            .iter()
            .flat_map(|sb| &sb.bundles)
            .flat_map(|b| &b.bundle.res)
            .map(|r| &r.asset)
            .find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Every res of type `res_type` by id; the first copy of each.
    pub fn res_of_type(&self, res_type: u32) -> HashMap<u64, Location> {
        let mut res = HashMap::new();
        for b in self.superbundles.iter().flat_map(|sb| &sb.bundles) {
            for r in b.bundle.res.iter().filter(|r| r.res_type == res_type) {
                res.entry(r.id).or_insert(r.asset.location);
            }
        }
        res
    }

    /// Every chunk stored whole, by id as bundles list it: a superbundle's chunks, and
    /// a bundle's chunk entries from logical offset 0. A bundle stores only a chunk's
    /// blocks that cover its logical range, so one from a later offset holds only the
    /// chunk's end (a texture's last mips) and is left out.
    pub fn whole_chunks(&self) -> HashMap<[u8; 16], Location> {
        let mut chunks = HashMap::new();
        for sb in &self.superbundles {
            for b in &sb.bundles {
                for c in b.bundle.chunks.iter().filter(|c| c.logical_offset == 0) {
                    chunks.entry(c.id).or_insert(c.location);
                }
            }
            for c in &sb.chunks {
                chunks
                    .entry(toc_chunk_id(c.chunk.id))
                    .or_insert(c.chunk.location);
            }
        }
        chunks
    }

    /// Reads the frames as stored.
    pub fn archives(&self) -> Archives<'_> {
        Archives::with_paths(|cas| self.pack_path(cas))
    }

    /// Reads the entries, decompressed.
    pub fn entries(&self) -> Result<Entries<'_>> {
        Ok(Entries {
            archives: self.archives(),
            placed: self.placement.as_ref().map(crate::copy::Placed::new),
            frames: FrameDecoder::new(&self.dictionary)?,
        })
    }

    /// Whether the install leaves out any of the entry at `location`; never for a store.
    pub fn is_cut(&self, location: Location) -> Result<bool> {
        match &self.placement {
            Some(p) => Ok(p.is_cut(location)?),
            None => Ok(false),
        }
    }
}

/// A store's entries, read and decompressed; one per thread.
pub struct Entries<'a> {
    archives: Archives<'a>,
    /// An install's files, read in place of the packs.
    placed: Option<crate::copy::Placed<'a>>,
    frames: FrameDecoder,
}

impl Entries<'_> {
    /// The decompressed entry at `location`; an error when an install left out any of it.
    pub fn read(&mut self, location: Location) -> Result<Vec<u8>> {
        match self.read_end(location)? {
            (data, false) => Ok(data),
            (_, true) => bail!("{location:?}: left out of this install"),
        }
    }

    /// What an install keeps of the entry at `location`, decompressed: all of it, or
    /// its end when its first frames were left out (a texture without its top mips),
    /// with whether any were. Empty when it was left out altogether.
    pub fn read_end(&mut self, location: Location) -> Result<(Vec<u8>, bool)> {
        let (frame, cut) = match &mut self.placed {
            Some(placed) => placed.stored(location)?,
            None => (self.archives.stored(location)?, false),
        };
        if frame.is_empty() && cut {
            return Ok((Vec::new(), true));
        }
        let data = self
            .frames
            .decode(&frame)
            .map_err(|e| crate::Error(format!("{location:?}: {e}")))?;
        Ok((data, cut))
    }

    /// What the bundle's `chunk` holds: its whole blocks over its logical range. In a
    /// store, a chunk's end may be read from the whole chunk it is the end of.
    pub fn read_chunk(&mut self, chunk: &Chunk) -> Result<Vec<u8>> {
        let mut data = self.read(chunk.location)?;
        let size = bundle_chunk_size(chunk.logical_offset, chunk.logical_size) as usize;
        if data.len() > size && chunk.logical_offset > 0 {
            let from = (chunk.logical_offset & !0xFFFF) as usize;
            let Some(part) = data.get(from..from + size) else {
                bail!(
                    "chunk {:02x?}: {size} bytes at {from:#x} of {}",
                    chunk.id,
                    data.len()
                );
            };
            data = part.to_vec();
        }
        Ok(data)
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
    /// What wasn't stored again, in the install's stored bytes.
    pub saved: plan::Summary,
    /// Each part's stored bytes, by name, with its bytes in each tier.
    pub parts: Vec<(String, u64)>,
    pub tiers: Vec<(String, u64)>,
}

/// Copies the superbundles `names` of `install` into a store in `dir`. Every distinct
/// entry they point at is read once with `decoder` (the install's Oodle) to decide what
/// to keep ([`plan`]); what is kept is written as its frames, on every core. Then it
/// reopens the store and checks that it lists the same superbundles, less the chunks
/// left out, and that every other entry reads back as the install's, byte for byte.
/// `dir` may be missing, empty, or an earlier store, which is replaced; anything else
/// in it is refused. The index is written last, so an import that stops early leaves
/// no index behind. `progress` hears of each step and each pack written.
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
    let listing = Listing::new(&superbundles);
    let finder = Finder::new(&listing.ids);
    progress(&format!("reading {} entries", listing.entries.len()));
    let scans = in_parallel(
        &listing.entries,
        || Ok(Archives::new(install)),
        |archives, entry| {
            let data = read(archives, entry.location, decoder)?;
            Ok(plan::scan(entry, &data, &listing, &finder))
        },
    )?;
    let fates = plan::decide(&listing, &scans);
    let saved = plan::summary(&listing, &fates);
    progress(&format!("{saved:?}"));

    // Every entry kept, in archive order, and whether it is an ebx.
    let kept: Vec<(usize, Location, bool)> = fates
        .iter()
        .enumerate()
        .filter_map(|(n, f)| match f {
            Fate::Keep(k) => Some((n, listing.entries[n].location, k.ebx)),
            _ => None,
        })
        .collect();
    let samples: Vec<(Location, bool)> = kept.iter().map(|k| (k.1, k.2)).collect();
    let dictionary = train_dictionary(install, decoder, &samples)?;
    let prepared = frames::prepare(&dictionary);

    let total: u64 = kept.iter().map(|k| u64::from(k.1.size)).sum();
    let (mut done, mut packer) = (0, Packer::new(PACK_LIMIT));
    let mut placed: Vec<Option<Location>> = vec![None; fates.len()];
    let mut packs: Vec<u64> = Vec::new();
    let mut table: Vec<Frame> = Vec::new();
    let mut out: Option<(PathBuf, BufWriter<File>)> = None;
    for batch in batches(&kept, |k| k.1.size.into(), BATCH) {
        let written = in_parallel(
            batch,
            || {
                Ok((
                    Archives::new(install),
                    FrameEncoder::new(prepared.as_ref())?,
                ))
            },
            |(archives, encoder), &(n, l, ebx)| {
                let data = read(archives, l, decoder)?;
                let Fate::Keep(keep) = &fates[n] else {
                    bail!("{l:?} is not kept");
                };
                let mut stored = Vec::new();
                let mut frames = Vec::new();
                for (range, tier) in keep.pieces() {
                    let frame = encoder.encode(&data[range], ebx)?;
                    frames.push(Frame {
                        size: frame.len() as u32,
                        part: keep.part,
                        tier,
                    });
                    stored.extend(frame);
                }
                Ok((stored, frames))
            },
        )?;
        for (&(n, from, _), (stored, frames)) in batch.iter().zip(written) {
            let Ok(size) = u32::try_from(stored.len()) else {
                bail!("{from:?}: {} stored bytes", stored.len());
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
            file.write_all(&stored).map_err(|e| io_error(path, e))?;
            *len += stored.len() as u64;
            done += u64::from(from.size);
            table.extend(frames);
            placed[n] = Some(to);
        }
    }
    if let Some((path, file)) = out {
        finish(&path, file)?;
        progress(&format!("{} written (100%)", path.display()));
    }

    // Where every entry listed is read from now, by where it was; none for those left
    // out, which leave the tables too.
    let moved: HashMap<Location, Location> = listing
        .entries
        .iter()
        .zip(&fates)
        .enumerate()
        .filter_map(|(n, (e, fate))| {
            let holder = match fate {
                Fate::Keep(_) => n,
                Fate::Same(w) | Fate::InWhole(w) => *w as usize,
                Fate::Dropped => return None,
            };
            Some((e.location, placed[holder]?))
        })
        .collect();
    for sb in &mut superbundles {
        sb.chunks.retain(|c| moved.contains_key(&c.chunk.location));
        each_location(sb, &mut |l| *l = moved[l]);
    }
    let index = Index {
        packs,
        dictionary,
        superbundles,
        partitions: Vec::new(),
        parts: plan::part_names(),
        frames: table,
    };
    let path = dir.join(INDEX);
    write_index(dir, &index)?;

    progress("checking the copy");
    let store = Store::open(dir)?;
    if store.superbundles != index.superbundles || store.dictionary != index.dictionary {
        bail!("{}: the index reads back differently", path.display());
    }
    // Every entry against what the store now reads for it, a chunk's end from the whole
    // chunk where it is read from there.
    let pairs: Vec<Copied> = listing
        .entries
        .iter()
        .zip(&fates)
        .filter_map(|(e, fate)| {
            let to = *moved.get(&e.location)?;
            let tail = matches!(fate, Fate::InWhole(_)).then_some(e.tail).flatten();
            Some((e.location, to, tail))
        })
        .collect();
    for batch in batches(&pairs, |p| p.0.size.into(), BATCH) {
        in_parallel(
            batch,
            || Ok((Archives::new(install), store.entries()?)),
            |(source, copy), &(from, to, tail)| {
                let theirs = read(source, from, decoder)?;
                let ours = copy.read(to)?;
                let same = match tail {
                    Some((at, len)) => {
                        let (at, len) = (at as usize, len as usize);
                        ours.get(at..at + len) == Some(&theirs[..])
                    }
                    None => ours == theirs,
                };
                if !same {
                    bail!("{from:?}: the copy at {to:?} differs");
                }
                Ok(())
            },
        )?;
    }
    progress("listing the partition GUIDs");
    let index_size = reindex(dir)?;
    let (mut parts, mut tiers) = (BTreeMap::new(), BTreeMap::new());
    for (&(part, tier), &bytes) in &store.part_sizes() {
        *parts.entry(part).or_insert(0) += bytes;
        *tiers.entry(tier).or_insert(0) += bytes;
    }
    let names = plan::part_names();
    Ok(Imported {
        superbundles: store.superbundles.len(),
        bundles: store.superbundles.iter().map(|s| s.bundles.len()).sum(),
        entries: kept.len(),
        packs: store.packs,
        index: index_size,
        dictionary: store.dictionary.len() as u64,
        saved,
        parts: parts
            .into_iter()
            .map(|(p, b)| (names[usize::from(p)].clone(), b))
            .collect(),
        tiers: tiers
            .into_iter()
            .map(|(t, b)| (plan::tier::name(t), b))
            .collect(),
    })
}

/// An entry of the install, where the store reads it from, and for a chunk's end read
/// from the whole chunk, its start and length there.
type Copied = (Location, Location, Option<(u32, u32)>);

/// What a store's parts refer to, so that the base and any one language are enough.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PartsCheck {
    /// The ebx and res read.
    pub entries: u64,
    /// The chunks those name, counting each entry's once.
    pub names: u64,
    /// An ebx's references to another partition, by where they lead.
    pub references: u64,
    pub base_to_language: u64,
    pub language_to_other: u64,
    /// Chunks a localized wave in the base (or a cinematic wave's res) names in a
    /// language's part: a cinematic's wave names a chunk per language.
    pub wave_to_language: u64,
    /// To a partition the store doesn't have, as the install doesn't either.
    pub to_nothing: u64,
    pub failures: Vec<String>,
    pub failed: u64,
}

impl PartsCheck {
    fn fail(&mut self, message: String) {
        self.failed += 1;
        if self.failures.len() < 20 {
            self.failures.push(message);
        }
    }

    pub fn markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Parts: {} ebx and res name {} chunks, {} failed, {} of them a localized wave's \
             in the base naming a language's chunk. Of {} references to other \
             partitions, {} go from the base to a language, {} from a language to another, \
             {} to a partition no table lists.",
            self.entries,
            self.names,
            self.failed,
            self.wave_to_language,
            self.references,
            self.base_to_language,
            self.language_to_other,
            self.to_nothing,
        );
        for f in &self.failures {
            let _ = writeln!(out, "- {f}");
        }
        out
    }
}

/// Checks that the base and any one language find everything they refer to: a chunk an
/// entry names must be in the base or in that entry's own part (or, for a localized
/// wave in the base and a cinematic wave's res, in any language's: a cinematic's wave
/// names a chunk per language),
/// and a texture or a mesh must find its chunks at all. Every ebx and res is read once.
pub fn check_parts(store: &Store) -> Result<PartsCheck> {
    let mut report = PartsCheck::default();
    if store.frames.is_empty() {
        return Ok(report);
    }
    let starts = store.frame_starts();
    let whole = store.whole_chunks();
    // Every chunk the tables list, with where one of its entries is.
    let mut ids: HashMap<[u8; 16], u32> = HashMap::new();
    let mut listed: Vec<Location> = Vec::new();
    let mut add = |id: [u8; 16], l: Location| {
        let next = ids.len() as u32;
        let n = *ids.entry(id).or_insert(next);
        if n == next {
            listed.push(l);
        }
    };
    for (&id, &l) in &whole {
        add(id, l);
    }
    for b in store.superbundles.iter().flat_map(|s| &s.bundles) {
        for c in &b.bundle.chunks {
            add(c.id, c.location);
        }
    }
    let finder = Finder::new(&ids);
    // A cinematic wave's res, of the wave's name, names its chunks as the wave does.
    let mut cinematic_res: HashSet<Location> = HashSet::new();
    let mut wanted: BTreeMap<Location, plan::Kind> = BTreeMap::new();
    for (_, a) in each_ebx(&store.superbundles) {
        wanted.insert(a.location, plan::Kind::Ebx);
    }
    for b in store.superbundles.iter().flat_map(|s| &s.bundles) {
        for r in &b.bundle.res {
            wanted.insert(r.asset.location, plan::Kind::Res(r.res_type));
            if plan::language(&r.asset.name) == plan::CINEMATIC {
                cinematic_res.insert(r.asset.location);
            }
        }
    }
    // An install checks what it keeps: an entry left out refers to nothing.
    let mut kept = Vec::with_capacity(wanted.len());
    for (l, kind) in wanted {
        if !store.is_cut(l)? {
            kept.push((l, kind));
        }
    }
    let wanted = kept;
    struct Found {
        names: Vec<u32>,
        /// What must be there, whatever the tables' other chunks are.
        needs: Vec<[u8; 16]>,
        imports: Vec<[u8; 16]>,
        wave: bool,
    }
    let found = in_parallel(
        &wanted,
        || store.entries(),
        |entries, &(l, kind)| {
            let data = entries.read(l)?;
            let mut f = Found {
                names: Vec::new(),
                needs: Vec::new(),
                imports: Vec::new(),
                wave: false,
            };
            match kind {
                plan::Kind::Res(texture::TEXTURE) => {
                    if let Ok(t) = Texture::parse(&data) {
                        f.needs.push(t.chunk);
                        f.names.extend(ids.get(&t.chunk).copied());
                    }
                }
                plan::Kind::Ebx => {
                    f.names = finder.scan(&data);
                    if let Ok(p) = Partition::parse(&data) {
                        f.wave = p.instances.first().map(|i| p.type_name(i.type_index))
                            == Some("LocalizedWaveAsset".into());
                        f.imports = p.imports.iter().map(|i| i.partition).collect();
                        f.imports.sort_unstable();
                        f.imports.dedup();
                    }
                }
                _ => {
                    f.names = finder.scan(&data);
                    f.wave = cinematic_res.contains(&l);
                    if kind == plan::Kind::Res(res_type_hash("MeshSet"))
                        && let Ok(m) = MeshSet::parse(&data)
                    {
                        f.needs = m.lods.iter().filter_map(|l| l.chunk).collect();
                    }
                }
            }
            Ok(f)
        },
    )?;
    for (&(l, _), f) in wanted.iter().zip(found) {
        report.entries += 1;
        let Some(part) = store.part_at(&starts, l) else {
            report.fail(format!("{l:?} is not at the start of a frame"));
            continue;
        };
        for id in f.needs {
            if !ids.contains_key(&id) {
                report.fail(format!("{l:?} needs chunk {id:02x?}, which is not listed"));
            }
        }
        for n in f.names {
            report.names += 1;
            let target = listed[n as usize];
            match store.part_at(&starts, target) {
                Some(p) if p == plan::BASE || p == part => {}
                Some(_) if f.wave && part == plan::BASE => report.wave_to_language += 1,
                Some(p) => report.fail(format!(
                    "{l:?} in part {part} names a chunk in part {p}, at {target:?}"
                )),
                None => report.fail(format!("{target:?} is not at the start of a frame")),
            }
        }
        for guid in f.imports {
            report.references += 1;
            let target = store.by_guid.partition_point(|(g, _)| *g < guid);
            let at = store.by_guid.get(target).filter(|(g, _)| *g == guid);
            let Some((asset_at, target)) = at.and_then(|&(_, e)| Some((e, store.ebx_at(e)?.1)))
            else {
                report.to_nothing += 1;
                continue;
            };
            let _ = asset_at;
            match store.part_at(&starts, target.location) {
                Some(p) if p == plan::BASE || p == part => {}
                Some(_) if part == plan::BASE => report.base_to_language += 1,
                Some(_) => report.language_to_other += 1,
                None => report.fail(format!(
                    "{:?} is not at the start of a frame",
                    target.location
                )),
            }
        }
    }
    Ok(report)
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
        parts,
        frames,
        ..
    } = store;
    write_index(
        dir,
        &Index {
            packs,
            dictionary,
            superbundles,
            partitions,
            parts,
            frames,
        },
    )
}

pub(crate) fn finish(path: &Path, file: BufWriter<File>) -> Result<()> {
    let file = file
        .into_inner()
        .map_err(|e| io_error(path, e.into_error()))?;
    file.sync_all().map_err(|e| io_error(path, e))
}

/// Makes `dir` an empty folder, removing an earlier store's or install's files (every
/// name ending in `.nfsh`, or `.nfsh.tmp` from a copy cut short); refuses if it holds
/// anything else.
pub(crate) fn clear(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    let mut ours = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| io_error(dir, e))? {
        let path = entry.map_err(|e| io_error(dir, e))?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ours_by_name = name.ends_with(".nfsh") || name.ends_with(".nfsh.tmp");
        if !ours_by_name || !path.is_file() {
            bail!(
                "{}: not an nfsh store (holds {name}); refusing to write into it",
                dir.display()
            );
        }
        ours.push(path);
    }
    // The index and an install's placement go first, so a store is never left with an
    // index and missing packs.
    ours.sort_by_key(|p| !(p.ends_with(INDEX) || p.ends_with(crate::copy::PLACEMENT)));
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
    /// The name of each part; empty for a store without parts.
    pub parts: Vec<String>,
    /// Every frame, in the packs' order; empty for a store without parts.
    pub frames: Vec<Frame>,
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
    w.count(&index.parts);
    index.parts.iter().for_each(|p| w.bytes(p.as_bytes()));
    w.count(&index.frames);
    for f in &index.frames {
        w.u32(f.size);
        w.u8(f.part);
        w.u8(f.tier);
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

/// Reads an index back.
pub fn decode(bytes: &[u8]) -> Result<Index> {
    let mut r = Reader::new(bytes);
    if r.bytes(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
        bail!("not an nfsh store index");
    }
    let version = r.u32_be()?;
    if !(VERSION_WITHOUT_GUIDS..=VERSION).contains(&version) {
        bail!(
            "store version {version}; this nfsh reads versions {VERSION_WITHOUT_GUIDS} to {VERSION}"
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
        VERSION_WITHOUT_GUIDS => Vec::new(),
        _ => i.list(|i| i.r.array())?,
    };
    let (parts, frames) = match version {
        VERSION => {
            let parts = i.list(IndexReader::string)?;
            let frames = i.list(|i| {
                Ok(Frame {
                    size: i.r.u32_be()?,
                    part: i.r.u8()?,
                    tier: i.r.u8()?,
                })
            })?;
            (parts, frames)
        }
        _ => (Vec::new(), Vec::new()),
    };
    check_frames(&i.packs, &parts, &frames)?;
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
        parts,
        frames,
    })
}

/// A store's frames must fill its packs exactly, each pack's starting at its start, and
/// name a part that exists. A store with no frames has no parts at all.
fn check_frames(packs: &[u64], parts: &[String], frames: &[Frame]) -> Result<()> {
    if frames.is_empty() {
        return Ok(());
    }
    let mut frame = frames.iter();
    for (n, &size) in packs.iter().enumerate() {
        let mut sum = 0;
        while sum < size {
            let Some(f) = frame.next() else {
                bail!("the frames end in pack {n}");
            };
            if usize::from(f.part) >= parts.len() {
                bail!("a frame of part {}, of {} parts", f.part, parts.len());
            }
            sum += u64::from(f.size);
        }
        if sum != size {
            bail!("the frames of pack {n} take {sum} bytes, the pack {size}");
        }
    }
    if frame.next().is_some() {
        bail!("more frames than the packs hold");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_ids_in_toc_and_bundle_order() {
        let memory: [u8; 16] = std::array::from_fn(|i| i as u8);
        let bundle = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
        assert_eq!(guid_order(memory), bundle);
        let mut toc = memory;
        toc.reverse();
        assert_eq!(toc_chunk_id(toc), bundle);
    }

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
            parts: Vec::new(),
            frames: Vec::new(),
        }
    }

    fn frame(size: u32, part: u8, tier: u8) -> Frame {
        Frame { size, part, tier }
    }

    #[test]
    fn frames_read_back_and_must_fill_the_packs() {
        let mut framed = index(vec![30, 35]);
        framed.parts = vec!["base".into(), "de".into()];
        framed.frames = vec![
            frame(10, 0, plan::tier::TOP),
            frame(20, 0, plan::tier::REST),
            frame(35, 1, plan::tier::WHOLE),
        ];
        assert_eq!(decode(&encode(&framed)), Ok(framed.clone()));
        let mut short = framed.clone();
        short.frames.pop();
        assert!(
            decode(&encode(&short))
                .unwrap_err()
                .0
                .contains("end in pack 1")
        );
        let mut more = framed.clone();
        more.frames.push(frame(1, 0, 0));
        assert!(
            decode(&encode(&more))
                .unwrap_err()
                .0
                .contains("more frames")
        );
        let mut over = framed.clone();
        over.frames[1].size = 25;
        assert!(decode(&encode(&over)).unwrap_err().0.contains("pack 0"));
        let mut unknown = framed.clone();
        unknown.frames[2].part = 2;
        assert!(
            decode(&encode(&unknown))
                .unwrap_err()
                .0
                .contains("of part 2")
        );
    }

    #[test]
    fn a_frame_is_found_by_where_it_starts_in_its_pack() {
        let dir = std::env::temp_dir().join(format!("nfsh-frames-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(pack_name(0)), [0; 30]).unwrap();
        std::fs::write(dir.join(pack_name(1)), [0; 35]).unwrap();
        let mut framed = index(vec![30, 35]);
        framed.parts = vec!["base".into(), "de".into()];
        framed.frames = vec![frame(10, 0, 1), frame(20, 0, 4), frame(35, 1, 0)];
        std::fs::write(dir.join(INDEX), encode(&framed)).unwrap();
        let store = Store::open(&dir).unwrap();
        let starts = store.frame_starts();
        assert_eq!(starts, [(0, 0), (0, 10), (1, 0)]);
        assert_eq!(store.part_at(&starts, at(1, 0, 35)), Some(1));
        assert_eq!(store.part_at(&starts, at(0, 10, 20)), Some(0));
        assert_eq!(store.part_at(&starts, at(0, 5, 5)), None);
        assert_eq!(store.part_sizes()[&(1, 0)], 35);
        std::fs::remove_dir_all(&dir).unwrap();
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
        // Version 2 ended after the superbundles: drop the empty lists after them.
        bytes.truncate(bytes.len() - 12);
        bytes[11] = 2;
        assert_eq!(decode(&bytes), Ok(old));
    }

    #[test]
    fn a_version_3_index_reads_without_parts() {
        let old = index(vec![30, 35]);
        let mut bytes = encode(&old);
        // Version 3 ended after the GUIDs: drop the empty lists after them.
        bytes.truncate(bytes.len() - 8);
        bytes[11] = 3;
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
                parts: Vec::new(),
                frames: Vec::new(),
            }),
        )
        .unwrap();
        let store = Store::open(&dir).unwrap();
        let mut entries = store.entries().unwrap();
        assert_eq!(entries.read(at(0, 0, e)).unwrap(), samples[3]);
        assert_eq!(entries.read(at(0, e, c)).unwrap(), [7; 5000]);
        // Frames back to back read as one entry.
        let both = entries.read(at(0, 0, e + c)).unwrap();
        assert_eq!(both, [samples[3].clone(), vec![7; 5000]].concat());
        // A location that isn't whole frames is an error, as is one past the pack.
        assert!(entries.read(at(0, 0, e + 1)).is_err());
        assert!(entries.read(at(0, 0, e - 1)).is_err());
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
            parts: Vec::new(),
            frames: Vec::new(),
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
        // What it keeps fills the packs, and the parts find what they refer to, but for
        // chunks in the superbundles this one store leaves out.
        assert_eq!(store.parts[0], "base");
        assert_eq!(
            store.frames.iter().map(|f| u64::from(f.size)).sum::<u64>(),
            store.packs.iter().sum::<u64>()
        );
        let parts = check_parts(&store).unwrap();
        let wrong = parts.failures.iter().filter(|f| !f.contains("not listed"));
        assert_eq!(wrong.collect::<Vec<_>>(), Vec::<&String>::new());
        assert!(parts.entries > 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A store of four entries, one a texture-like chunk in two tiers, in parts base,
    /// en and de, and a frame in `others`, copied into installs. The store has an
    /// English `loc` table and no German one.
    #[test]
    fn an_install_keeps_the_chosen_parts_and_tiers() {
        use crate::copy::{Choice, copy};
        let root = std::env::temp_dir().join(format!("nfsh-copy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, dest, all) = (root.join("store"), root.join("install"), root.join("all"));
        std::fs::create_dir_all(&dir).unwrap();
        let data: [&[u8]; 6] = [
            b"ebx of the base",
            b"top mip",
            b"other mips",
            b"english",
            b"german",
            b"a cinematic's line in a language with no table",
        ];
        let mut encoder = FrameEncoder::new(None).unwrap();
        let stored: Vec<Vec<u8>> = data
            .iter()
            .map(|d| encoder.encode(d, false).unwrap())
            .collect();
        let tags = [
            (plan::BASE, plan::tier::WHOLE),
            (plan::BASE, plan::tier::TOP),
            (plan::BASE, plan::tier::REST),
            (1, plan::tier::WHOLE),
            (2, plan::tier::WHOLE),
            (plan::OTHERS, plan::tier::WHOLE),
        ];
        let frames: Vec<Frame> = stored
            .iter()
            .zip(tags)
            .map(|(f, (part, tier))| Frame {
                size: f.len() as u32,
                part,
                tier,
            })
            .collect();
        let pack = stored.concat();
        std::fs::write(dir.join(pack_name(0)), &pack).unwrap();
        let mut offset = 0;
        let mut place = |sizes: &[usize]| {
            let size: usize = sizes.iter().sum();
            let l = at(0, offset, size as u32);
            offset += size as u32;
            l
        };
        let len = |n: usize| stored[n].len();
        let (base, texture) = (place(&[len(0)]), place(&[len(1), len(2)]));
        let (english, german) = (place(&[len(3)]), place(&[len(4)]));
        let mut sb = sample();
        let bundle = &mut sb[0].bundles[0].bundle;
        bundle.ebx[0].location = base;
        bundle.ebx.push(Asset {
            name: "audio/vo/content/en/x".into(),
            sha1: [0; 20],
            original_size: data[3].len() as u32,
            location: english,
        });
        bundle.res.clear();
        bundle.chunks[0].logical_offset = 0;
        bundle.chunks[0].location = texture;
        sb[0].chunks[0].chunk.location = german;
        sb.push(Superbundle {
            name: "Win32/loc/en".into(),
            bundles: Vec::new(),
            chunks: Vec::new(),
        });
        std::fs::write(
            dir.join(INDEX),
            encode(&Index {
                packs: vec![pack.len() as u64],
                dictionary: Vec::new(),
                superbundles: sb,
                partitions: Vec::new(),
                parts: plan::part_names(),
                frames,
            }),
        )
        .unwrap();
        let store = Store::open(&dir).unwrap();
        assert!(copy(&store, &dir, &Choice::default(), &mut |_| {}).is_err());

        let choice = Choice {
            languages: Some(vec!["en".into()]),
            cut: vec![plan::tier::TOP],
        };
        let copied = copy(&store, &dest, &choice, &mut |_| {}).unwrap();
        let names: Vec<&str> = copied.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["base.nfsh", "en.nfsh"]);
        assert_eq!(copied.frames_cut, 3);
        assert_eq!(copied.bytes, (len(0) + len(2) + len(3)) as u64);
        let install = Store::open(&dest).unwrap();
        let mut entries = install.entries().unwrap();
        assert_eq!(entries.read(base).unwrap(), data[0]);
        assert_eq!(entries.read(english).unwrap(), data[3]);
        assert!(entries.read(texture).is_err());
        assert_eq!(entries.read_end(texture).unwrap(), (data[2].to_vec(), true));
        assert_eq!(entries.read_end(german).unwrap(), (Vec::new(), true));
        assert!(install.is_cut(german).unwrap() && !install.is_cut(base).unwrap());
        assert!(copy(&install, &all, &Choice::default(), &mut |_| {}).is_err());

        // German has no table, so its cinematics' lines are in `others`, which is no
        // language of its own.
        let german_only = Choice {
            languages: Some(vec!["de".into()]),
            cut: vec![plan::tier::TOP],
        };
        let copied = copy(&store, &dest, &german_only, &mut |_| {}).unwrap();
        let names: Vec<&str> = copied.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["base.nfsh", "de.nfsh", "others.nfsh"]);
        let others = Choice {
            languages: Some(vec!["others".into()]),
            cut: Vec::new(),
        };
        assert!(copy(&store, &dest, &others, &mut |_| {}).is_err());

        // Keeping everything, every entry reads as from the store.
        let copied = copy(&store, &all, &Choice::default(), &mut |_| {}).unwrap();
        let names: Vec<&str> = copied.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "base.nfsh",
                "base.mip0.nfsh",
                "en.nfsh",
                "de.nfsh",
                "others.nfsh"
            ]
        );
        assert_eq!((copied.frames_cut, copied.bytes), (0, pack.len() as u64));
        let install = Store::open(&all).unwrap();
        let (mut from, mut to) = (store.entries().unwrap(), install.entries().unwrap());
        for l in [base, texture, english, german] {
            assert_eq!(to.read(l).unwrap(), from.read(l).unwrap());
        }
        // A file of the wrong size stops the install opening.
        std::fs::write(all.join("de.nfsh"), b"x").unwrap();
        assert!(Store::open(&all).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The tail of a chunk read from the whole chunk it is the end of.
    #[test]
    fn a_chunks_end_reads_from_the_whole_chunk() {
        let dir = std::env::temp_dir().join(format!("nfsh-tail-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let data: Vec<u8> = (0..0x2_0100u32).map(|i| (i % 251) as u8).collect();
        let mut encoder = FrameEncoder::new(None).unwrap();
        let whole = [
            encoder.encode(&data[..0x1_0000], false).unwrap(),
            encoder.encode(&data[0x1_0000..], false).unwrap(),
        ]
        .concat();
        std::fs::write(dir.join(pack_name(0)), &whole).unwrap();
        let mut sb = sample();
        let location = at(0, 0, whole.len() as u32);
        let bundle = &mut sb[0].bundles[0].bundle;
        bundle.ebx.clear();
        bundle.res.clear();
        let tail = |logical_offset, logical_size| Chunk {
            id: [4; 16],
            sha1: [0; 20],
            logical_offset,
            logical_size,
            location,
        };
        bundle.chunks = vec![tail(0, data.len() as u32)];
        sb[0].chunks.clear();
        let packs = vec![whole.len() as u64];
        std::fs::write(
            dir.join(INDEX),
            encode(&Index {
                packs,
                dictionary: Vec::new(),
                superbundles: sb,
                partitions: Vec::new(),
                parts: Vec::new(),
                frames: Vec::new(),
            }),
        )
        .unwrap();
        let store = Store::open(&dir).unwrap();
        let mut entries = store.entries().unwrap();
        assert_eq!(entries.read_chunk(&tail(0, 0x2_0100)).unwrap(), data);
        // From 0x10100, the whole blocks from 0x10000 to its end.
        assert_eq!(
            entries.read_chunk(&tail(0x1_0100, 0x1_0000)).unwrap(),
            &data[0x1_0000..]
        );
        // A range the chunk doesn't have.
        assert!(entries.read_chunk(&tail(0x3_0000, 100)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
