//! `nfsh sizes`: what a store's bytes are, and what streaming them would take.
//!
//! Every distinct entry is read once and counted once, at its stored size, so the
//! tables add up to the packs. A chunk's owner comes from what names it: a texture's
//! header, a mesh set's LODs, or, for anything else, the chunk id found in the bytes of
//! an ebx or res. A chunk nothing names is listed as unnamed. The output is Markdown and
//! holds numbers and type names only.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
use std::hash::{DefaultHasher, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use frostbite::Location;
use frostbite::Result;
use frostbite::ebx::Partition;
use frostbite::install::Layer;
use frostbite::level::{Level, bundle_key};
use frostbite::mesh::MeshSet;
use frostbite::plan::{CINEMATIC, Finder, language};
use frostbite::sb::res_type_hash;
use frostbite::store::{Store, in_parallel, toc_chunk_id};
use frostbite::texture::{self, Texture};

use crate::level::{ROOT, SUPERBUNDLE};

const MIB: f64 = 1024.0 * 1024.0;

/// An entry more than one group of bundles needs.
const MIXED: u32 = u32::MAX;

/// The voice-over folders that are not English.
const OTHER_LANGUAGES: [&str; 7] = ["de", "es", "fr", "it", "jp", "pl", "ru"];

/// Cars at their fastest, about 400 km/h, in km/s: what turns MiB/km into MiB/s.
const TOP_SPEED: f64 = 0.111;

/// Res type names tried against the hashes in the store; an unmatched hash prints as hex.
const RES_TYPES: &[&str] = &[
    "Texture",
    "MeshSet",
    "SwfMovie",
    "ShaderProgramDatabase",
    "ShaderBlockDepot",
    "ShaderDatabase",
    "ByteCode",
    "AtlasTexture",
    "ClothAsset",
    "ClothWrappingAsset",
    "RawFileData",
    "LocalizedStringResource",
    "Luts",
    "AssetBank",
    "EAClothEntityData",
    "EAClothAssetData",
    "EAClothData",
    "HavokPhysicsData",
    "GroupRigidBodyData",
    "RigidBodyData",
    "SkinnedMeshData",
    "AnimTrackData",
    "IesResource",
    "Dx11ShaderProgramDatabase",
    "EnlightenDatabase",
    "EnlightenSystem",
    "EnlightenProbeSet",
    "EnlightenStaticDatabase",
    "EnlightenShaderDatabase",
    "EnlightenLightProbeSet",
    "OccluderMesh",
    "PamProgram",
    "Terrain",
    "TerrainQuadTree",
    "HeightfieldData",
    "MorphResource",
    "MeshSetPVS",
    "SmartPhysicsData",
    "NavMesh",
    "AiNavMesh",
    "Headmorph",
    "TerrainLayerCombinations",
    "Dx12ShaderDatabase",
    "StreamingStall",
    "GtsSpline",
    "NewWaveResource",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Ebx,
    Res(u32),
    /// A chunk stored from its start.
    Whole,
    /// A chunk's end only: a bundle's copy of a texture's small mips.
    Tail,
}

/// One distinct entry of the packs.
struct Entry {
    location: Location,
    kind: Kind,
    /// The chunk it belongs to, for [`Kind::Whole`] and [`Kind::Tail`].
    chunk: Option<u32>,
    /// The superbundle whose table lists it as a chunk with no bundle, if one does.
    toc: Option<u32>,
}

/// What reading an entry finds.
struct Scan {
    raw: u64,
    hash: u64,
    /// The first bytes, which tell an unnamed chunk's format apart.
    magic: [u8; 4],
    /// An ebx's first instance's type, with the language of a localized one.
    root: Option<String>,
    /// The chunks it names, by index.
    names: Vec<u32>,
    texture: Option<TextureInfo>,
    /// The chunk of each LOD, in order, of a mesh set.
    lods: Vec<u32>,
}

#[derive(Clone)]
struct TextureInfo {
    chunk: u32,
    format: String,
    side: u32,
    /// The chunk's bytes; the largest mip's, all layers; those only the whole chunk has.
    bytes: u64,
    top: u64,
    streamed: u64,
    area: String,
}

/// Who a chunk belongs to.
#[derive(Clone)]
enum Owner {
    Texture(TextureInfo),
    Mesh { lod: usize },
    Named(String),
}

#[derive(Default, Clone, Copy)]
struct Sum {
    count: u64,
    raw: u64,
    stored: u64,
}

impl Sum {
    fn add(&mut self, raw: u64, stored: u64) {
        self.count += 1;
        self.raw += raw;
        self.stored += stored;
    }
}

fn gib(n: u64) -> String {
    format!("{:.2} GiB", n as f64 / (MIB * 1024.0))
}

fn mib(n: u64) -> String {
    format!("{:.1} MiB", n as f64 / MIB)
}

fn table(out: &mut String, title: &str, head: &str, sums: &BTreeMap<String, Sum>, total: u64) {
    let _ = writeln!(out, "\n## {title}\n");
    let _ = writeln!(out, "| {head} | Entries | Decompressed | Stored | Share |");
    let _ = writeln!(out, "|---|---:|---:|---:|---:|");
    let mut rows: Vec<_> = sums.iter().collect();
    rows.sort_by_key(|(_, s)| std::cmp::Reverse(s.stored));
    for (name, s) in rows {
        let _ = writeln!(
            out,
            "| {name} | {} | {} | {} | {:.1}% |",
            s.count,
            mib(s.raw),
            mib(s.stored),
            s.stored as f64 * 100.0 / total as f64
        );
    }
}

/// The first path segment of an asset's name, and the second where the first says little.
fn area(name: &str) -> String {
    let mut parts = name.split('/');
    let first = parts.next().unwrap_or("");
    match (first, parts.next()) {
        ("levels" | "ui" | "characters" | "vehicles", Some(second)) => format!("{first}/{second}"),
        _ => first.to_owned(),
    }
}

/// A bundle's first two path segments.
fn group(key: &str) -> String {
    key.split('/').take(2).collect::<Vec<_>>().join("/")
}

/// Whether `same` holds for each pair of entries' bytes.
fn compare(
    store: &Store,
    entries: &[Entry],
    pairs: &[(u32, u32)],
    same: impl Fn(&[u8], &[u8]) -> bool + Sync,
) -> Result<Vec<bool>> {
    in_parallel(
        pairs,
        || store.entries(),
        |reader, &(a, b)| {
            let x = reader.read(entries[a as usize].location)?;
            let y = reader.read(entries[b as usize].location)?;
            Ok(same(&x, &y))
        },
    )
}

/// The name of a bundle with its numbers replaced, so a zone's variants group together.
fn pattern(name: &str) -> String {
    let mut out = String::new();
    let mut in_number = false;
    for c in name.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                out.push('#');
            }
            in_number = true;
        } else {
            out.push(c);
            in_number = false;
        }
    }
    out
}

