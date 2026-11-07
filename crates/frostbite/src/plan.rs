//! What a store keeps of an install, decided from one read of every entry: identical
//! content is stored once, a bundle's copy of a chunk's end is read from the whole chunk,
//! a chunk nothing names is left out, a texture's chunk is split into a frame per tier,
//! and every frame is tagged with the part of the game it belongs to.
//!
//! [`Listing`] says what the tables list, [`scan`] what one entry's bytes hold, and
//! [`decide`] what becomes of each entry. None of them touch a file.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hasher;
use std::ops::Range;

use crate::Location;
use crate::ebx::Partition;
use crate::install::Superbundle;
use crate::mesh::MeshSet;
use crate::sb::{bundle_chunk_size, res_type_hash};
use crate::store::{guid_order, toc_chunk_id};
use crate::texture::{self, Texture};

/// The part every install needs, whatever its languages.
pub const BASE: u8 = 0;
/// The languages' parts, numbered from 1 in this order.
pub const LANGUAGES: [&str; 8] = ["en", "de", "es", "fr", "it", "jp", "pl", "ru"];
/// What a wave's name says when it is in `content/` with no folder: English, by what the
/// lines hold.
pub const NO_FOLDER: &str = "content, no language folder";
/// What a cinematic wave's name says: every language is in one asset.
pub const CINEMATIC: &str = "cinematics: every language in one asset";

/// A frame's tier: what a player may leave out. Everything not listed here is
/// [`tier::WHOLE`].
pub mod tier {
    /// All of an entry; nothing in it can be cut.
    pub const WHOLE: u8 = 0;
    /// A texture's top mip, second and third.
    pub const TOP: u8 = 1;
    pub const SECOND: u8 = 2;
    pub const THIRD: u8 = 3;
    /// A texture's other mips, always kept.
    pub const REST: u8 = 4;
    /// A mesh's LOD 0; later LODs follow.
    pub const LOD: u8 = 16;

    pub fn name(tier: u8) -> String {
        match tier {
            WHOLE => "whole".into(),
            TOP => "top mip".into(),
            SECOND => "second mip".into(),
            THIRD => "third mip".into(),
            REST => "other mips".into(),
            lod if lod >= LOD => format!("mesh LOD {}", lod - LOD),
            other => format!("tier {other}"),
        }
    }
}

/// The name of each part, indexed by a frame's part number.
pub fn part_names() -> Vec<String> {
    std::iter::once("base")
        .chain(LANGUAGES)
        .map(str::to_owned)
        .collect()
}

/// A localized wave's language: the path segment after `content`, by how the names go.
pub fn language(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let mut parts = lower.split('/');
    while let Some(part) = parts.next() {
        if part == "content" {
            // `content/<file>`, with no folder for it, is the English of those lines.
            return match (parts.next(), parts.next()) {
                (Some(folder), Some(_)) => folder.to_owned(),
                _ => NO_FOLDER.to_owned(),
            };
        }
    }
    if lower.starts_with("audio/cinematics/") {
        return CINEMATIC.to_owned();
    }
    "other".to_owned()
}

/// The part a localized wave of this name belongs to; a name that isn't in one
/// language's folder stays in the base.
fn wave_part(name: &str) -> u8 {
    let language = language(name);
    let language = if language == NO_FOLDER {
        "en"
    } else {
        language.as_str()
    };
    LANGUAGES
        .iter()
        .position(|l| *l == language)
        .map_or(BASE, |n| n as u8 + 1)
}

/// Finds chunk ids in an entry's bytes.
pub struct Finder {
    ids: HashMap<[u8; 16], u32>,
    /// Whether any id starts with these three bytes: most positions end here.
    first: Vec<bool>,
}

impl Finder {
    pub fn new(ids: &HashMap<[u8; 16], u32>) -> Self {
        let mut both = HashMap::with_capacity(ids.len() * 2);
        let mut first = vec![false; 1 << 24];
        for (&id, &n) in ids {
            for id in [id, guid_order(id)] {
                both.insert(id, n);
                first[usize::from(id[0]) << 16 | usize::from(id[1]) << 8 | usize::from(id[2])] =
                    true;
            }
        }
        Self { ids: both, first }
    }

