//! An installed copy of the game: `Data/` as shipped, and `Patch/` with every later
//! update. Each folder has a `layout.toc` (a signed DbObject) naming the superbundles
//! and the install packages that hold the `cas_NN.cas` archives.
//!
//! A `Patch/` superbundle lists only the bundles and chunks that changed; those replace
//! the `Data/` entries with the same name (bundles) or id (chunks), and the rest still
//! come from `Data/`. Patched entries may point back into `Data/` archives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dbobject::{self, Value};
use crate::toc::{self, TocBundle, TocChunk};
use crate::{CasRef, Result, bail, sb};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    Data,
    Patch,
}

impl Layer {
    pub fn dir(self) -> &'static str {
        match self {
            Layer::Data => "Data",
            Layer::Patch => "Patch",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Install {
    pub root: PathBuf,
    /// Superbundle names from the layouts (`Win32/globals`, ...), in layout order.
    pub superbundles: Vec<String>,
    /// Install package folders under `Win32/`, indexed by [`CasRef::package`].
    pub packages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BundleEntry {
    pub name: String,
    pub layer: Layer,
    pub bundle: sb::Bundle,
}

#[derive(Debug, Clone)]
pub struct ChunkEntry {
    pub chunk: TocChunk,
    pub layer: Layer,
}

/// A superbundle with the patch applied.
#[derive(Debug, Clone)]
pub struct Superbundle {
    pub name: String,
    pub bundles: Vec<BundleEntry>,
    pub chunks: Vec<ChunkEntry>,
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))
}

/// Prefixes an error with the file it came from.
fn in_file<T>(path: &Path, result: Result<T>) -> Result<T> {
    result.map_err(|e| crate::Error(format!("{}: {e}", path.display())))
}

/// Superbundle names and install package names from a parsed `layout.toc`.
fn read_layout(layout: &Value) -> Result<(Vec<String>, Vec<String>)> {
    let names = |list: Option<&Value>, field: &str| -> Result<Vec<String>> {
        let Some(items) = list.and_then(Value::as_list) else {
            bail!("layout has no {field} list");
        };
        items
            .iter()
            .map(|item| match item.get("name").and_then(Value::as_str) {
                Some(name) => Ok(name.to_owned()),
                None => bail!("layout {field} entry has no name"),
            })
            .collect()
    };
    let superbundles = names(layout.get("superBundles"), "superBundles")?;
    let chunks = layout
        .get("installManifest")
        .and_then(|m| m.get("installChunks"));
    let packages = names(chunks, "installChunks")?;
    Ok((superbundles, packages))
}

impl Install {
    /// Opens the install at `root` (the folder with `Data/` and `Patch/`).
    pub fn open(root: &Path) -> Result<Self> {
        let mut superbundles = Vec::new();
        let mut packages = Vec::new();
        for layer in [Layer::Data, Layer::Patch] {
            let path = root.join(layer.dir()).join("layout.toc");
            if layer == Layer::Patch && !path.exists() {
                continue;
            }
            let file = read(&path)?;
            let layout = in_file(&path, toc::signed_body(&file).and_then(dbobject::parse))?;
            let (names, layer_packages) = in_file(&path, read_layout(&layout))?;
            for name in names {
                if !superbundles.contains(&name) {
                    superbundles.push(name);
                }
            }
            if packages.is_empty() {
                packages = layer_packages;
            } else if packages != layer_packages {
                bail!(
                    "{}: install packages differ from Data/layout.toc",
                    path.display()
                );
            }
        }
        Ok(Self {
            root: root.to_owned(),
            superbundles,
            packages,
        })
    }

    pub fn cas_path(&self, cas: CasRef) -> Option<PathBuf> {
        let package = self.packages.get(usize::from(cas.package))?;
        let layer = if cas.patch { Layer::Patch } else { Layer::Data };
        Some(
            self.root
                .join(layer.dir())
                .join("Win32")
                .join(package)
                .join(format!("cas_{:02}.cas", cas.cas)),
        )
    }

