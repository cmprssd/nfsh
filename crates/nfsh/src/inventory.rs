//! `nfsh inventory`: what an install holds, measured against its size on disk.
//!
//! Every superbundle is read with the patch applied. Entries are counted per
//! superbundle and per resource type, and the bytes each entry occupies in the cas
//! archives are merged into extents, so an archive's "live" bytes are those some
//! current entry still uses; the rest is data the patch replaced.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::Path;

use frostbite::install::{Install, Superbundle};
use frostbite::sb::res_type_hash;
use frostbite::{CasRef, Location, Result};

/// Resource type names whose hash matches the types seen in Heat's bundles and whose
/// assets fit the name (see `docs/FORMATS.md`).
const RES_TYPE_NAMES: &[&str] = &[
    "AnimTrackData",
    "AssetBank",
    "AtlasTexture",
    "EnlightenDatabase",
    "EnlightenStaticDatabase",
    "IesResource",
    "MeshEmitterResource",
    "MeshSet",
    "NewWaveResource",
    "OccluderMesh",
    "PhysicsResource",
    "TerrainDecals",
    "TerrainLayerCombinations",
    "TerrainStreamingTree",
    "Texture",
    "VisualTerrain",
    "ZoneStreamerGrid",
];

fn res_type_name(hash: u32) -> Option<&'static str> {
    RES_TYPE_NAMES
        .iter()
        .copied()
        .find(|name| res_type_hash(name) == hash)
}

/// Files on disk, grouped by what they are.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileGroup {
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    pub bundles: u64,
    pub ebx: u64,
    pub res: u64,
    pub chunks: u64,
    /// Bytes the entries take in the archives (each reference counted).
    pub stored: u64,
    /// Of `stored`, the bytes of the ebx and res entries.
    pub asset_stored: u64,
    /// Decompressed size of the ebx and res entries.
    pub original: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResType {
    pub count: u64,
    pub stored: u64,
    pub original: u64,
    pub example: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// Path relative to the install.
    pub path: String,
    pub bytes: u64,
    pub live: u64,
}

#[derive(Debug, Default)]
pub struct Report {
    pub groups: BTreeMap<&'static str, FileGroup>,
    pub superbundles: Vec<(String, Counts)>,
    /// Superbundles the layout lists but the install does not have (other languages).
    pub missing: Vec<String>,
    pub res_types: BTreeMap<u32, ResType>,
    pub archives: Vec<Archive>,
    pub unique_ebx: u64,
    pub unique_res: u64,
    pub unique_chunks: u64,
}

/// What a file in the install is, from its path relative to the install.
fn group(rel: &str) -> &'static str {
    let rel = rel.replace('\\', "/");
    let top = rel.split('/').next().unwrap_or_default();
    match top {
        "Data" | "Patch" if rel.ends_with(".cas") => {
            if top == "Data" {
                "cas archives, Data/"
            } else {
                "cas archives, Patch/"
            }
        }
        "Data" | "Patch" => "tables (.toc, .sb, layout, initfs)",
        "__Installer" => "installer and redistributables",
        _ if !rel.contains('/') && (rel.ends_with(".exe") || rel.ends_with(".dll")) => {
            "executables and libraries"
        }
        _ => "other (licences, launcher files)",
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) -> Result<()> {
    let io = |e: std::io::Error| frostbite::Error(format!("{}: {e}", dir.display()));
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let entry = entry.map_err(io)?;
        let meta = entry.metadata().map_err(io)?;
        let path = entry.path();
        if meta.is_dir() {
            walk(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.push((rel.to_string_lossy().replace('\\', "/"), meta.len()));
        }
    }
    Ok(())
}