    /// Every chunk id in `data`, at any offset: records can leave them unaligned.
    pub fn scan(&self, data: &[u8]) -> Vec<u32> {
        let mut found = Vec::new();
        for at in 0..data.len().saturating_sub(15) {
            let w = &data[at..at + 16];
            if self.first[usize::from(w[0]) << 16 | usize::from(w[1]) << 8 | usize::from(w[2])] {
                let id: [u8; 16] = w.try_into().unwrap_or_default();
                if let Some(&n) = self.ids.get(&id) {
                    found.push(n);
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ebx,
    Res(u32),
    /// A chunk stored from its start.
    Whole,
    /// A chunk's end only: a bundle's copy of a texture's small mips.
    Tail,
}

/// One distinct entry the tables list.
#[derive(Debug, Clone)]
pub struct Entry {
    pub location: Location,
    pub kind: Kind,
    /// The chunks listed here, by index in [`Listing::ids`].
    pub chunks: Vec<u32>,
    /// For a tail, the bytes of the chunk it holds: their start and length.
    pub tail: Option<(u32, u32)>,
    /// For an ebx, the part it belongs to if it turns out to be a localized wave.
    pub wave: u8,
    /// For a res, the ebx of the same name, if that name is in a language's folder: a
    /// wave's res names its chunks too, and goes where the wave goes.
    pub twin: Option<Location>,
}

/// Every entry of some superbundles, in archive order.
#[derive(Debug)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// Every chunk id, as bundles list it.
    pub ids: HashMap<[u8; 16], u32>,
    /// By chunk: whether a bundle lists it.
    pub in_bundle: Vec<bool>,
    /// By chunk: the ranges bundles list as tails.
    pub tails: HashMap<u32, Vec<(u32, u32)>>,
}

/// The entry at `location`, listed as `kind`, added if this is its first listing.
fn listed(
    entries: &mut BTreeMap<Location, Entry>,
    location: Location,
    kind: Kind,
    wave: u8,
) -> &mut Entry {
    let e = entries.entry(location).or_insert(Entry {
        location,
        kind,
        chunks: Vec::new(),
        tail: None,
        wave,
        twin: None,
    });
    // A location listed as a whole chunk by anyone is one.
    if e.kind == Kind::Tail && kind == Kind::Whole {
        e.kind = kind;
    }
    e
}

impl Listing {
    pub fn new(superbundles: &[Superbundle]) -> Self {
        let mut entries: BTreeMap<Location, Entry> = BTreeMap::new();
        let mut ids: HashMap<[u8; 16], u32> = HashMap::new();
        let mut in_bundle = Vec::new();
        let mut tails: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for sb in superbundles {
            for b in &sb.bundles {
                let twins: HashMap<&str, Location> = b
                    .bundle
                    .ebx
                    .iter()
                    .filter(|a| wave_part(&a.name) != BASE)
                    .map(|a| (a.name.as_str(), a.location))
                    .collect();
                for a in &b.bundle.ebx {
                    listed(&mut entries, a.location, Kind::Ebx, wave_part(&a.name));
                }
                for r in &b.bundle.res {
                    let twin = twins.get(r.asset.name.as_str()).copied();
                    listed(&mut entries, r.asset.location, Kind::Res(r.res_type), BASE).twin = twin;
                }
                for c in &b.bundle.chunks {
                    let next = ids.len() as u32;
                    let id = *ids.entry(c.id).or_insert(next);
                    in_bundle.resize(ids.len(), false);
                    in_bundle[id as usize] = true;
                    let kind = if c.logical_offset == 0 {
                        Kind::Whole
                    } else {
                        Kind::Tail
                    };
                    let e = listed(&mut entries, c.location, kind, BASE);
                    e.chunks.push(id);
                    if kind == Kind::Tail {
                        let len = bundle_chunk_size(c.logical_offset, c.logical_size) as u32;
                        let range = (c.logical_offset & !0xFFFF, len);
                        e.tail.get_or_insert(range);
                        tails.entry(id).or_default().push(range);
                    }
                }
            }
            for c in &sb.chunks {
                let next = ids.len() as u32;
                let id = *ids.entry(toc_chunk_id(c.chunk.id)).or_insert(next);
                in_bundle.resize(ids.len(), false);
                listed(&mut entries, c.chunk.location, Kind::Whole, BASE)
                    .chunks
                    .push(id);
            }
        }
        let mut entries: Vec<Entry> = entries.into_values().collect();
        for e in &mut entries {
            e.chunks.sort_unstable();
            e.chunks.dedup();
        }
        for ranges in tails.values_mut() {
            ranges.sort_unstable();
            ranges.dedup();
        }
        Self {
            entries,
            ids,
            in_bundle,
            tails,
        }
    }
}

/// A texture's cuts, for a chunk of exactly `bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cuts {
    pub chunk: u32,
    pub at: [u64; 3],
    pub bytes: u64,
}

/// What one entry's bytes hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub raw: u64,
    pub hash: u64,
    /// The part of an ebx that is a localized wave; otherwise the base.
    pub part: u8,
    /// An ebx's partition GUID, and the partitions it refers to.
    pub guid: [u8; 16],
    pub imports: Vec<[u8; 16]>,
    /// Whether an ebx is a `SentenceGroup`, one language's list of its lines.
    pub sentences: bool,
    /// The chunks it names, by index.
    pub names: Vec<u32>,
    pub texture: Option<Cuts>,
    /// The chunk of each LOD, in order, of a mesh set.
    pub lods: Vec<u32>,
    /// For a whole chunk, the hash of each tail range bundles list for it.
    pub ends: Vec<(u32, u32, u64)>,
}

fn hash(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(data);
    hasher.finish()
}

/// Reads one entry's bytes.
pub fn scan(entry: &Entry, data: &[u8], listing: &Listing, finder: &Finder) -> Scan {
    let mut scan = Scan {
        raw: data.len() as u64,
        hash: hash(data),
        ..Scan::default()
    };
    match entry.kind {
        Kind::Ebx => {
            if let Ok(p) = Partition::parse(data) {
                let root = p.instances.first().map(|i| p.type_name(i.type_index));
                if root.as_deref() == Some("LocalizedWaveAsset") {
                    scan.part = entry.wave;
                }
                scan.sentences = root.as_deref() == Some("SentenceGroup");
                scan.guid = p.header.guid;
                scan.imports = p.imports.iter().map(|i| i.partition).collect();
                scan.imports.sort_unstable();
                scan.imports.dedup();
            }
            scan.names = finder.scan(data);
        }
        Kind::Res(texture::TEXTURE) => {
            if let Ok(t) = Texture::parse(data)
                && let Some(&chunk) = listing.ids.get(&t.chunk)
            {
                scan.names = vec![chunk];
                scan.texture = Some(Cuts {
                    chunk,
                    at: t.tier_cuts(),
                    bytes: t.chunk_bytes(),
                });
            }
        }
        Kind::Res(res) => {
            scan.names = finder.scan(data);
            if res == res_type_hash("MeshSet")
                && let Ok(m) = MeshSet::parse(data)
            {
                scan.lods = m
                    .lods
                    .iter()
                    .filter_map(|l| listing.ids.get(&l.chunk?).copied())
                    .collect();
            }
        }
        Kind::Whole | Kind::Tail => {
            for c in &entry.chunks {
                for &(from, len) in listing.tails.get(c).into_iter().flatten() {
                    let end = u64::from(from) + u64::from(len);
                    if end <= scan.raw {
                        let range = from as usize..end as usize;
                        scan.ends.push((from, len, hash(&data[range])));
                    }
                }
            }
        }
    }
    scan
}

/// How a kept entry is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keep {
    /// Whether its frames use the ebx dictionary.
    pub ebx: bool,
    pub part: u8,
    /// Its frames in order: where each ends in the entry, and its tier.
    pub frames: Vec<(u64, u8)>,
}

