//! The copy step: a store's frames copied into an *install*, keeping only the parts
//! and tiers a player chose, one file per part and tier (`base.nfsh`, `base.mip0.nfsh`,
//! `en.nfsh`, …), so adding or removing a tier later is a matter of whole files.
//!
//! An install holds the store's `index.nfsh` unchanged, whose locations still name
//! the store's packs, and a [`Placement`] (`install.nfsh`) saying for every frame which
//! file holds it and where, or that it was left out. A texture's chunk can lose its
//! leading frames (the top mips) and a mesh's chunk can be missing altogether (a LOD
//! cut); readers fall back to what is there. See `docs/FORMATS.md`, "Installs".

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::ebx::Partition;
use crate::mesh::MeshSet;
use crate::plan::{BASE, Finder, tier};
use crate::sb::res_type_hash;
use crate::store::{INDEX, Store, clear, finish, in_parallel, io_error, pack_name, toc_chunk_id};
use crate::{Location, Result, bail};

/// Where an install's frames are.
pub const PLACEMENT: &str = "install.nfsh";
const MAGIC: &[u8; 8] = b"NFSHINST";
const VERSION: u32 = 1;
/// A frame the install left out.
const CUT: u8 = u8::MAX;
/// How much of each file the copy holds in memory before writing it.
const WRITE_BUFFER: usize = 1 << 20;

/// One of an install's files: every frame of one part in one tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartFile {
    pub name: String,
    /// An index into [`Store::parts`].
    pub part: u8,
    /// [`tier::WHOLE`] for what is never cut, else the tier it holds ([`install_tiers`]).
    pub tier: u8,
    pub size: u64,
}

/// The file name for `part`'s frames of `tier`.
pub fn file_name(part: &str, tier: u8) -> String {
    match tier::slug(tier) {
        Some(slug) => format!("{part}.{slug}.nfsh"),
        None => format!("{part}.nfsh"),
    }
}

/// A name an install may give a file: letters, digits, dots, dashes and underscores,
/// not starting with a dot, so it stays inside the install's folder.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// A run of frames in one of an install's files: (file, offset, length).
type Piece = (u8, u64, u64);

/// Where each frame of an install is: the file and the offset in it, or `None` when the
/// install left it out.
#[derive(Debug)]
pub struct Placement {
    dir: PathBuf,
    pub files: Vec<PartFile>,
    at: Vec<Option<(u8, u64)>>,
    sizes: Vec<u32>,
    /// Where each frame started in the store, as (pack, offset).
    starts: Vec<(u8, u32)>,
}