fn hash_of(value: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// Accumulates entries as superbundles are read, so each can be dropped after.
#[derive(Default)]
struct Tally {
    extents: Vec<(CasRef, u32, u32)>,
    ebx: HashSet<u64>,
    res: HashSet<u64>,
    chunks: HashSet<[u8; 16]>,
    res_types: BTreeMap<u32, ResType>,
}

impl Tally {
    fn location(&mut self, counts: &mut Counts, l: Location) {
        counts.stored += u64::from(l.size);
        self.extents.push((l.cas, l.offset, l.size));
    }

    fn add(&mut self, sb: &Superbundle) -> Counts {
        let mut c = Counts {
            bundles: sb.bundles.len() as u64,
            ..Counts::default()
        };
        for entry in &sb.bundles {
            let b = &entry.bundle;
            for a in &b.ebx {
                c.ebx += 1;
                c.original += u64::from(a.original_size);
                c.asset_stored += u64::from(a.location.size);
                self.ebx.insert(hash_of(&a.name));
                self.location(&mut c, a.location);
            }
            for r in &b.res {
                let a = &r.asset;
                c.res += 1;
                c.original += u64::from(a.original_size);
                c.asset_stored += u64::from(a.location.size);
                self.res.insert(hash_of(&a.name));
                self.location(&mut c, a.location);
                let t = self.res_types.entry(r.res_type).or_default();
                t.count += 1;
                t.stored += u64::from(a.location.size);
                t.original += u64::from(a.original_size);
                if t.example.is_empty() {
                    t.example = a.name.clone();
                }
            }
            for ch in &b.chunks {
                c.chunks += 1;
                self.chunks.insert(ch.id);
                self.location(&mut c, ch.location);
            }
        }
        for ch in &sb.chunks {
            c.chunks += 1;
            self.chunks.insert(ch.chunk.id);
            self.location(&mut c, ch.chunk.location);
        }
        c
    }
}

/// Bytes covered by the extents in each archive, overlaps counted once.
pub fn live_bytes(mut extents: Vec<(CasRef, u32, u32)>) -> BTreeMap<CasRef, u64> {
    extents.sort_unstable();
    let mut out = BTreeMap::new();
    let mut current: Option<(CasRef, u64, u64)> = None;
    let mut flush = |run: Option<(CasRef, u64, u64)>| {
        if let Some((cas, start, end)) = run {
            *out.entry(cas).or_insert(0) += end - start;
        }
    };
    for (cas, offset, size) in extents {
        let (start, end) = (u64::from(offset), u64::from(offset) + u64::from(size));
        match &mut current {
            Some((c, _, run_end)) if *c == cas && start <= *run_end => {
                *run_end = (*run_end).max(end)
            }
            _ => flush(current.replace((cas, start, end))),
        }
    }
    flush(current);
    out
}

pub fn scan(root: &Path) -> Result<Report> {
    let install = Install::open(root)?;
    let mut report = Report::default();

    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    let sizes: BTreeMap<String, u64> = files.into_iter().collect();
    for (rel, &bytes) in &sizes {
        let g = report.groups.entry(group(rel)).or_default();
        g.files += 1;
        g.bytes += bytes;
    }

    let mut tally = Tally::default();
    for name in &install.superbundles {
        match install.superbundle(name)? {
            Some(sb) => report.superbundles.push((name.clone(), tally.add(&sb))),
            None => report.missing.push(name.clone()),
        }
    }
    report.unique_ebx = tally.ebx.len() as u64;
    report.unique_res = tally.res.len() as u64;
    report.unique_chunks = tally.chunks.len() as u64;
    report.res_types = std::mem::take(&mut tally.res_types);

    let mut live_by_path = BTreeMap::new();
    for (cas, live) in live_bytes(std::mem::take(&mut tally.extents)) {
        let rel = install.cas_path(cas).and_then(|p| {
            Some(
                p.strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        });
        match rel {
            Some(rel) if sizes.contains_key(&rel) => live_by_path.insert(rel, live),
            _ => {
                return Err(frostbite::Error(format!(
                    "entries point at a missing archive: {cas:?}"
                )));
            }
        };
    }
    for (rel, &bytes) in sizes.iter().filter(|(rel, _)| rel.ends_with(".cas")) {
        let live = live_by_path.get(rel).copied().unwrap_or(0);
        if live > bytes {
            return Err(frostbite::Error(format!(
                "{rel}: entries cover {live} bytes of a {bytes}-byte file"
            )));
        }
        report.archives.push(Archive {
            path: rel.clone(),
            bytes,
            live,
        });
    }
    Ok(report)
}

/// Bytes as GiB (or MiB below one GiB), with two decimals.
fn size(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let mib = bytes as f64 / MIB;
    if mib >= 1024.0 {
        format!("{:.2} GiB", mib / 1024.0)
    } else {
        format!("{mib:.2} MiB")
    }
}

fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        "-".into()
    } else {
        format!("{:.1}%", part as f64 * 100.0 / whole as f64)
    }
}