impl Keep {
    /// The bytes of each frame, with its tier.
    pub fn pieces(&self) -> impl Iterator<Item = (Range<usize>, u8)> + '_ {
        let mut from = 0;
        self.frames.iter().map(move |&(end, tier)| {
            let range = from as usize..end as usize;
            from = end;
            (range, tier)
        })
    }
}

/// What becomes of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fate {
    Keep(Keep),
    /// Identical to this entry, which is kept.
    Same(u32),
    /// A chunk's end, read from the whole chunk at this entry, which is kept.
    InWhole(u32),
    /// Named by nothing, and no bundle lists it.
    Dropped,
}

/// The parts two things that share bytes belong to: both need the bytes, so unless they
/// are one language the bytes go in the base.
fn shared(a: Option<u8>, b: u8) -> Option<u8> {
    match a {
        None => Some(b),
        Some(a) if a == b => Some(a),
        Some(_) => Some(BASE),
    }
}

/// The part each entry needs to be in, by what it is and what refers to it:
///
/// - a localized wave is in its language's part, by its folder;
/// - a `SentenceGroup` listing the waves of one language is in that language's part;
/// - any other ebx that refers to a wave needs it whatever the language, so the wave
///   goes in the base;
/// - a wave's res goes where the wave goes.
fn entry_parts(listing: &Listing, scans: &[Scan]) -> Vec<u8> {
    let mut parts: Vec<u8> = scans.iter().map(|s| s.part).collect();
    let mut by_guid: HashMap<[u8; 16], Vec<usize>> = HashMap::new();
    for (n, e) in listing.entries.iter().enumerate() {
        if e.kind == Kind::Ebx {
            by_guid.entry(scans[n].guid).or_default().push(n);
        }
    }
    let mut to_base = Vec::new();
    for (n, s) in scans.iter().enumerate() {
        if s.part != BASE {
            continue;
        }
        let waves = || {
            s.imports
                .iter()
                .flat_map(|g| by_guid.get(g).into_iter().flatten())
                .copied()
                .filter(|&w| scans[w].part != BASE)
        };
        let mut languages: Vec<u8> = waves().map(|w| scans[w].part).collect();
        languages.sort_unstable();
        languages.dedup();
        match (s.sentences, languages.as_slice()) {
            (true, &[language]) => parts[n] = language,
            _ => to_base.extend(waves()),
        }
    }
    for w in to_base {
        parts[w] = BASE;
    }
    for (n, e) in listing.entries.iter().enumerate() {
        let twin = e.twin.and_then(|l| {
            listing
                .entries
                .binary_search_by_key(&l, |e| e.location)
                .ok()
        });
        if let Some(twin) = twin {
            parts[n] = parts[twin];
        }
    }
    parts
}