/// The entries a bundle needs: its own, the whole chunks it lists, what those name.
fn closure(
    bundle: &frostbite::sb::Bundle,
    at: &HashMap<Location, u32>,
    ids: &HashMap<[u8; 16], u32>,
    whole: &[Option<u32>],
    scans: &[Scan],
) -> Vec<u32> {
    let mut set = Vec::new();
    let entry = |set: &mut Vec<u32>, l: Location| {
        let n = at[&l];
        set.push(n);
        for &c in &scans[n as usize].names {
            set.extend(whole[c as usize]);
        }
    };
    bundle.ebx.iter().for_each(|a| entry(&mut set, a.location));
    bundle
        .res
        .iter()
        .for_each(|r| entry(&mut set, r.asset.location));
    for c in &bundle.chunks {
        // The whole chunk replaces the bundle's copy of its end, when there is one.
        match whole[ids[&c.id] as usize] {
            Some(n) => set.push(n),
            None => set.push(at[&c.location]),
        }
    }
    set.sort_unstable();
    set.dedup();
    set
}

/// A place in the city that streams together.
struct Site {
    at: [f32; 2],
    entries: Vec<u32>,
    stored: u64,
    raw: u64,
}

/// Drives straight lines across the city's box, loading every site within `radius`:
/// what is new when a site comes into range, unless a site still in range or `resident`
/// holds it. Returns MiB per km (mean of the lines; the worst line), and the worst MiB
/// in any `window` metres of any line, per km.
fn drive(
    sites: &[Site],
    sizes: &[u64],
    radius: f32,
    window: f32,
    lead: f32,
    skip: &HashSet<u32>,
) -> [f64; 4] {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for s in sites {
        for i in 0..2 {
            lo[i] = lo[i].min(s.at[i]);
            hi[i] = hi[i].max(s.at[i]);
        }
    }
    const STEP: f32 = 25.0;
    let mut lines = Vec::new();
    for i in 1..8 {
        let t = i as f32 / 8.0;
        lines.push((
            [lo[0], lo[1] + (hi[1] - lo[1]) * t],
            [hi[0], lo[1] + (hi[1] - lo[1]) * t],
        ));
        lines.push((
            [lo[0] + (hi[0] - lo[0]) * t, lo[1]],
            [lo[0] + (hi[0] - lo[0]) * t, hi[1]],
        ));
        // Diagonals, shifted across the box.
        lines.push((
            [
                lo[0] + (hi[0] - lo[0]) * (t - 0.5).max(0.0) * 2.0,
                lo[1] + (hi[1] - lo[1]) * (0.5 - t).max(0.0) * 2.0,
            ],
            [
                hi[0] - (hi[0] - lo[0]) * (0.5 - t).max(0.0) * 2.0,
                hi[1] - (hi[1] - lo[1]) * (t - 0.5).max(0.0) * 2.0,
            ],
        ));
    }
    let (mut sum_km, mut sum_bytes, mut worst_line, mut worst_window) =
        (0.0, 0.0, 0.0_f64, 0.0_f64);
    let mut rate = 0.0_f64;
    let per = (window / STEP).ceil() as usize;
    for (from, to) in lines {
        let length = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
        let steps = (length / STEP) as usize;
        let mut held: HashMap<u32, u32> = HashMap::new();
        let mut inside = vec![false; sites.len()];
        let mut loaded = Vec::with_capacity(steps + 1);
        for step in 0..=steps {
            let t = step as f32 * STEP / length;
            let p = [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ];
            let mut bytes = 0;
            let mut leaving = Vec::new();
            for (n, s) in sites.iter().enumerate() {
                let d = ((s.at[0] - p[0]).powi(2) + (s.at[1] - p[1]).powi(2)).sqrt();
                if d <= radius && !inside[n] {
                    inside[n] = true;
                    for &e in &s.entries {
                        if skip.contains(&e) {
                            continue;
                        }
                        let c = held.entry(e).or_insert(0);
                        if *c == 0 {
                            bytes += sizes[e as usize];
                        }
                        *c += 1;
                    }
                } else if d > radius && inside[n] {
                    inside[n] = false;
                    leaving.push(n);
                }
            }
            for n in leaving {
                for &e in &sites[n].entries {
                    if let Some(c) = held.get_mut(&e) {
                        *c = c.saturating_sub(1);
                    }
                }
            }
            // The first steps fill the window from nothing: that is a loading screen.
            loaded.push(if f64::from(step as f32 * STEP) < f64::from(radius) * 2.0 {
                0
            } else {
                bytes
            });
        }
        let total: u64 = loaded.iter().sum();
        let km = f64::from(length - radius * 2.0).max(0.1) / 1000.0;
        sum_km += km;
        sum_bytes += total as f64;
        worst_line = worst_line.max(total as f64 / MIB / km);
        for w in loaded.windows(per.max(1)) {
            let bytes: u64 = w.iter().sum();
            worst_window = worst_window.max(bytes as f64 / MIB / (f64::from(window) / 1000.0));
        }
        // Over any stretch of steps, the bytes due in it were readable from `lead` before
        // it began: their time at top speed sets the rate needed.
        let mut sum = vec![0u64; loaded.len() + 1];
        for (i, &b) in loaded.iter().enumerate() {
            sum[i + 1] = sum[i] + b;
        }
        for from in 0..loaded.len() {
            for to in from + 1..=loaded.len() {
                let bytes = sum[to] - sum[from];
                if bytes == 0 {
                    continue;
                }
                let metres = f64::from(lead) + (to - from - 1) as f64 * f64::from(STEP);
                let seconds = metres / (TOP_SPEED * 1000.0);
                if seconds > 0.0 {
                    rate = rate.max(bytes as f64 / MIB / seconds);
                }
            }
        }
    }
    [sum_bytes / MIB / sum_km, worst_line, worst_window, rate]
}