impl Placement {
    /// The placement of the install in `dir`, given its store's frames; `None` when
    /// `dir` is a store, not an install. Every file must be there and of its size.
    pub fn open(dir: &Path, store: &Store) -> Result<Option<Self>> {
        let path = dir.join(PLACEMENT);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error(&path, e)),
        };
        let (files, of) =
            decode(&bytes).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
        if of.len() != store.frames.len() {
            bail!(
                "{}: {} frames, the index {}",
                path.display(),
                of.len(),
                store.frames.len()
            );
        }
        let mut ends = vec![0u64; files.len()];
        let mut at = Vec::with_capacity(of.len());
        for (&file, frame) in of.iter().zip(&store.frames) {
            if file == CUT {
                at.push(None);
                continue;
            }
            let Some(end) = ends.get_mut(usize::from(file)) else {
                bail!(
                    "{}: a frame in file {file}, of {}",
                    path.display(),
                    files.len()
                );
            };
            if usize::from(frame.part) >= store.parts.len()
                || files[usize::from(file)].part != frame.part
            {
                bail!("{}: a frame in another part's file", path.display());
            }
            at.push(Some((file, *end)));
            *end += u64::from(frame.size);
        }
        for (f, &end) in files.iter().zip(&ends) {
            let path = dir.join(&f.name);
            let len = std::fs::metadata(&path)
                .map_err(|e| io_error(&path, e))?
                .len();
            if len != f.size || end != f.size {
                bail!(
                    "{}: {len} bytes, the install says {} and its frames take {end}",
                    path.display(),
                    f.size
                );
            }
        }
        Ok(Some(Self {
            dir: dir.to_owned(),
            files,
            at,
            sizes: store.frames.iter().map(|f| f.size).collect(),
            starts: store.frame_starts(),
        }))
    }

    /// Whether any frame of the entry at `location` is left out.
    pub fn is_cut(&self, location: Location) -> Result<bool> {
        Ok(self.pieces(location)?.1)
    }

    /// The pieces of files holding the entry at `location` (a location in the store),
    /// as (file, offset, length), with whether any of its frames is left out. Only an
    /// entry's first frames can be left out: a texture keeps its smallest mips.
    fn pieces(&self, location: Location) -> Result<(Vec<Piece>, bool)> {
        let Ok(mut n) = self
            .starts
            .binary_search(&(location.cas.cas, location.offset))
        else {
            bail!("{location:?} is not at the start of a frame");
        };
        let mut left = u64::from(location.size);
        let mut out: Vec<Piece> = Vec::new();
        let mut cut = false;
        while left > 0 {
            let Some(&size) = self.sizes.get(n) else {
                bail!("{location:?} runs past the last frame");
            };
            let size = u64::from(size);
            if size > left {
                bail!("{location:?} ends inside a frame");
            }
            match self.at[n] {
                None if !out.is_empty() => {
                    bail!("{location:?}: a frame is left out after one that is kept")
                }
                None => cut = true,
                Some((file, offset)) => match out.last_mut() {
                    Some((f, o, len)) if *f == file && *o + *len == offset => *len += size,
                    _ => out.push((file, offset, size)),
                },
            }
            left -= size;
            n += 1;
        }
        Ok((out, cut))
    }
}

/// An install's files, open for reading; one per thread.
pub struct Placed<'a> {
    placement: &'a Placement,
    open: Vec<Option<File>>,
}

impl<'a> Placed<'a> {
    pub fn new(placement: &'a Placement) -> Self {
        Self {
            placement,
            open: (0..placement.files.len()).map(|_| None).collect(),
        }
    }

    /// The stored frames of the entry at `location` that the install keeps, back to
    /// back, and whether any were left out.
    pub fn stored(&mut self, location: Location) -> Result<(Vec<u8>, bool)> {
        let (pieces, cut) = self.placement.pieces(location)?;
        let mut out = Vec::new();
        for (file, offset, len) in pieces {
            let path = self
                .placement
                .dir
                .join(&self.placement.files[usize::from(file)].name);
            let slot = &mut self.open[usize::from(file)];
            if slot.is_none() {
                *slot = Some(File::open(&path).map_err(|e| io_error(&path, e))?);
            }
            let Some(handle) = slot.as_ref() else {
                bail!("{}: not open", path.display());
            };
            let from = out.len();
            out.resize(from + len as usize, 0);
            crate::cas::read_at(handle, &mut out[from..], offset).map_err(|e| {
                crate::Error(format!(
                    "{}: {len} bytes at {offset:#x}: {e}",
                    path.display()
                ))
            })?;
        }
        Ok((out, cut))
    }
}

fn encode(files: &[PartFile], of: &[u8]) -> Vec<u8> {
    let mut w = MAGIC.to_vec();
    w.extend(VERSION.to_be_bytes());
    w.extend((files.len() as u32).to_be_bytes());
    for f in files {
        w.extend((f.name.len() as u32).to_be_bytes());
        w.extend(f.name.as_bytes());
        w.push(f.part);
        w.push(f.tier);
        w.extend(f.size.to_be_bytes());
    }
    w.extend((of.len() as u32).to_be_bytes());
    w.extend(of);
    w
}