/// What becomes of each of `listing`'s entries, given each one's [`Scan`].
pub fn decide(listing: &Listing, scans: &[Scan]) -> Vec<Fate> {
    let chunks = listing.ids.len();
    let part_of = entry_parts(listing, scans);
    let mut named = vec![false; chunks];
    let mut parts: Vec<Option<u8>> = vec![None; chunks];
    let mut cuts: Vec<Option<Cuts>> = vec![None; chunks];
    let mut lods: Vec<Option<usize>> = vec![None; chunks];
    for (s, &part) in scans.iter().zip(&part_of) {
        for &c in s.names.iter().chain(&s.lods) {
            named[c as usize] = true;
            parts[c as usize] = shared(parts[c as usize], part);
        }
        if let Some(t) = s.texture {
            cuts[t.chunk as usize].get_or_insert(t);
        }
        for (lod, &c) in s.lods.iter().enumerate() {
            let at = &mut lods[c as usize];
            *at = Some(at.map_or(lod, |l| l.min(lod)));
        }
    }

    let mut fates: Vec<Option<Fate>> = vec![None; scans.len()];
    // By chunk, the entries holding it whole, none of them left out.
    let mut wholes: Vec<Vec<u32>> = vec![Vec::new(); chunks];
    for (n, e) in listing.entries.iter().enumerate() {
        if e.kind != Kind::Whole {
            continue;
        }
        let kept = |&c: &u32| named[c as usize] || listing.in_bundle[c as usize];
        if e.chunks.iter().any(kept) {
            for &c in &e.chunks {
                wholes[c as usize].push(n as u32);
            }
        } else {
            fates[n] = Some(Fate::Dropped);
        }
    }
    for (n, e) in listing.entries.iter().enumerate() {
        let (Kind::Tail, Some((from, len))) = (e.kind, e.tail) else {
            continue;
        };
        let holder = e.chunks.iter().find_map(|&c| {
            wholes[c as usize].iter().copied().find(|&w| {
                scans[w as usize].ends.contains(&(from, len, scans[n].hash))
                    && scans[n].raw == u64::from(len)
            })
        });
        if let Some(w) = holder {
            fates[n] = Some(Fate::InWhole(w));
        }
    }

    // What each distinct content must be, from everything sharing it.
    #[derive(Default)]
    struct Group {
        first: u32,
        part: Option<u8>,
        ebx: bool,
        cuts: Option<Cuts>,
        lod: Option<usize>,
    }
    let mut groups: HashMap<(u64, u64), Group> = HashMap::new();
    for (n, e) in listing.entries.iter().enumerate() {
        if matches!(fates[n], Some(Fate::Dropped)) {
            continue;
        }
        let s = &scans[n];
        // A tail read from its whole chunk adds nothing but what that chunk's id says,
        // and the whole chunk has the same id.
        if matches!(fates[n], Some(Fate::InWhole(_))) {
            continue;
        }
        let g = groups.entry((s.hash, s.raw)).or_insert_with(|| Group {
            first: n as u32,
            ..Group::default()
        });
        g.ebx |= e.kind == Kind::Ebx;
        match e.kind {
            Kind::Whole | Kind::Tail => {
                for &c in &e.chunks {
                    let part = parts[c as usize].unwrap_or(BASE);
                    g.part = shared(g.part, part);
                    if e.kind == Kind::Whole {
                        let fits = |t: &Cuts| t.bytes == s.raw;
                        g.cuts = g.cuts.or(cuts[c as usize].filter(fits));
                        g.lod = g.lod.or(lods[c as usize]);
                    }
                }
            }
            _ => g.part = shared(g.part, part_of[n]),
        }
    }
    for (n, _) in listing.entries.iter().enumerate() {
        if fates[n].is_some() {
            continue;
        }
        let s = &scans[n];
        let g = &groups[&(s.hash, s.raw)];
        fates[n] = Some(if g.first as usize != n {
            Fate::Same(g.first)
        } else {
            Fate::Keep(Keep {
                ebx: g.ebx,
                part: g.part.unwrap_or(BASE),
                frames: frames_of(s.raw, g.cuts, g.lod),
            })
        });
    }
    // A chunk's end points at the entry that holds it whole, or the one identical to it.
    let resolve: Vec<Option<u32>> = fates
        .iter()
        .map(|f| match f {
            Some(Fate::Same(n)) => Some(*n),
            _ => None,
        })
        .collect();
    fates
        .into_iter()
        .map(|f| match f {
            Some(Fate::InWhole(w)) => Fate::InWhole(resolve[w as usize].unwrap_or(w)),
            Some(f) => f,
            None => Fate::Dropped,
        })
        .collect()
}