pub fn run(dir: &Path) -> Result<String> {
    let store = Store::open(dir)?;
    let mut out = String::new();

    // Every distinct entry, and every chunk id with its whole copy.
    let mut at: HashMap<Location, u32> = HashMap::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut ids: HashMap<[u8; 16], u32> = HashMap::new();
    let mut add = |entries: &mut Vec<Entry>, l: Location, kind, chunk, toc| {
        let n = *at.entry(l).or_insert_with(|| {
            entries.push(Entry {
                location: l,
                kind,
                chunk,
                toc,
            });
            (entries.len() - 1) as u32
        });
        let e = &mut entries[n as usize];
        // A location listed as a whole chunk by anyone is one.
        if e.kind == Kind::Tail && kind == Kind::Whole {
            e.kind = kind;
        }
        // Only a chunk no bundle lists keeps a table's name.
        if toc.is_none() {
            e.toc = None;
        }
    };
    for sb in &store.superbundles {
        for b in &sb.bundles {
            for a in &b.bundle.ebx {
                add(&mut entries, a.location, Kind::Ebx, None, None);
            }
            for r in &b.bundle.res {
                add(
                    &mut entries,
                    r.asset.location,
                    Kind::Res(r.res_type),
                    None,
                    None,
                );
            }
            for c in &b.bundle.chunks {
                let next = ids.len() as u32;
                let id = *ids.entry(c.id).or_insert(next);
                let kind = if c.logical_offset == 0 {
                    Kind::Whole
                } else {
                    Kind::Tail
                };
                add(&mut entries, c.location, kind, Some(id), None);
            }
        }
    }
    for (s, sb) in store.superbundles.iter().enumerate() {
        for c in &sb.chunks {
            let next = ids.len() as u32;
            let id = *ids.entry(toc_chunk_id(c.chunk.id)).or_insert(next);
            // Bundles came first: a location one of them lists keeps no table.
            add(
                &mut entries,
                c.chunk.location,
                Kind::Whole,
                Some(id),
                Some(s as u32),
            );
        }
    }
    let finder = Finder::new(&ids);
    let res_names: HashMap<u32, &str> = RES_TYPES.iter().map(|n| (res_type_hash(n), *n)).collect();
    let mut names: HashMap<Location, &str> = HashMap::new();
    for b in store.superbundles.iter().flat_map(|sb| &sb.bundles) {
        for a in &b.bundle.ebx {
            names.insert(a.location, &a.name);
        }
        for r in &b.bundle.res {
            names.insert(r.asset.location, &r.asset.name);
        }
    }
    eprintln!("{} entries, {} chunk ids", entries.len(), ids.len());

    // Read every entry once, in pack order.
    let mut order: Vec<u32> = (0..entries.len() as u32).collect();
    order.sort_by_key(|&n| entries[n as usize].location);
    let done = AtomicUsize::new(0);
    let plan: Vec<(Location, Kind)> = order
        .iter()
        .map(|&n| (entries[n as usize].location, entries[n as usize].kind))
        .collect();
    let ordered = in_parallel(
        &plan,
        || store.entries(),
        |reader, &(location, kind)| {
            let n = done.fetch_add(1, Ordering::Relaxed);
            if n.is_multiple_of(100_000) {
                eprintln!("{n} entries read");
            }
            let data = reader.read(location)?;
            let mut hasher = DefaultHasher::new();
            hasher.write(&data);
            let mut scan = Scan {
                raw: data.len() as u64,
                hash: hasher.finish(),
                magic: std::array::from_fn(|i| data.get(i).copied().unwrap_or(0)),
                root: None,
                names: Vec::new(),
                texture: None,
                lods: Vec::new(),
            };
            match kind {
                Kind::Ebx => {
                    if let Ok(p) = Partition::parse(&data) {
                        scan.root = p.instances.first().map(|i| {
                            let ty = p.type_name(i.type_index);
                            match (ty.as_str(), names.get(&location)) {
                                ("LocalizedWaveAsset", Some(name)) => {
                                    format!("{ty} {}", language(name))
                                }
                                _ => ty,
                            }
                        });
                    }
                    scan.names = finder.scan(&data);
                }
                Kind::Res(texture::TEXTURE) => {
                    if let Ok(t) = Texture::parse(&data)
                        && let Some(&chunk) = ids.get(&t.chunk)
                    {
                        scan.names = vec![chunk];
                        scan.texture = Some(texture_info(&t, chunk, names.get(&location)));
                    }
                }
                Kind::Res(_) => {
                    scan.names = finder.scan(&data);
                    if let Ok(m) = MeshSet::parse(&data) {
                        scan.lods = m
                            .lods
                            .iter()
                            .filter_map(|l| ids.get(&l.chunk?).copied())
                            .collect();
                    }
                }
                Kind::Whole | Kind::Tail => {}
            }
            Ok(scan)
        },
    )?;
    let mut slots: Vec<Option<Scan>> = (0..entries.len()).map(|_| None).collect();
    for (&n, scan) in order.iter().zip(ordered) {
        slots[n as usize] = Some(scan);
    }
    let Some(scans) = slots.into_iter().collect::<Option<Vec<Scan>>>() else {
        return Err(frostbite::Error("an entry was not read".into()));
    };

    // Who owns each chunk: a texture, then a mesh, then whatever else names it.
    let mut owners: Vec<Option<Owner>> = vec![None; ids.len()];
    for scan in &scans {
        if let Some(t) = &scan.texture {
            owners[t.chunk as usize] = Some(Owner::Texture(t.clone()));
        }
    }
    for scan in &scans {
        for (lod, &c) in scan.lods.iter().enumerate() {
            let owner = &mut owners[c as usize];
            if owner.is_none() {
                *owner = Some(Owner::Mesh { lod });
            }
        }
    }
    for pass in [true, false] {
        for (e, scan) in entries.iter().zip(&scans) {
            let label = match e.kind {
                Kind::Ebx if pass => scan.root.clone().unwrap_or_else(|| "ebx".into()),
                Kind::Res(t) if !pass => res_label(t, &res_names),
                _ => continue,
            };
            for &c in &scan.names {
                let owner = &mut owners[c as usize];
                if owner.is_none() {
                    *owner = Some(Owner::Named(label.clone()));
                }
            }
        }
    }

    // The tables.
    let total: u64 = entries.iter().map(|e| u64::from(e.location.size)).sum();
    let raw_total: u64 = scans.iter().map(|s| s.raw).sum();
    let _ = writeln!(
        out,
        "# What a store holds\n\n{} entries, {} decompressed, {} stored; index {}.",
        entries.len(),
        gib(raw_total),
        gib(total),
        mib(std::fs::metadata(dir.join(frostbite::store::INDEX)).map_or(0, |m| m.len())),
    );

    let mut kinds: BTreeMap<String, Sum> = BTreeMap::new();
    let mut formats: BTreeMap<String, Sum> = BTreeMap::new();
    let mut sides: BTreeMap<String, Sum> = BTreeMap::new();
    let mut areas: BTreeMap<String, Sum> = BTreeMap::new();
    let mut lods: BTreeMap<String, Sum> = BTreeMap::new();
    let mut unnamed: BTreeMap<String, Sum> = BTreeMap::new();
    let (mut top_bytes, mut top_big, mut texture_stored) = (0.0_f64, 0.0_f64, 0_u64);
    let (mut tails, mut tail_ok_candidates) = (Sum::default(), Vec::new());
    let mut whole: Vec<Option<u32>> = vec![None; ids.len()];
    for (n, e) in entries.iter().enumerate() {
        if e.kind == Kind::Whole
            && let Some(c) = e.chunk
        {
            let slot = &mut whole[c as usize];
            // The largest copy, if a patch left two.
            if slot.is_none_or(|w| scans[w as usize].raw < scans[n].raw) {
                *slot = Some(n as u32);
            }
        }
    }
    for (n, (e, s)) in entries.iter().zip(&scans).enumerate() {
        let stored = u64::from(e.location.size);
        let kind = match e.kind {
            Kind::Ebx => "ebx".to_owned(),
            Kind::Res(t) => format!("res {}", res_label(t, &res_names)),
            Kind::Whole | Kind::Tail => {
                let c = e.chunk.unwrap_or(0) as usize;
                if e.kind == Kind::Tail {
                    tails.add(s.raw, stored);
                    if let Some(w) = whole[c] {
                        tail_ok_candidates.push((n as u32, w));
                    }
                }
                match &owners[c] {
                    Some(Owner::Texture(t)) => {
                        formats
                            .entry(t.format.clone())
                            .or_default()
                            .add(s.raw, stored);
                        let side = match t.side {
                            0..=256 => "up to 256",
                            257..=512 => "512",
                            513..=1024 => "1024",
                            1025..=2048 => "2048",
                            2049..=4096 => "4096",
                            _ => "over 4096",
                        };
                        sides.entry(side.into()).or_default().add(s.raw, stored);
                        areas.entry(t.area.clone()).or_default().add(s.raw, stored);
                        if e.kind == Kind::Whole {
                            let share = t.top as f64 / t.bytes.max(1) as f64;
                            top_bytes += stored as f64 * share;
                            if t.side > 2048 {
                                top_big += stored as f64 * share;
                            }
                        }
                        texture_stored += stored;
                        if e.kind == Kind::Tail {
                            "chunk: texture, a bundle's copy of the small mips".to_owned()
                        } else {
                            "chunk: texture".to_owned()
                        }
                    }
                    Some(Owner::Mesh { lod }) => {
                        lods.entry(format!("LOD {lod}"))
                            .or_default()
                            .add(s.raw, stored);
                        "chunk: mesh".to_owned()
                    }
                    Some(Owner::Named(label)) => format!("chunk: {label}"),
                    None => {
                        let what = match s.magic {
                            [0x1A, 0x45, 0xDF, 0xA3] => "WebM video",
                            [0x48, 0, 0, 0x0C] => "EA audio",
                            _ if s.raw.is_multiple_of(8) && s.raw > 0 => "other, block-sized",
                            _ => "other",
                        };
                        let layer = match toc_layer(&store, e) {
                            Some(Layer::Patch) => "`Patch/`",
                            Some(Layer::Data) => "`Data/`",
                            None => "a bundle",
                        };
                        unnamed
                            .entry(format!("{what}, from {layer}"))
                            .or_default()
                            .add(s.raw, stored);
                        "chunk: named by nothing read".to_owned()
                    }
                }
            }
        };
        kinds.entry(kind).or_default().add(s.raw, stored);
    }
    table(&mut out, "By kind", "Kind", &kinds, total);
    table(&mut out, "Textures by format", "Format", &formats, total);
    table(&mut out, "Textures by largest side", "Side", &sides, total);
    table(&mut out, "Textures by area", "Area", &areas, total);
    let _ = writeln!(
        out,
        "\nTextures store {}. Their largest mips, by share of raw bytes: {} of it \
         ({} on textures over 2048).",
        gib(texture_stored),
        gib(top_bytes as u64),
        gib(top_big as u64),
    );
    table(&mut out, "Meshes by LOD", "LOD", &lods, total);
    table(
        &mut out,
        "Chunks nothing names",
        "What, from where",
        &unnamed,
        total,
    );

    // Identical content stored more than once, checked byte for byte.
    let mut first: HashMap<(u64, u64), u32> = HashMap::new();
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for (n, s) in scans.iter().enumerate() {
        match first.entry((s.hash, s.raw)) {
            std::collections::hash_map::Entry::Occupied(o) => pairs.push((n as u32, *o.get())),
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(n as u32);
            }
        }
    }
    let same_bytes = compare(&store, &entries, &pairs, |a, b| a == b)?;
    let mut removable = vec![false; entries.len()];
    let mut same: BTreeMap<String, Sum> = BTreeMap::new();
    let mut differ = 0;
    for (&(n, _), ok) in pairs.iter().zip(same_bytes) {
        let n = n as usize;
        if !ok {
            differ += 1;
            continue;
        }
        removable[n] = true;
        let kind = match entries[n].kind {
            Kind::Ebx => "ebx".to_owned(),
            Kind::Res(t) => format!("res {}", res_label(t, &res_names)),
            Kind::Whole => "chunk from its start".to_owned(),
            Kind::Tail => "chunk end".to_owned(),
        };
        same.entry(kind)
            .or_default()
            .add(scans[n].raw, u64::from(entries[n].location.size));
    }
    table(
        &mut out,
        "Lossless: identical to an earlier entry",
        "Kind",
        &same,
        total,
    );
    let _ = writeln!(out, "\nSame hash and size, different bytes: {differ}.");

    // Whether each bundle's copy of a chunk's end is the whole chunk's end.
    let tail_same = compare(&store, &entries, &tail_ok_candidates, |a, b| b.ends_with(a))?;
    let mut copies = Sum::default();
    let mut tail_copy = vec![false; entries.len()];
    for (&(tail, _), ok) in tail_ok_candidates.iter().zip(tail_same) {
        if ok {
            tail_copy[tail as usize] = true;
            copies.add(
                scans[tail as usize].raw,
                u64::from(entries[tail as usize].location.size),
            );
            removable[tail as usize] = true;
        }
    }
    let _ = writeln!(
        out,
        "\nChunk ends stored alone: {}, {} stored; of which exactly the end of a whole chunk \
         also stored: {}, {} stored.",
        tails.count,
        mib(tails.stored),
        copies.count,
        mib(copies.stored),
    );
    let gone: u64 = entries
        .iter()
        .zip(&removable)
        .filter(|(_, r)| **r)
        .map(|(e, _)| u64::from(e.location.size))
        .sum();
    let _ = writeln!(
        out,
        "\nBoth together (an entry counted once): {} stored.",
        gib(gone)
    );

    // What is left after each saving, in turn, with no byte counted twice: identical
    // entries keep one copy for as long as any of them is still needed.
    let label = |n: usize| match entries[n].chunk.map(|c| &owners[c as usize]) {
        Some(Some(Owner::Named(l))) => Some(l.as_str()),
        _ => None,
    };
    let unnamed_chunk =
        |n: usize| matches!(entries[n].chunk.map(|c| &owners[c as usize]), Some(None));
    let other_language = |n: usize| {
        label(n).is_some_and(|l| {
            OTHER_LANGUAGES
                .iter()
                .any(|lang| l == format!("LocalizedWaveAsset {lang}"))
        })
    };
    let left = |needed: &dyn Fn(usize) -> bool| -> (u64, u64) {
        let mut copies: HashMap<(u64, u64), (bool, u64, u64)> = HashMap::new();
        for n in 0..entries.len() {
            let c = copies
                .entry((scans[n].hash, scans[n].raw))
                .or_insert((false, u64::MAX, 0));
            c.0 |= needed(n);
            let stored = u64::from(entries[n].location.size);
            if stored < c.1 {
                c.1 = stored;
                let cinematic = label(n).is_some_and(|l| l.ends_with(CINEMATIC));
                c.2 = if cinematic { stored } else { 0 };
            }
        }
        copies
            .values()
            .filter(|c| c.0)
            .fold((0, 0), |(all, cinematic), c| (all + c.1, cinematic + c.2))
    };
    let (once, _) = left(&|n| !tail_copy[n]);
    let (named, _) = left(&|n| !tail_copy[n] && !unnamed_chunk(n));
    let (one_language, cinematic) =
        left(&|n| !tail_copy[n] && !unnamed_chunk(n) && !other_language(n));
    let _ = writeln!(out, "\n## What is left after each saving, in turn\n");
    let _ = writeln!(out, "| After | Packs |\n|---|---:|");
    for (what, bytes) in [
        ("nothing: the store now", total),
        ("each content stored once, no copied chunk ends", once),
        ("and no chunk nothing names", named),
        (
            "and voice-over in folders of one language, English",
            one_language,
        ),
        (
            "and 7/8 of the cinematic audio, if its 8 chunks are 8 languages",
            one_language - cinematic / 8 * 7,
        ),
    ] {
        let _ = writeln!(out, "| {what} | {} |", gib(bytes));
    }

    // What bundles of each name pattern need.
    let mut patterns: BTreeMap<String, (u64, HashSet<u32>)> = BTreeMap::new();
    let mut by_name: HashMap<String, Vec<u32>> = HashMap::new();
    let mut group_of: Vec<Option<u32>> = vec![None; entries.len()];
    let mut groups: Vec<String> = Vec::new();
    for sb in &store.superbundles {
        for b in &sb.bundles {
            let set = closure(&b.bundle, &at, &ids, &whole, &scans);
            let p = patterns.entry(pattern(&b.name)).or_default();
            p.0 += 1;
            p.1.extend(set.iter().copied());
            let key = bundle_key(&b.name);
            let group = group(&key);
            let g = match groups.iter().position(|x| *x == group) {
                Some(g) => g,
                None => {
                    groups.push(group);
                    groups.len() - 1
                }
            } as u32;
            for &e in &set {
                let slot = &mut group_of[e as usize];
                *slot = match *slot {
                    None => Some(g),
                    Some(o) if o == g => Some(g),
                    Some(_) => Some(MIXED),
                };
            }
            by_name.entry(key).or_default().extend(set);
        }
    }
    let sizes: Vec<u64> = entries.iter().map(|e| u64::from(e.location.size)).collect();
    let mut rows: Vec<(String, u64, u64)> = patterns
        .into_iter()
        .map(|(p, (n, set))| {
            let bytes = set.iter().map(|&e| sizes[e as usize]).sum();
            (p, n, bytes)
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.2));
    let _ = writeln!(
        out,
        "\n## Bundles by name pattern (what they need, stored)\n"
    );
    let _ = writeln!(out, "| Pattern | Bundles | Need |");
    let _ = writeln!(out, "|---|---:|---:|");
    for (p, n, bytes) in rows.iter().take(60) {
        let _ = writeln!(out, "| `{p}` | {n} | {} |", mib(*bytes));
    }

    let mut only: BTreeMap<String, Sum> = BTreeMap::new();
    for (n, slot) in group_of.iter().enumerate() {
        let label = match *slot {
            None => "no bundle",
            Some(MIXED) => "several groups",
            Some(g) => groups[g as usize].as_str(),
        };
        only.entry(label.to_owned())
            .or_default()
            .add(scans[n].raw, sizes[n]);
    }
    only.retain(|_, s| s.stored > 8 << 20);
    table(
        &mut out,
        "What only one group of bundles needs, over 8 MiB",
        "Group",
        &only,
        total,
    );

    // A texture's stored bytes without the mips only the whole chunk holds.
    let mut lean = sizes.clone();
    for (n, e) in entries.iter().enumerate() {
        if let (Kind::Whole, Some(c)) = (e.kind, e.chunk)
            && let Some(Owner::Texture(t)) = &owners[c as usize]
        {
            let keep = 1.0 - t.streamed as f64 / t.bytes.max(1) as f64;
            lean[n] = (sizes[n] as f64 * keep) as u64;
        }
    }
    streaming(&mut out, &store, &by_name, &sizes, &lean, &scans)?;
    Ok(out)
}