    /// Reads one superbundle with the patch applied, or `None` if neither layer has its
    /// files (the layout lists every language, the install has only some).
    pub fn superbundle(&self, name: &str) -> Result<Option<Superbundle>> {
        let mut bundles: Vec<BundleEntry> = Vec::new();
        let mut chunks: Vec<ChunkEntry> = Vec::new();
        let mut found = false;
        for layer in [Layer::Data, Layer::Patch] {
            let base = self.root.join(layer.dir()).join(name);
            let toc_path = base.with_extension("toc");
            if !toc_path.exists() {
                continue;
            }
            found = true;
            let toc = in_file(&toc_path, toc::parse(&read(&toc_path)?))?;
            let sb_path = base.with_extension("sb");
            let sb_file = if toc.bundles.is_empty() {
                Vec::new()
            } else {
                read(&sb_path)?
            };
            let by_name: HashMap<String, usize> = bundles
                .iter()
                .enumerate()
                .map(|(i, b)| (b.name.clone(), i))
                .collect();
            for TocBundle { name, offset, size } in toc.bundles {
                let bytes = usize::try_from(offset)
                    .ok()
                    .and_then(|start| sb_file.get(start..)?.get(..size as usize));
                let Some(bytes) = bytes else {
                    bail!(
                        "{}: bundle {name} ({size} bytes at {offset:#x}) is past the end",
                        sb_path.display()
                    );
                };
                let bundle = in_file(
                    &sb_path,
                    sb::parse(bytes).map_err(|e| crate::Error(format!("{name}: {e}"))),
                )?;
                let entry = BundleEntry {
                    name,
                    layer,
                    bundle,
                };
                match by_name.get(&entry.name) {
                    Some(&i) => bundles[i] = entry,
                    None => bundles.push(entry),
                }
            }
            let by_id: HashMap<[u8; 16], usize> = chunks
                .iter()
                .enumerate()
                .map(|(i, c)| (c.chunk.id, i))
                .collect();
            for chunk in toc.chunks {
                let entry = ChunkEntry { chunk, layer };
                match by_id.get(&entry.chunk.id) {
                    Some(&i) => chunks[i] = entry,
                    None => chunks.push(entry),
                }
            }
        }
        Ok(found.then(|| Superbundle {
            name: name.to_owned(),
            bundles,
            chunks,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cas_paths_follow_the_reference() {
        let install = Install {
            root: PathBuf::from("/heat"),
            superbundles: vec![],
            packages: vec!["configurations/a".into(), "configurations/b".into()],
        };
        assert_eq!(
            install.cas_path(CasRef {
                patch: true,
                package: 1,
                cas: 7
            }),
            Some(PathBuf::from(
                "/heat/Patch/Win32/configurations/b/cas_07.cas"
            ))
        );
        assert_eq!(
            install.cas_path(CasRef {
                patch: false,
                package: 2,
                cas: 1
            }),
            None
        );
    }

    /// Reads the owner's install when `NFSH_INSTALL` points at it (local-ci sets it on
    /// the Windows side); otherwise there is nothing to check against.
    #[test]
    fn reads_every_superbundle_of_the_real_install() {
        let Some(root) = std::env::var_os("NFSH_INSTALL") else {
            eprintln!("NFSH_INSTALL not set; skipped");
            return;
        };
        let install = Install::open(Path::new(&root)).unwrap();
        assert_eq!(install.packages.len(), 2);
        let mut bundles = 0;
        // Each archive is checked once: there are few of them but millions of entries.
        let mut archives = std::collections::BTreeSet::new();
        for name in &install.superbundles {
            if let Some(sb) = install.superbundle(name).unwrap() {
                bundles += sb.bundles.len();
                archives.extend(sb.chunks.iter().map(|c| c.chunk.location.cas));
                for b in &sb.bundles {
                    let ebx = b.bundle.ebx.iter().map(|a| a.location.cas);
                    let res = b.bundle.res.iter().map(|r| r.asset.location.cas);
                    let chunks = b.bundle.chunks.iter().map(|c| c.location.cas);
                    archives.extend(ebx.chain(res).chain(chunks));
                }
            }
        }
        for cas in archives {
            let path = install.cas_path(cas).unwrap();
            assert!(path.exists(), "{} is missing", path.display());
        }
        assert!(bundles > 10_000, "only {bundles} bundles");
    }
}