/// An entry of `raw` bytes as frames: a frame per tier for a texture's chunk, one
/// otherwise.
fn frames_of(raw: u64, cuts: Option<Cuts>, lod: Option<usize>) -> Vec<(u64, u8)> {
    if let Some(c) = cuts {
        let frames = [tier::TOP, tier::SECOND, tier::THIRD]
            .into_iter()
            .zip(c.at)
            .map(|(tier, end)| (end, tier))
            .chain([(raw, tier::REST)]);
        let mut out: Vec<(u64, u8)> = Vec::new();
        let mut from = 0;
        for (end, tier) in frames {
            if end > from {
                out.push((end, tier));
                from = end;
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    let tier = match lod {
        Some(lod) => tier::LOD.saturating_add(lod.min(200) as u8),
        None => tier::WHOLE,
    };
    vec![(raw, tier)]
}

/// How many entries and how many stored bytes each fate took, as the install stores them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Saved {
    pub entries: u64,
    pub stored: u64,
}

/// What [`decide`] left out, in the install's own stored bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub same: Saved,
    pub in_whole: Saved,
    pub dropped: Saved,
}

pub fn summary(listing: &Listing, fates: &[Fate]) -> Summary {
    let mut out = Summary::default();
    for (e, f) in listing.entries.iter().zip(fates) {
        let saved = match f {
            Fate::Keep(_) => continue,
            Fate::Same(_) => &mut out.same,
            Fate::InWhole(_) => &mut out.in_whole,
            Fate::Dropped => &mut out.dropped,
        };
        saved.entries += 1;
        saved.stored += u64::from(e.location.size);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{BundleEntry, ChunkEntry, Layer};
    use crate::sb::{Asset, Bundle, Chunk, Res};
    use crate::toc::TocChunk;
    use crate::{CasRef, Location};

    fn at(offset: u32) -> Location {
        Location {
            cas: CasRef {
                patch: false,
                package: 0,
                cas: 0,
            },
            offset,
            size: 10,
        }
    }

    fn id(n: u8) -> [u8; 16] {
        [n; 16]
    }

    fn chunk(n: u8, offset: u32, size: u32, location: Location) -> Chunk {
        Chunk {
            id: id(n),
            sha1: [0; 20],
            logical_offset: offset,
            logical_size: size,
            location,
        }
    }

    fn toc(n: u8, location: Location) -> ChunkEntry {
        // A table lists an id with all its bytes reversed, and in memory order.
        let mut id = guid_order(id(n));
        id.reverse();
        ChunkEntry {
            chunk: TocChunk { id, location },
            layer: Layer::Data,
        }
    }

    fn superbundle(
        ebx: &[(&str, Location)],
        chunks: Vec<Chunk>,
        toc: Vec<ChunkEntry>,
    ) -> Superbundle {
        let bundle = Bundle {
            ebx: ebx
                .iter()
                .map(|&(name, location)| Asset {
                    name: name.into(),
                    sha1: [0; 20],
                    original_size: 1,
                    location,
                })
                .collect(),
            res: Vec::new(),
            chunks,
            meta: Vec::new(),
        };
        Superbundle {
            name: "sb".into(),
            bundles: vec![BundleEntry {
                name: "b".into(),
                layer: Layer::Data,
                bundle,
            }],
            chunks: toc,
        }
    }

    /// A scan of a piece of content called `content`, of `raw` bytes.
    fn scan_of(content: u64, raw: u64) -> Scan {
        Scan {
            raw,
            hash: content,
            ..Scan::default()
        }
    }

    fn chunk_index(l: &Listing, n: u8) -> u32 {
        l.ids[&id(n)]
    }

    #[test]
    fn languages_come_from_the_folders() {
        assert_eq!(wave_part("Audio/VO/Cops/Content/DE/line"), 2);
        assert_eq!(wave_part("audio/vo/cops/content/en/line"), 1);
        assert_eq!(wave_part("audio/vo/cops/content/line"), 1);
        assert_eq!(wave_part("audio/vo/cops/content/sfx/line"), BASE);
        assert_eq!(wave_part("audio/cinematics/audioassets/c0/line"), BASE);
        assert_eq!(part_names()[2], "de");
    }

    #[test]
    fn a_chunk_nothing_names_is_dropped_only_when_no_bundle_lists_it() {
        let sb = superbundle(
            &[("a", at(0))],
            vec![chunk(1, 0, 10, at(10))],
            vec![toc(2, at(20)), toc(3, at(30))],
        );
        let l = Listing::new(&[sb]);
        let mut scans = vec![
            scan_of(1, 10),
            scan_of(2, 10),
            scan_of(3, 10),
            scan_of(4, 10),
        ];
        scans[0].names = vec![chunk_index(&l, 3)];
        let fates = decide(&l, &scans);
        assert!(matches!(fates[1], Fate::Keep(_)), "a bundle lists it");
        assert_eq!(fates[2], Fate::Dropped, "no bundle, no name");
        assert!(matches!(fates[3], Fate::Keep(_)), "an ebx names it");
        let s = summary(&l, &fates);
        assert_eq!(
            s.dropped,
            Saved {
                entries: 1,
                stored: 10
            }
        );
    }

    #[test]
    fn identical_bytes_are_kept_once_as_what_any_of_them_needs() {
        let sb = superbundle(
            &[("a", at(0)), ("b", at(10))],
            vec![chunk(1, 0, 10, at(20))],
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        let fates = decide(&l, &[scan_of(7, 10), scan_of(7, 10), scan_of(7, 10)]);
        assert_eq!(fates[1], Fate::Same(0));
        assert_eq!(fates[2], Fate::Same(0));
        let Fate::Keep(k) = &fates[0] else {
            panic!("{:?}", fates[0]);
        };
        assert!(k.ebx);
        // Same size but different content is kept apart.
        let sb = superbundle(&[("a", at(0)), ("b", at(10))], Vec::new(), Vec::new());
        let l = Listing::new(&[sb]);
        let fates = decide(&l, &[scan_of(7, 10), scan_of(8, 10)]);
        assert!(matches!(fates[1], Fate::Keep(_)));
    }

    #[test]
    fn a_tail_equal_to_its_chunks_end_is_read_from_the_whole_chunk() {
        // Entry 0 is a whole chunk, entry 1 identical to it, entries 2 and 3 tails. Tail 3
        // has other bytes, so it stays.
        let sb = superbundle(
            &[],
            vec![
                chunk(1, 0, 200_000, at(0)),
                chunk(1, 0, 200_000, at(10)),
                chunk(1, 0x20000, 1000, at(20)),
                chunk(2, 0x20000, 1000, at(30)),
            ],
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        assert_eq!(l.tails[&chunk_index(&l, 1)], [(0x20000, 1000)]);
        let mut scans = vec![
            scan_of(1, 200_000),
            scan_of(1, 200_000),
            scan_of(5, 1000),
            scan_of(6, 1000),
        ];
        scans[0].ends = vec![(0x20000, 1000, 5)];
        scans[1].ends = vec![(0x20000, 1000, 5)];
        let fates = decide(&l, &scans);
        assert_eq!(fates[1], Fate::Same(0));
        assert_eq!(fates[2], Fate::InWhole(0));
        assert!(matches!(fates[3], Fate::Keep(_)));
    }

    #[test]
    fn a_tail_held_only_by_a_copy_points_at_the_entry_kept() {
        // The tail matches entry 1 alone, which is identical to entry 0.
        let sb = superbundle(
            &[],
            vec![
                chunk(1, 0, 100, at(0)),
                chunk(1, 0, 100, at(10)),
                chunk(1, 0x20000, 1000, at(20)),
            ],
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        let mut scans = vec![scan_of(1, 200_000), scan_of(1, 200_000), scan_of(5, 1000)];
        scans[1].ends = vec![(0x20000, 1000, 5)];
        assert_eq!(decide(&l, &scans)[2], Fate::InWhole(0));
    }

    #[test]
    fn a_textures_chunk_is_split_when_it_is_exactly_its_mips() {
        let sb = superbundle(
            &[("a", at(0))],
            vec![chunk(1, 0, 100, at(10)), chunk(2, 0, 90, at(20))],
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        let cuts = |chunk, bytes| Cuts {
            chunk,
            at: [64, 80, 88],
            bytes,
        };
        let mut scans = vec![scan_of(1, 10), scan_of(2, 100), scan_of(3, 90)];
        scans[0].names = vec![chunk_index(&l, 1), chunk_index(&l, 2)];
        scans[0].texture = Some(cuts(chunk_index(&l, 1), 100));
        let fates = decide(&l, &scans);
        let Fate::Keep(k) = &fates[1] else {
            panic!("{:?}", fates[1]);
        };
        assert_eq!(
            k.frames,
            [
                (64, tier::TOP),
                (80, tier::SECOND),
                (88, tier::THIRD),
                (100, tier::REST)
            ]
        );
        let pieces: Vec<_> = k.pieces().collect();
        assert_eq!(pieces[1], (64..80, tier::SECOND));
        // Not exactly the mips of any texture: one frame.
        let Fate::Keep(k) = &fates[2] else {
            panic!("{:?}", fates[2]);
        };
        assert_eq!(k.frames, [(90, tier::WHOLE)]);
    }

    #[test]
    fn empty_tiers_are_not_written_and_a_mesh_chunk_keeps_its_best_lod() {
        let cuts = Cuts {
            chunk: 0,
            at: [64, 64, 64],
            bytes: 80,
        };
        assert_eq!(
            frames_of(80, Some(cuts), None),
            [(64, tier::TOP), (80, tier::REST)]
        );
        let one = Cuts {
            at: [0, 0, 0],
            ..cuts
        };
        assert_eq!(frames_of(64, Some(one), None), [(64, tier::REST)]);
        assert_eq!(frames_of(5, None, Some(2)), [(5, tier::LOD + 2)]);
        assert_eq!(frames_of(0, None, None), [(0, tier::WHOLE)]);

        let sb = superbundle(
            &[("a", at(0)), ("b", at(10))],
            vec![chunk(1, 0, 9, at(20))],
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        let c = chunk_index(&l, 1);
        let mut scans = vec![scan_of(1, 1), scan_of(2, 1), scan_of(3, 9)];
        scans[0].lods = vec![c, c];
        scans[1].lods = vec![99, 99].into_iter().map(|_| c).collect();
        let Fate::Keep(k) = &decide(&l, &scans)[2] else {
            panic!();
        };
        assert_eq!(k.frames, [(9, tier::LOD)]);
    }

    #[test]
    fn a_language_keeps_only_what_nothing_else_needs() {
        // Ebx 0 and 1 are German and French waves, ebx 2 is not localized. Chunk 1 is
        // named by the German alone, chunk 2 by both languages, chunk 3 by the base too.
        let sb = superbundle(
            &[
                ("audio/vo/content/de/a", at(0)),
                ("audio/vo/content/fr/a", at(10)),
                ("audio/music/a", at(20)),
            ],
            vec![
                chunk(1, 0, 9, at(30)),
                chunk(2, 0, 9, at(40)),
                chunk(3, 0, 9, at(50)),
            ],
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        let c = |n| chunk_index(&l, n);
        let mut scans: Vec<Scan> = (0..6).map(|n| scan_of(n, 9)).collect();
        scans[0].part = 2;
        scans[0].names = vec![c(1), c(2), c(3)];
        scans[1].part = 4;
        scans[1].names = vec![c(2), c(3)];
        scans[2].names = vec![c(3)];
        let fates = decide(&l, &scans);
        let part = |n: usize| match &fates[n] {
            Fate::Keep(k) => k.part,
            other => panic!("{other:?}"),
        };
        assert_eq!([part(0), part(1), part(2)], [2, 4, BASE]);
        assert_eq!([part(3), part(4), part(5)], [2, BASE, BASE]);
    }

    /// As in the game: a wave's ebx and the res of the same name both name its chunk.
    #[test]
    fn a_waves_res_goes_where_the_wave_goes() {
        let mut sb = superbundle(
            &[
                ("audio/vo/content/de/a", at(0)),
                ("audio/vo/content/fr/b", at(10)),
            ],
            vec![chunk(1, 0, 9, at(40))],
            Vec::new(),
        );
        let res = |name: &str, location| Res {
            asset: Asset {
                name: name.into(),
                sha1: [0; 20],
                original_size: 1,
                location,
            },
            res_type: 1,
            meta: [0; 16],
            id: 1,
        };
        sb.bundles[0].bundle.res = vec![res("audio/vo/content/de/a", at(20)), res("other", at(30))];
        let l = Listing::new(&[sb]);
        let c = chunk_index(&l, 1);
        let mut scans: Vec<Scan> = (0..5).map(|n| scan_of(n, 9)).collect();
        scans[0].part = 2;
        scans[0].names = vec![c];
        scans[2].names = vec![c];
        let fates = decide(&l, &scans);
        let part = |n: usize| match &fates[n] {
            Fate::Keep(k) => k.part,
            other => panic!("{other:?}"),
        };
        assert_eq!(part(2), 2, "the res");
        assert_eq!(part(4), 2, "the chunk both name");
        assert_eq!(part(3), BASE, "a res of no wave");
        // The same name on an ebx that turns out not to be a wave stays in the base.
        scans[0].part = BASE;
        assert!(matches!(&decide(&l, &scans)[4], Fate::Keep(k) if k.part == BASE));
    }

    /// A language's list of lines goes with them. Anything else referring to a line
    /// needs it whatever the language, so the line goes in the base with its chunk.
    #[test]
    fn what_refers_to_a_wave_decides_where_it_goes() {
        let sb = superbundle(
            &[
                ("audio/vo/content/de/a", at(0)),
                ("audio/vo/content/en/b", at(10)),
                ("audio/vo/ctxtfiles/de/group", at(20)),
                ("audio/vo/ctxtfiles/mixed", at(30)),
                ("audio/config", at(40)),
            ],
            vec![chunk(1, 0, 9, at(50)), chunk(2, 0, 9, at(60))],
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        let mut scans: Vec<Scan> = (0..7).map(|n| scan_of(n, 9)).collect();
        let (a, b) = ([1; 16], [2; 16]);
        scans[0].part = 2;
        scans[0].guid = a;
        scans[0].names = vec![chunk_index(&l, 1)];
        scans[1].part = 1;
        scans[1].guid = b;
        scans[1].names = vec![chunk_index(&l, 2)];
        scans[2].sentences = true;
        scans[2].imports = vec![a];
        let fates = decide(&l, &scans);
        let parts = |fates: &[Fate]| -> Vec<u8> {
            fates
                .iter()
                .map(|f| match f {
                    Fate::Keep(k) => k.part,
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(parts(&fates), [2, 1, 2, BASE, BASE, 2, 1]);

        // A group of two languages is no one language's: both lines go in the base.
        scans[3].sentences = true;
        scans[3].imports = vec![a, b];
        assert_eq!(
            parts(&decide(&l, &scans)),
            [BASE, BASE, 2, BASE, BASE, BASE, BASE]
        );

        // Nor is a config: its line goes in the base, and the other stays.
        scans[3].sentences = false;
        scans[3].imports.clear();
        scans[4].imports = vec![b];
        assert_eq!(
            parts(&decide(&l, &scans)),
            [2, BASE, 2, BASE, BASE, 2, BASE]
        );
    }

    #[test]
    fn a_language_shares_bytes_with_the_base_by_going_in_the_base() {
        let sb = superbundle(
            &[("audio/vo/content/de/a", at(0)), ("x", at(10))],
            Vec::new(),
            Vec::new(),
        );
        let l = Listing::new(&[sb]);
        let mut scans = vec![scan_of(1, 9), scan_of(1, 9)];
        scans[0].part = 2;
        let Fate::Keep(k) = &decide(&l, &scans)[0] else {
            panic!();
        };
        assert_eq!(k.part, BASE);
    }

    #[test]
    fn a_wave_named_for_a_language_is_only_in_it_if_its_root_says_so() {
        let sb = superbundle(&[("audio/vo/content/de/a", at(0))], Vec::new(), Vec::new());
        let l = Listing::new(&[sb]);
        assert_eq!(l.entries[0].wave, 2);
        let finder = Finder::new(&l.ids);
        // Not a partition at all: it stays in the base.
        assert_eq!(scan(&l.entries[0], b"nope", &l, &finder).part, BASE);
    }
}