/// The layer of the table entry of a chunk no bundle lists.
fn toc_layer(store: &Store, e: &Entry) -> Option<Layer> {
    let sb = store.superbundles.get(e.toc? as usize)?;
    let c = sb.chunks.iter().find(|c| c.chunk.location == e.location)?;
    Some(c.layer)
}

fn res_label(t: u32, names: &HashMap<u32, &str>) -> String {
    names
        .get(&t)
        .map_or_else(|| format!("{t:#010x}"), |n| (*n).to_owned())
}

fn texture_info(t: &Texture, chunk: u32, name: Option<&&str>) -> TextureInfo {
    let bytes = t.chunk_bytes();
    let top = u64::from(t.mip_sizes.first().copied().unwrap_or(0))
        * match t.kind {
            texture::Kind::Volume => 1,
            _ => u64::from(t.layers),
        };
    TextureInfo {
        chunk,
        format: match t.format() {
            Some(f) => f.name.to_owned(),
            None => format!("{:#04x}", t.format),
        },
        side: u32::from(t.width.max(t.height)),
        bytes,
        top,
        streamed: t.tail_offset(),
        area: name.map_or_else(String::new, |n| area(n)),
    }
}

/// MiB per zone, MiB per km and what is always resident.
fn streaming(
    out: &mut String,
    store: &Store,
    by_name: &HashMap<String, Vec<u32>>,
    sizes: &[u64],
    lean: &[u64],
    scans: &[Scan],
) -> Result<()> {
    let mut level = Level::new(store, SUPERBUNDLE)?;
    let subs = level.sub_levels(ROOT)?;
    let raw = |set: &[u32]| -> u64 { set.iter().map(|&e| scans[e as usize].raw).sum() };
    let bytes = |set: &[u32]| -> u64 { set.iter().map(|&e| sizes[e as usize]).sum() };
    let thin = |set: &[u32]| -> u64 { set.iter().map(|&e| lean[e as usize]).sum() };
    let mut sites: Vec<Site> = Vec::new();
    let mut names: Vec<String> = subs
        .iter()
        .filter(|s| crate::level::is_zone(&s.name))
        .map(|s| s.name.clone())
        .collect();
    let art = names.len();
    names.extend(level.drivable_zones());
    let mut in_sites = HashSet::new();
    for name in &names {
        let zone = level.zone(name)?;
        let median = |axis: usize| {
            let mut v: Vec<f32> = zone
                .placements
                .iter()
                .map(|p| p.transform.trans[axis])
                .collect();
            v.sort_by(f32::total_cmp);
            v.get(v.len() / 2).copied()
        };
        let at = median(0).zip(median(2)).map(|(x, z)| [x, z]);
        // The zone, and its midday light: one time of day at a time.
        let mut set = by_name.get(name).cloned().unwrap_or_default();
        if let Some(light) = by_name.get(&format!("{name}_midday")) {
            set.extend(light);
        }
        set.sort_unstable();
        set.dedup();
        in_sites.insert(name.clone());
        in_sites.insert(format!("{name}_midday"));
        let (stored, raw_bytes) = (bytes(&set), raw(&set));
        sites.push(Site {
            at: at.unwrap_or([f32::NAN; 2]),
            entries: set,
            stored,
            raw: raw_bytes,
        });
    }

    let stats = |sites: &[Site]| {
        let mut v: Vec<u64> = sites.iter().map(|s| s.stored).collect();
        v.sort_unstable();
        (
            v.iter().sum::<u64>(),
            v.first().copied().unwrap_or(0),
            v.get(v.len() / 2).copied().unwrap_or(0),
            v.last().copied().unwrap_or(0),
        )
    };
    let _ = writeln!(out, "\n## Zones\n");
    let _ = writeln!(out, "| Zones | Count | Sum | Smallest | Median | Largest |");
    let _ = writeln!(out, "|---|---:|---:|---:|---:|---:|");
    let (a, d) = sites.split_at(art.min(sites.len()));
    for (label, part) in [("art, with midday light", a), ("drivable", d)] {
        let (sum, lo, mid, hi) = stats(part);
        let _ = writeln!(
            out,
            "| {label} | {} | {} | {} | {} | {} |",
            part.len(),
            gib(sum),
            mib(lo),
            mib(mid),
            mib(hi),
        );
    }
    let unplaced: Vec<&Site> = sites.iter().filter(|s| s.at[0].is_nan()).collect();
    let _ = writeln!(
        out,
        "\nZones that place no mesh, so have no position and are left out of the drives: {}, {}.",
        unplaced.len(),
        mib(unplaced.iter().map(|s| s.stored).sum()),
    );
    let placed_art = sites[..art.min(sites.len())]
        .iter()
        .filter(|s| !s.at[0].is_nan())
        .count();
    sites.retain(|s| !s.at[0].is_nan());
    let raw_sum: u64 = sites.iter().map(|s| s.raw).sum();
    let _ = writeln!(
        out,
        "\nDecompressed, all zones counted separately: {}.",
        gib(raw_sum)
    );

    // What every zone leaves to the level: the level's other bundles.
    let mut level_rest: Vec<u32> = Vec::new();
    for sub in &subs {
        if in_sites.contains(&sub.name) {
            continue;
        }
        let is_variant = ["_overcast", "_sunset", "_tealnight", "_midday"]
            .iter()
            .any(|v| sub.name.ends_with(v));
        if is_variant {
            continue;
        }
        if let Some(set) = by_name.get(&sub.name) {
            level_rest.extend(set);
        }
    }
    level_rest.sort_unstable();
    level_rest.dedup();
    let mut globals: Vec<u32> = Vec::new();
    for sb in &store.superbundles {
        if sb.name.eq_ignore_ascii_case("Win32/globals") {
            for b in &sb.bundles {
                if let Some(set) = by_name.get(&bundle_key(&b.name)) {
                    globals.extend(set);
                }
            }
        }
    }
    globals.sort_unstable();
    globals.dedup();
    let spacing = |part: &[Site]| {
        let mut gaps: Vec<f32> = part
            .iter()
            .enumerate()
            .map(|(i, s)| {
                part.iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, o)| {
                        ((s.at[0] - o.at[0]).powi(2) + (s.at[1] - o.at[1]).powi(2)).sqrt()
                    })
                    .fold(f32::MAX, f32::min)
            })
            .collect();
        gaps.sort_by(f32::total_cmp);
        gaps.get(gaps.len() / 2).copied().unwrap_or(0.0)
    };
    let _ = writeln!(
        out,
        "\nMedian distance to the nearest zone: art {:.0} m, drivable {:.0} m.",
        spacing(&sites[..placed_art]),
        spacing(&sites[placed_art..])
    );
    let mut parts: Vec<(u64, u64, &str)> = subs
        .iter()
        .filter(|s| !in_sites.contains(&s.name))
        .filter_map(|s| {
            let set = by_name.get(&s.name)?;
            Some((bytes(set), thin(set), s.name.as_str()))
        })
        .collect();
    parts.sort_unstable_by_key(|p| std::cmp::Reverse(p.0));
    let _ = writeln!(out, "\n## The level's largest bundles outside the zones\n");
    let _ = writeln!(out, "| Bundle | Needs, stored | Without streamed mips |");
    let _ = writeln!(out, "|---|---:|---:|");
    for (all, without, name) in parts.iter().take(15) {
        let _ = writeln!(out, "| `{name}` | {} | {} |", mib(*all), mib(*without));
    }
    let _ = writeln!(out, "\n## Resident\n");
    let _ = writeln!(
        out,
        "| Set | Stored | Without streamed mips | Decompressed |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|");
    for (label, set) in [
        (
            "the level's bundles outside the zones and their light",
            &level_rest,
        ),
        ("the `globals` superbundle", &globals),
    ] {
        let _ = writeln!(
            out,
            "| {label} | {} | {} | {} |",
            mib(bytes(set)),
            mib(thin(set)),
            mib(raw(set))
        );
    }

    // Streaming along straight lines.
    let resident: HashSet<u32> = level_rest.iter().chain(&globals).copied().collect();
    let _ = writeln!(out, "\n## Streaming along straight lines\n");
    let _ = writeln!(
        out,
        "| Radius | Mips | Mean MiB/km | Worst line MiB/km | Worst 1 km MiB/km | Mean MiB/s | Worst 1 km MiB/s |"
    );
    let _ = writeln!(out, "|---:|---|---:|---:|---:|---:|---:|");
    for radius in [300.0, 500.0, 1000.0, 1500.0, 2000.0, 3000.0] {
        for (label, set) in [("all", sizes), ("without streamed", lean)] {
            let [mean, line, worst, _] = drive(&sites, set, radius, 1000.0, 0.0, &resident);
            let _ = writeln!(
                out,
                "| {radius} m | {label} | {mean:.0} | {line:.0} | {worst:.0} | {:.1} | {:.1} |",
                mean * TOP_SPEED,
                worst * TOP_SPEED
            );
        }
    }
    // The steady read rate that never falls behind, when a site's bytes must be in by
    // the time it comes within the radius and reading may start `lead` metres earlier.
    let _ = writeln!(
        out,
        "\n## Steady read rate that never falls behind, at top speed\n"
    );
    let _ = writeln!(
        out,
        "| Everything in by | Read ahead by | Mips | MiB/s |\n|---:|---:|---|---:|"
    );
    for radius in [300.0_f32, 500.0, 1000.0, 1500.0] {
        for lead in [500.0_f32, 1000.0, 2000.0] {
            for (label, set) in [("all", sizes), ("without streamed", lean)] {
                let [.., rate] = drive(&sites, set, radius, 1000.0, lead, &resident);
                let _ = writeln!(out, "| {radius} m | {lead} m | {label} | {rate:.1} |");
            }
        }
    }
    Ok(())
}