fn decode(bytes: &[u8]) -> Result<(Vec<PartFile>, Vec<u8>)> {
    let mut r = crate::reader::Reader::new(bytes);
    if r.bytes(8)? != MAGIC {
        bail!("not an nfsh install's placement");
    }
    let version = r.u32_be()?;
    if version != VERSION {
        bail!("placement version {version}; this nfsh reads {VERSION}");
    }
    let count = r.u32_be()? as usize;
    if count >= usize::from(CUT) {
        bail!("{count} files");
    }
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.u32_be()? as usize;
        let name = String::from_utf8_lossy(r.bytes(len)?).into_owned();
        if !safe_name(&name) {
            bail!("a file named {name:?}");
        }
        let part = r.u8()?;
        let tier = r.u8()?;
        let size = r.u64_be()?;
        files.push(PartFile {
            name,
            part,
            tier,
            size,
        });
    }
    let frames = r.u32_be()? as usize;
    let of = r.bytes(frames)?.to_vec();
    if r.pos() != bytes.len() {
        bail!("{} bytes after the frames", bytes.len() - r.pos());
    }
    Ok((files, of))
}

/// What a player keeps: the base always, these languages (`None` for every one the
/// store has), and every tier but these.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub languages: Option<Vec<String>>,
    pub cut: Vec<u8>,
}

/// The install-time tier of an ebx named `name` whose first instance is a `root`, if
/// it is one: cutscene video (the movies under `cinematics/`), voices (the spoken lines
/// under `audio/vo/` and the cinematics' audio, in every language) or music.
pub fn media_tier(name: &str, root: &str) -> Option<u8> {
    let name = name.to_ascii_lowercase();
    let under = |folder: &str| name.starts_with(folder);
    match root {
        "MovieTexture2Asset" if under("cinematics/") => Some(tier::VIDEO),
        "LocalizedWaveAsset" if under("audio/vo/") || under("audio/cinematics/") => {
            Some(tier::VOICES)
        }
        "NewWaveAsset" if under("audio/music/") => Some(tier::MUSIC),
        _ => None,
    }
}

/// By chunk, its media tier: one tier's ebx name it, and no other ebx does.
/// `named` is every ebx's media tier, if any, and the chunks it names.
fn media_of(named: &[(Option<u8>, Vec<u32>)], chunks: usize) -> Vec<Option<u8>> {
    // `None`: nothing names it yet; `Some(None)`: something that is no one tier's media.
    let mut of: Vec<Option<Option<u8>>> = vec![None; chunks];
    for (tier, names) in named {
        for &c in names {
            let Some(slot) = of.get_mut(c as usize) else {
                continue;
            };
            *slot = Some(match *slot {
                None => *tier,
                Some(had) if had == *tier => had,
                Some(_) => None,
            });
        }
    }
    of.into_iter().map(Option::flatten).collect()
}

/// The frames holding the entry at `location`, if it starts at a frame and ends at one.
fn frames_at(starts: &[(u8, u32)], sizes: &[u32], location: Location) -> Option<Range<usize>> {
    let first = starts
        .binary_search(&(location.cas.cas, location.offset))
        .ok()?;
    let mut left = u64::from(location.size);
    let mut end = first;
    while left > 0 {
        let size = u64::from(*sizes.get(end)?);
        left = left.checked_sub(size)?;
        end += 1;
    }
    Some(first..end)
}