impl Report {
    pub fn install_bytes(&self) -> u64 {
        self.groups.values().map(|g| g.bytes).sum()
    }

    pub fn markdown(&self) -> String {
        let mut o = String::new();
        let total = self.install_bytes();
        let cas_bytes: u64 = self.archives.iter().map(|a| a.bytes).sum();
        let live: u64 = self.archives.iter().map(|a| a.live).sum();
        let files: u64 = self.groups.values().map(|g| g.files).sum();
        let _ = writeln!(o, "## On disk\n");
        let _ = writeln!(o, "{files} files, {} ({total} bytes).\n", size(total));
        let _ = writeln!(o, "| What | Files | Size | Share |\n|---|---:|---:|---:|");
        let mut groups: Vec<_> = self.groups.iter().collect();
        groups.sort_by_key(|(_, g)| std::cmp::Reverse(g.bytes));
        for (name, g) in groups {
            let _ = writeln!(
                o,
                "| {name} | {} | {} | {} |",
                g.files,
                size(g.bytes),
                percent(g.bytes, total)
            );
        }

        let _ = writeln!(o, "\n## Live and replaced data\n");
        let _ = writeln!(
            o,
            "Of {} in cas archives, current entries (patch applied) use {} ({}); the other {} is \
             data the patch replaced, which nothing reads any more.\n",
            size(cas_bytes),
            size(live),
            percent(live, cas_bytes),
            size(cas_bytes - live)
        );
        let _ = writeln!(
            o,
            "| Archives | Count | Size | Live | Live share |\n|---|---:|---:|---:|---:|"
        );
        let mut folders: BTreeMap<&str, (u64, u64, u64)> = BTreeMap::new();
        for a in &self.archives {
            let folder = a.path.rsplit_once('/').map_or("", |(dir, _)| dir);
            let f = folders.entry(folder).or_default();
            f.0 += 1;
            f.1 += a.bytes;
            f.2 += a.live;
        }
        for (folder, (count, bytes, live)) in folders {
            let _ = writeln!(
                o,
                "| `{folder}` | {count} | {} | {} | {} |",
                size(bytes),
                size(live),
                percent(live, bytes)
            );
        }

        let sum = self
            .superbundles
            .iter()
            .fold(Counts::default(), |mut s, (_, c)| {
                s.bundles += c.bundles;
                s.ebx += c.ebx;
                s.res += c.res;
                s.chunks += c.chunks;
                s.stored += c.stored;
                s.asset_stored += c.asset_stored;
                s.original += c.original;
                s
            });
        let _ = writeln!(o, "\n## Superbundles\n");
        let _ = writeln!(
            o,
            "{} superbundles hold {} bundles listing {} ebx, {} res and {} chunk entries; distinct by \
             name or id: {} ebx, {} res, {} chunks. Stored is the bytes the entries take in the \
             archives, counting every reference (shared entries count once per bundle).\n",
            self.superbundles.len(),
            sum.bundles,
            sum.ebx,
            sum.res,
            sum.chunks,
            self.unique_ebx,
            self.unique_res,
            self.unique_chunks
        );
        let _ = writeln!(
            o,
            "| Superbundle | Bundles | Ebx | Res | Chunks | Stored |\n|---|---:|---:|---:|---:|---:|"
        );
        let mut rows: Vec<_> = self.superbundles.iter().collect();
        rows.sort_by_key(|(_, c)| std::cmp::Reverse(c.stored));
        for (name, c) in rows {
            let _ = writeln!(
                o,
                "| `{name}` | {} | {} | {} | {} | {} |",
                c.bundles,
                c.ebx,
                c.res,
                c.chunks,
                size(c.stored)
            );
        }
        if !self.missing.is_empty() {
            let _ = writeln!(
                o,
                "\nListed in the layout but not installed: {}.",
                self.missing.join(", ")
            );
        }

        let _ = writeln!(o, "\n## Resource types\n");
        let _ = writeln!(
            o,
            "Res entries by type. Names are given only where the type hash is confirmed; the \
             others show the hash and one asset that carries it. All ebx and res entries take {} \
             stored and {} decompressed; chunks (bulk data such as texture mips) have no \
             decompressed size in the tables.\n",
            size(sum.asset_stored),
            size(sum.original)
        );
        let _ = writeln!(
            o,
            "| Type | Entries | Stored | Decompressed | Example |\n|---|---:|---:|---:|---|"
        );
        let mut types: Vec<_> = self.res_types.iter().collect();
        types.sort_by_key(|(_, t)| std::cmp::Reverse(t.stored));
        for (hash, t) in types {
            let name = res_type_name(*hash).map_or_else(|| format!("`{hash:08x}`"), str::to_owned);
            let _ = writeln!(
                o,
                "| {name} | {} | {} | {} | `{}` |",
                t.count,
                size(t.stored),
                size(t.original),
                t.example
            );
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: CasRef = CasRef {
        patch: false,
        package: 0,
        cas: 1,
    };
    const B: CasRef = CasRef {
        patch: true,
        package: 1,
        cas: 1,
    };

    #[test]
    fn live_bytes_merge_overlaps_per_archive() {
        let live = live_bytes(vec![
            (A, 100, 50),
            (B, 0, 10),
            (A, 0, 10),
            (A, 120, 50),
            (A, 100, 50),
            (A, 10, 5),
        ]);
        // A: [0,15) and [100,170); B: [0,10).
        assert_eq!(live, BTreeMap::from([(A, 85), (B, 10)]));
        assert!(live_bytes(vec![]).is_empty());
    }

    #[test]
    fn files_are_grouped_by_what_they_are() {
        assert_eq!(
            group("Data/Win32/configurations/x/cas_01.cas"),
            "cas archives, Data/"
        );
        assert_eq!(
            group("Patch/Win32/configurations/x/cas_01.cas"),
            "cas archives, Patch/"
        );
        assert_eq!(
            group("Patch/Win32/globals.toc"),
            "tables (.toc, .sb, layout, initfs)"
        );
        assert_eq!(group("NeedForSpeedHeat.exe"), "executables and libraries");
        assert_eq!(
            group("Core/Activation64.dll"),
            "other (licences, launcher files)"
        );
        assert_eq!(
            group("__Installer/Touchup.exe"),
            "installer and redistributables"
        );
    }

    #[test]
    fn confirmed_res_type_names_resolve() {
        assert_eq!(res_type_name(0x6bde_20ba), Some("Texture"));
        assert_eq!(res_type_name(0x4175_9364), Some("PhysicsResource"));
        assert_eq!(res_type_name(0x0e2b_02f7), None);
    }

    #[test]
    fn markdown_reports_live_share_and_sizes() {
        let mut report = Report::default();
        report.groups.insert(
            "cas archives, Data/",
            FileGroup {
                files: 1,
                bytes: 4 << 30,
            },
        );
        report.archives.push(Archive {
            path: "Data/Win32/p/cas_01.cas".into(),
            bytes: 4 << 30,
            live: 1 << 30,
        });
        report.res_types.insert(
            0x6bde_20ba,
            ResType {
                count: 2,
                stored: 3 << 20,
                original: 6 << 20,
                example: "tex".into(),
            },
        );
        let md = report.markdown();
        assert!(md.contains("1 files, 4.00 GiB"), "{md}");
        assert!(
            md.contains("current entries (patch applied) use 1.00 GiB (25.0%)"),
            "{md}"
        );
        assert!(
            md.contains("| `Data/Win32/p` | 1 | 4.00 GiB | 1.00 GiB | 25.0% |"),
            "{md}"
        );
        assert!(
            md.contains("| Texture | 2 | 3.00 MiB | 6.00 MiB | `tex` |"),
            "{md}"
        );
    }
}