/// How a format's blocks split into fields, each `(offset, length)`: fields of the same
/// kind side by side compress better than whole blocks do.
fn block_fields(format: &str) -> Option<(usize, &'static [(usize, usize)])> {
    let name = format.split('_').next()?;
    Some(match name {
        // Colours, then which colour each pixel takes.
        "BC1" | "BC1A" => (8, &[(0, 4), (4, 4)]),
        // Alpha end points, alpha indices, colours, colour indices.
        "BC2" | "BC3" => (16, &[(0, 2), (8, 4), (2, 6), (12, 4)]),
        "BC4" => (8, &[(0, 2), (2, 6)]),
        // Both channels' end points, then both channels' indices.
        "BC5" => (16, &[(0, 2), (8, 2), (2, 6), (10, 6)]),
        _ => return None,
    })
}

/// `data` with each block's fields gathered: all of the first field, then the second...
/// Reversible: the block size and the fields say where every byte came from.
fn gather(data: &[u8], block: usize, fields: &[(usize, usize)]) -> Vec<u8> {
    let blocks = data.chunks_exact(block);
    let rest = blocks.remainder();
    let mut out = Vec::with_capacity(data.len());
    for &(offset, length) in fields {
        for b in blocks.clone() {
            out.extend_from_slice(&b[offset..offset + length]);
        }
    }
    out.extend_from_slice(rest);
    out
}