/// The tier each frame of `store` is installed by: a texture's top three mips each
/// their own, a mesh chunk's LOD its own unless it is the last LOD of a mesh using it
/// (a mesh keeps its last LOD, as a texture its smallest mip), cutscene video, voices
/// and music their own ([`media_tier`]), and everything else [`tier::WHOLE`]. The
/// store tags a mesh chunk with its best LOD; this reads every mesh set again to know
/// which chunks are a mesh's last, and every ebx for the chunks it names.
pub fn install_tiers(store: &Store) -> Result<Vec<u8>> {
    let mut tiers: Vec<u8> = store
        .frames
        .iter()
        .map(|f| match f.tier {
            t @ (tier::TOP | tier::SECOND | tier::THIRD) => t,
            _ => tier::WHOLE,
        })
        .collect();
    let starts = store.frame_starts();
    let whole = store.whole_chunks();
    let sets: Vec<Location> = store
        .res_of_type(res_type_hash("MeshSet"))
        .into_values()
        .collect();
    let lods = in_parallel(
        &sets,
        || store.entries(),
        |entries, &l| {
            let data = entries.read(l)?;
            Ok(MeshSet::parse(&data)
                .map(|m| m.lods.iter().map(|lod| lod.chunk).collect::<Vec<_>>())
                .ok())
        },
    )?;
    // By frame: `Some(None)` when it is some mesh's last LOD, else the highest LOD it is.
    let mut need: Vec<Option<Option<u8>>> = vec![None; tiers.len()];
    for lods in lods.into_iter().flatten() {
        for (i, id) in lods.iter().enumerate() {
            let Some(l) = id.and_then(|id| whole.get(&id)) else {
                continue;
            };
            let Ok(f) = starts.binary_search(&(l.cas.cas, l.offset)) else {
                continue;
            };
            let frame = store.frames[f];
            if frame.tier < tier::LOD || frame.size != l.size {
                continue;
            }
            let this = (i + 1 < lods.len()).then(|| i.min(200) as u8);
            need[f] = Some(match need[f] {
                None => this,
                Some(had) => had.zip(this).map(|(a, b)| a.max(b)),
            });
        }
    }
    for (t, need) in tiers.iter_mut().zip(need) {
        if let Some(Some(lod)) = need {
            *t = tier::LOD + lod;
        }
    }
    media_tiers(store, &starts, &mut tiers)?;
    Ok(tiers)
}

/// Tags the frames of cutscene video, voices and music with their tier, where no
/// other entry needs them: a frame stays whole if any entry that isn't one tier's
/// media is stored in it.
fn media_tiers(store: &Store, starts: &[(u8, u32)], tiers: &mut [u8]) -> Result<()> {
    let mut ids: HashMap<[u8; 16], u32> = HashMap::new();
    let mut chunks: Vec<(u32, Location)> = Vec::new();
    let mut others: Vec<Location> = Vec::new();
    let mut ebx: BTreeMap<Location, Vec<&str>> = BTreeMap::new();
    let mut chunk = |ids: &mut HashMap<[u8; 16], u32>, id: [u8; 16], l: Location| {
        let next = ids.len() as u32;
        chunks.push((*ids.entry(id).or_insert(next), l));
    };
    for sb in &store.superbundles {
        for b in &sb.bundles {
            for a in &b.bundle.ebx {
                ebx.entry(a.location).or_default().push(&a.name);
                others.push(a.location);
            }
            others.extend(b.bundle.res.iter().map(|r| r.asset.location));
            for c in &b.bundle.chunks {
                chunk(&mut ids, c.id, c.location);
            }
        }
        for c in &sb.chunks {
            chunk(&mut ids, toc_chunk_id(c.chunk.id), c.chunk.location);
        }
    }
    let finder = Finder::new(&ids);
    let ebx: Vec<(Location, Vec<&str>)> = ebx.into_iter().collect();
    let named = in_parallel(
        &ebx,
        || store.entries(),
        |entries, (l, names)| {
            let data = entries.read(*l)?;
            let root = Partition::parse(&data)
                .ok()
                .and_then(|p| p.instances.first().map(|i| p.type_name(i.type_index)))
                .unwrap_or_default();
            let found = finder.scan(&data);
            Ok(names
                .iter()
                .map(|n| (media_tier(n, &root), found.clone()))
                .collect::<Vec<_>>())
        },
    )?;
    let named: Vec<(Option<u8>, Vec<u32>)> = named.into_iter().flatten().collect();
    let media = media_of(&named, ids.len());
    let sizes: Vec<u32> = store.frames.iter().map(|f| f.size).collect();
    let mut kept = vec![false; tiers.len()];
    let mut tagged: Vec<(Range<usize>, u8)> = Vec::new();
    for (c, l) in chunks {
        let Some(frames) = frames_at(starts, &sizes, l) else {
            continue;
        };
        match media[c as usize] {
            Some(t) => tagged.push((frames, t)),
            None => kept[frames].fill(true),
        }
    }
    for l in others {
        if let Some(frames) = frames_at(starts, &sizes, l) {
            kept[frames].fill(true);
        }
    }
    for (frames, t) in tagged {
        for f in frames {
            if !kept[f] && tiers[f] == tier::WHOLE {
                tiers[f] = t;
            }
        }
    }
    Ok(())
}