/// A LOD's chunk with each stream's vertices gathered byte by byte (all the first bytes,
/// then the second...), and its 16-bit indices as differences from the one before.
fn gather_mesh(data: &[u8], lod: &frostbite::mesh::Lod) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut covered = vec![false; data.len()];
    for s in &lod.sections {
        let mut start = s.vertex_offset as usize;
        for &stride in &s.strides {
            let (stride, count) = (usize::from(stride), s.vertices as usize);
            if stride == 0 {
                continue;
            }
            let Some(part) = data.get(start..start + stride * count) else {
                continue;
            };
            for byte in 0..stride {
                out.extend(part.iter().skip(byte).step_by(stride));
            }
            covered[start..start + stride * count].fill(true);
            start += stride * count;
        }
    }
    let index_at = lod.vertex_bytes as usize;
    let mut last = 0u16;
    if lod.index_width == 2 {
        let end = (index_at + lod.index_bytes as usize).min(data.len());
        if let Some(indices) = data.get(index_at..end) {
            let mut bytes = (Vec::new(), Vec::new());
            for pair in indices.as_chunks::<2>().0 {
                let value = u16::from_le_bytes([pair[0], pair[1]]);
                let [low, high] = value.wrapping_sub(last).to_le_bytes();
                bytes.0.push(low);
                bytes.1.push(high);
                last = value;
            }
            out.extend(bytes.0);
            out.extend(bytes.1);
            covered[index_at..end].fill(true);
        }
    }
    // Anything no section covers stays as it is, in order.
    out.extend(
        data.iter()
            .zip(&covered)
            .filter(|(_, c)| !**c)
            .map(|(b, _)| *b),
    );
    out
}

/// One line of the trial: what a kind's sampled chunks store now and after the change.
#[derive(Default)]
struct Trial {
    chunks: u64,
    raw: u64,
    now: u64,
    after: u64,
}

/// What a chunk of `format` stores split into a frame per tier, largest mips first.
#[derive(Default)]
struct Tiers {
    chunks: u64,
    raw: [u64; TIERS],
    stored: [u64; TIERS],
    now: u64,
}

/// The tiers: the top mip, the second, the third, the rest.
const TIERS: usize = 4;

/// `nfsh sizes-trial STORE`: recompresses every `EVERY`th texture and mesh chunk at the
/// store's own level with their fields gathered, and compares. Nothing is written.
pub fn trial(dir: &Path) -> Result<String> {
    const EVERY: usize = 40;
    let store = Store::open(dir)?;
    let whole = store.whole_chunks();
    let mut res: Vec<(Location, u32)> = store
        .superbundles
        .iter()
        .flat_map(|sb| &sb.bundles)
        .flat_map(|b| &b.bundle.res)
        .filter(|r| r.res_type == texture::TEXTURE || r.res_type == res_type_hash("MeshSet"))
        .map(|r| (r.asset.location, r.res_type))
        .collect();
    res.sort_unstable_by_key(|r| r.0);
    res.dedup();
    enum Sample {
        Texture(Box<Texture>, Location),
        Mesh(Box<frostbite::mesh::Lod>, Location),
    }
    let found = in_parallel(
        &res,
        || store.entries(),
        |reader, &(l, kind)| {
            let data = reader.read(l)?;
            let mut out = Vec::new();
            if kind == texture::TEXTURE {
                if let Ok(t) = Texture::parse(&data)
                    && let Some(&at) = whole.get(&t.chunk)
                    && t.format().is_some()
                {
                    out.push((t.chunk, Sample::Texture(Box::new(t), at)));
                }
            } else if let Ok(m) = MeshSet::parse(&data) {
                for lod in m.lods {
                    if let Some(id) = lod.chunk
                        && let Some(&at) = whole.get(&id)
                    {
                        out.push((id, Sample::Mesh(Box::new(lod), at)));
                    }
                }
            }
            Ok(out)
        },
    )?;
    // A chunk named twice is one sample, picked by its id so the pick has no order in it.
    let mut samples: HashMap<[u8; 16], Sample> = HashMap::new();
    for (id, s) in found.into_iter().flatten() {
        samples.entry(id).or_insert(s);
    }
    let mut picked: Vec<Sample> = samples
        .into_iter()
        .filter(|(id, _)| usize::from(id[1]) % EVERY == 0)
        .map(|(_, s)| s)
        .collect();
    picked.sort_by_key(|s| match s {
        Sample::Texture(_, l) | Sample::Mesh(_, l) => *l,
    });
    eprintln!("{} samples", picked.len());
    let results = in_parallel(
        &picked,
        || {
            Ok((
                store.entries()?,
                frostbite::frames::FrameEncoder::new(None)?,
            ))
        },
        |(reader, encoder), s| {
            let (label, at, moved, split) = match s {
                Sample::Texture(t, at) => {
                    let data = reader.read(*at)?;
                    let format = t.format().map_or("", |f| f.name);
                    let moved = block_fields(format).map(|(b, f)| gather(&data, b, f));
                    let mut split = ([0; TIERS], [0; TIERS]);
                    // A chunk that isn't exactly its mips has no tiers to cut at.
                    let cuts = (data.len() as u64 == t.chunk_bytes()).then(|| t.tier_cuts());
                    let cuts = cuts.unwrap_or([0; TIERS - 1]).map(|c| c as usize);
                    let mut from = 0;
                    for (tier, to) in cuts.into_iter().chain([data.len()]).enumerate() {
                        if to > from {
                            let frame = encoder.encode(&data[from..to], false)?;
                            split.0[tier] = (to - from) as u64;
                            split.1[tier] = frame.len() as u64;
                        }
                        from = to;
                    }
                    (
                        format.to_owned(),
                        *at,
                        moved.unwrap_or(data),
                        Some((t.side(), split)),
                    )
                }
                Sample::Mesh(lod, at) => {
                    let data = reader.read(*at)?;
                    ("mesh".to_owned(), *at, gather_mesh(&data, lod), None)
                }
            };
            let frame = encoder.encode(&moved, false)?;
            Ok((
                label,
                moved.len() as u64,
                u64::from(at.size),
                frame.len() as u64,
                split,
            ))
        },
    )?;
    let mut sums: BTreeMap<String, Trial> = BTreeMap::new();
    let mut tiers: BTreeMap<u32, Tiers> = BTreeMap::new();
    for (label, raw, now, after, split) in results {
        let t = sums.entry(label).or_default();
        t.chunks += 1;
        t.raw += raw;
        t.now += now;
        t.after += after;
        if let Some((side, (raws, stored))) = split {
            let t = tiers.entry(side.next_power_of_two().max(256)).or_default();
            t.chunks += 1;
            t.now += now;
            for i in 0..TIERS {
                t.raw[i] += raws[i];
                t.stored[i] += stored[i];
            }
        }
    }
    let mut out = String::from(
        "| Kind | Chunks sampled | Decompressed | Stored now | Fields gathered | Change |\n\
         |---|---:|---:|---:|---:|---:|\n",
    );
    for (label, t) in &sums {
        let _ = writeln!(
            out,
            "| {label} | {} | {} | {} | {} | {:+.1}% |",
            t.chunks,
            mib(t.raw),
            mib(t.now),
            mib(t.after),
            (t.after as f64 / t.now.max(1) as f64 - 1.0) * 100.0
        );
    }
    let _ = writeln!(
        out,
        "\nTextures split into a frame per tier, by largest side (stored bytes):\n\n\
         | Largest side | Chunks sampled | One frame | Top mip | Second | Third | Rest | \
         Split | Change |\n|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    let mut all = Tiers::default();
    for (side, t) in &tiers {
        all.chunks += t.chunks;
        all.now += t.now;
        for i in 0..TIERS {
            all.raw[i] += t.raw[i];
            all.stored[i] += t.stored[i];
        }
        let label = match side {
            256 => "up to 256".to_owned(),
            side => format!("up to {side}"),
        };
        let _ = writeln!(out, "{}", tier_row(&label, t));
    }
    let _ = writeln!(out, "{}", tier_row("all", &all));
    let _ = writeln!(
        out,
        "\nDecompressed, all: top {}, second {}, third {}, rest {}.",
        mib(all.raw[0]),
        mib(all.raw[1]),
        mib(all.raw[2]),
        mib(all.raw[3]),
    );
    Ok(out)
}

fn tier_row(label: &str, t: &Tiers) -> String {
    let split: u64 = t.stored.iter().sum();
    format!(
        "| {label} | {} | {} | {} | {} | {} | {} | {} | {:+.2}% |",
        t.chunks,
        mib(t.now),
        mib(t.stored[0]),
        mib(t.stored[1]),
        mib(t.stored[2]),
        mib(t.stored[3]),
        mib(split),
        (split as f64 / t.now.max(1) as f64 - 1.0) * 100.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_name_loses_its_numbers() {
        assert_eq!(
            pattern("levels/mainlevel/worldart_zs_495_midday"),
            "levels/mainlevel/worldart_zs_#_midday"
        );
        assert_eq!(
            group("levels/mainlevel/worldart_zs_495"),
            "levels/mainlevel"
        );
    }

    #[test]
    fn a_wave_takes_its_language_from_its_path() {
        assert_eq!(language("Audio/VO/Cops/Content/DE/line"), "de");
        assert_eq!(
            language("audio/vo/cops/content/line"),
            "content, no language folder"
        );
        assert_eq!(language("audio/cinematics/audioassets/c0/line"), CINEMATIC);
        assert_eq!(language("audio/crowds/line"), "other");
    }

    #[test]
    fn fields_gather_and_keep_every_byte() {
        let (block, fields) = block_fields("BC1_SRGB").unwrap();
        let data = [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 9];
        assert_eq!(
            gather(&data, block, fields),
            [1, 1, 1, 1, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4, 9]
        );
        for format in [
            "BC1_UNORM",
            "BC1A_SRGB",
            "BC3_SRGB",
            "BC4_UNORM",
            "BC5_UNORM",
        ] {
            let (block, fields) = block_fields(format).unwrap();
            let mut seen: Vec<usize> = fields
                .iter()
                .flat_map(|&(offset, length)| offset..offset + length)
                .collect();
            seen.sort_unstable();
            assert_eq!(seen, (0..block).collect::<Vec<_>>(), "{format}");
        }
        assert!(block_fields("BC7_UNORM").is_none());
    }

    #[test]
    fn chunk_ids_are_found_at_any_offset_in_either_order() {
        let id: [u8; 16] = std::array::from_fn(|i| i as u8 + 1);
        let finder = Finder::new(&HashMap::from([(id, 7)]));
        let mut data = vec![0xEE; 3];
        data.extend(id);
        assert_eq!(finder.scan(&data), [7]);
        let mut data = vec![0xEE; 5];
        data.extend(frostbite::store::guid_order(id));
        data.extend(id);
        assert_eq!(finder.scan(&data), [7], "one chunk, found once");
        assert!(finder.scan(&id[..15]).is_empty());
    }

    /// Three sites on the box's diagonal. The first comes into range during the drive's
    /// first steps, a loading screen; the other two are each read in the `lead` before
    /// they are due, 1 s at top speed.
    #[test]
    fn a_drive_counts_what_comes_into_range() {
        let site = |x: f32, e: u32| Site {
            at: [x, x],
            entries: vec![e],
            stored: 0,
            raw: 0,
        };
        let sites = [site(0.0, 0), site(2000.0, 1), site(4000.0, 2)];
        let sizes = [111 << 20; 3];
        let lead = TOP_SPEED as f32 * 1000.0;
        let [.., rate] = drive(&sites, &sizes, 100.0, 1000.0, lead, &HashSet::new());
        assert!((rate - 111.0).abs() < 0.5, "{rate}");
        let all = HashSet::from([0, 1, 2]);
        assert_eq!(
            drive(&sites, &sizes, 100.0, 1000.0, lead, &all),
            [0.0; 4],
            "what is resident is never read"
        );
    }
}