/// What [`copy`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    pub files: Vec<PartFile>,
    pub parts: Vec<String>,
    /// The store's frames, and those left out.
    pub frames: u64,
    pub frames_cut: u64,
    /// The store's packs, and the install's files, in bytes.
    pub store_bytes: u64,
    pub bytes: u64,
    /// The index and the placement, in bytes.
    pub index: u64,
}

/// Which file each frame goes in (or [`CUT`]), and the files, sorted by part and tier.
fn assign(
    parts: &[String],
    frames: &[(u8, u8)],
    keep_part: &[bool],
    cut: &[u8],
) -> (Vec<u8>, Vec<PartFile>) {
    let kept = |&(part, tier): &(u8, u8)| {
        keep_part.get(usize::from(part)) == Some(&true) && !cut.contains(&tier)
    };
    let mut keys: BTreeMap<(u8, u8), u64> = BTreeMap::new();
    for f in frames.iter().filter(|f| kept(f)) {
        keys.entry(*f).or_default();
    }
    let number: BTreeMap<(u8, u8), u8> = keys.keys().zip(0..).map(|(&k, n)| (k, n)).collect();
    let of = frames
        .iter()
        .map(|f| if kept(f) { number[f] } else { CUT })
        .collect();
    let files = number
        .keys()
        .map(|&(part, tier)| PartFile {
            name: file_name(&parts[usize::from(part)], tier),
            part,
            tier,
            size: 0,
        })
        .collect();
    (of, files)
}

/// Copies the store `store` into an install in `dest`, keeping what `choice` says. An
/// earlier store or install in `dest` is replaced; anything else there is refused.
/// `say` hears each step. The placement is written last, so an install cut short
/// doesn't open.
pub fn copy(
    store: &Store,
    dest: &Path,
    choice: &Choice,
    say: &mut dyn FnMut(&str),
) -> Result<Copied> {
    if store.frames.is_empty() {
        bail!(
            "{}: a store made before parts can't be installed; import it again",
            store.dir.display()
        );
    }
    if store.placement.is_some() {
        bail!("{} is an install; copy from the store", store.dir.display());
    }
    let same = |a: &Path, b: &Path| matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b);
    if same(&store.dir, dest) {
        bail!(
            "{}: the install can't replace its own store",
            dest.display()
        );
    }
    let mut keep_part = vec![false; store.parts.len()];
    keep_part[usize::from(BASE)] = true;
    match &choice.languages {
        None => keep_part.iter_mut().for_each(|k| *k = true),
        Some(languages) => {
            for l in languages {
                let Some(n) = store.parts.iter().position(|p| p == l && p != "base") else {
                    bail!(
                        "no language {l:?} in the store (it has {})",
                        store.parts[1..].join(", ")
                    );
                };
                keep_part[n] = true;
            }
        }
    }
    say("reading the mesh sets for their last LODs, and the ebx for the media they name");
    let tiers = install_tiers(store)?;
    let keys: Vec<(u8, u8)> = store
        .frames
        .iter()
        .zip(&tiers)
        .map(|(f, &t)| (f.part, t))
        .collect();
    let (of, mut files) = assign(&store.parts, &keys, &keep_part, &choice.cut);
    if files.len() >= usize::from(CUT) {
        bail!(
            "{} files; an install holds at most {}",
            files.len(),
            CUT - 1
        );
    }

    clear(dest)?;
    let index = store.dir.join(INDEX);
    std::fs::copy(&index, dest.join(INDEX)).map_err(|e| io_error(&index, e))?;
    let mut writers = Vec::with_capacity(files.len());
    for f in &files {
        let path = dest.join(&f.name);
        let file = File::create(&path).map_err(|e| io_error(&path, e))?;
        writers.push((path, BufWriter::with_capacity(WRITE_BUFFER, file)));
    }
    let starts = store.frame_starts();
    let mut reader: Option<(u8, PathBuf, BufReader<File>)> = None;
    let mut buf = Vec::new();
    let mut frames_cut = 0;
    for ((frame, &(pack, _)), &file) in store.frames.iter().zip(&starts).zip(&of) {
        if reader.as_ref().is_none_or(|(p, _, _)| *p != pack) {
            say(&format!(
                "copying pack {} of {}",
                u32::from(pack) + 1,
                store.packs.len()
            ));
            let path = store.dir.join(pack_name(pack));
            let f = File::open(&path).map_err(|e| io_error(&path, e))?;
            reader = Some((pack, path, BufReader::with_capacity(16 << 20, f)));
        }
        let Some((_, path, r)) = reader.as_mut() else {
            bail!("no pack open");
        };
        buf.resize(frame.size as usize, 0);
        r.read_exact(&mut buf).map_err(|e| io_error(path, e))?;
        if file == CUT {
            frames_cut += 1;
            continue;
        }
        let (path, w) = &mut writers[usize::from(file)];
        w.write_all(&buf).map_err(|e| io_error(path, e))?;
        files[usize::from(file)].size += u64::from(frame.size);
    }
    for (path, w) in writers {
        finish(&path, w)?;
    }
    let tmp = dest.join(format!("{PLACEMENT}.tmp"));
    let placement = encode(&files, &of);
    std::fs::write(&tmp, &placement).map_err(|e| io_error(&tmp, e))?;
    let path = dest.join(PLACEMENT);
    std::fs::rename(&tmp, &path).map_err(|e| io_error(&path, e))?;
    say("checking the install opens");
    let opened = Store::open(dest)?;
    if opened.placement.as_ref().map(|p| &p.files) != Some(&files) {
        bail!("{}: the install reads back differently", dest.display());
    }
    let index = std::fs::metadata(dest.join(INDEX)).map_err(|e| io_error(dest, e))?;
    Ok(Copied {
        index: index.len() + placement.len() as u64,
        bytes: files.iter().map(|f| f.size).sum(),
        files,
        parts: store.parts.clone(),
        frames: store.frames.len() as u64,
        frames_cut,
        store_bytes: store.packs.iter().sum(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_by_part_and_tier_and_stay_in_the_folder() {
        assert_eq!(file_name("base", tier::WHOLE), "base.nfsh");
        assert_eq!(file_name("en", tier::TOP), "en.mip0.nfsh");
        assert_eq!(file_name("base", tier::LOD + 1), "base.lod1.nfsh");
        assert!(safe_name("base.lod1.nfsh"));
        assert!(!safe_name("../base.nfsh"));
        assert!(!safe_name(".nfsh"));
        assert!(!safe_name("a/b.nfsh"));
        assert!(!safe_name(""));
    }

    #[test]
    fn frames_go_to_their_parts_file_unless_cut() {
        let parts: Vec<String> = ["base", "en", "de"].map(String::from).to_vec();
        let frames = [
            (0, tier::WHOLE),
            (0, tier::TOP),
            (1, tier::WHOLE),
            (2, tier::WHOLE),
            (0, tier::LOD),
            (0, tier::WHOLE),
        ];
        let (of, files) = assign(&parts, &frames, &[true, true, false], &[tier::TOP]);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["base.nfsh", "base.lod0.nfsh", "en.nfsh"]);
        assert_eq!(of, [0, CUT, 2, CUT, 1, 0]);
    }

    #[test]
    fn media_is_known_by_its_type_and_folder() {
        let t = |name, root| media_tier(name, root);
        assert_eq!(
            t("Cinematics/Raw_Videos/c01", "MovieTexture2Asset"),
            Some(tier::VIDEO)
        );
        assert_eq!(
            t("ui/movies/introsizzle", "MovieTexture2Asset"),
            None,
            "UI movies stay"
        );
        assert_eq!(
            t("audio/vo/cops/content/en/line", "LocalizedWaveAsset"),
            Some(tier::VOICES)
        );
        assert_eq!(
            t("audio/cinematics/audioassets/c0", "LocalizedWaveAsset"),
            Some(tier::VOICES)
        );
        assert_eq!(
            t("audio/cinematics/sfx/c0", "NewWaveAsset"),
            None,
            "cinematic effects stay"
        );
        assert_eq!(t("audio/crowds/walla", "LocalizedWaveAsset"), None);
        assert_eq!(t("audio/music/track_01", "NewWaveAsset"), Some(tier::MUSIC));
        assert_eq!(
            t("audio/music/track_01", "SoundPatchAsset"),
            None,
            "only the waves"
        );
    }

    #[test]
    fn a_chunk_is_media_only_if_nothing_else_names_it() {
        let named = [
            (Some(tier::MUSIC), vec![0, 1]),
            (Some(tier::MUSIC), vec![1]),
            (Some(tier::VOICES), vec![2, 3]),
            (Some(tier::MUSIC), vec![3]),
            (None, vec![4]),
            (Some(tier::VIDEO), vec![4, 5]),
        ];
        assert_eq!(
            media_of(&named, 7),
            [
                Some(tier::MUSIC),
                Some(tier::MUSIC),
                Some(tier::VOICES),
                None,
                None,
                Some(tier::VIDEO),
                None
            ]
        );
    }

    #[test]
    fn an_entry_covers_whole_frames_or_none() {
        let starts = [(0, 0), (0, 10), (0, 30), (1, 0)];
        let sizes = [10, 20, 5, 7];
        let at = |cas: u8, offset, size| Location {
            cas: crate::CasRef {
                patch: false,
                package: 0,
                cas,
            },
            offset,
            size,
        };
        assert_eq!(frames_at(&starts, &sizes, at(0, 10, 25)), Some(1..3));
        assert_eq!(frames_at(&starts, &sizes, at(1, 0, 7)), Some(3..4));
        assert_eq!(
            frames_at(&starts, &sizes, at(0, 10, 21)),
            None,
            "ends inside a frame"
        );
        assert_eq!(
            frames_at(&starts, &sizes, at(0, 5, 5)),
            None,
            "starts inside one"
        );
        assert_eq!(
            frames_at(&starts, &sizes, at(1, 0, 8)),
            None,
            "runs past the last"
        );
    }

    #[test]
    fn the_placement_reads_back_and_refuses_what_is_not_one() {
        let files = vec![PartFile {
            name: "base.nfsh".into(),
            part: 0,
            tier: tier::WHOLE,
            size: 7,
        }];
        let bytes = encode(&files, &[0, CUT, 0]);
        assert_eq!(decode(&bytes).unwrap(), (files, vec![0, CUT, 0]));
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(decode(&bad).is_err());
        let unsafe_name = encode(
            &[PartFile {
                name: "../x".into(),
                part: 0,
                tier: 0,
                size: 0,
            }],
            &[],
        );
        assert!(decode(&unsafe_name).is_err());
    }
}
