//! `nfsh import`: recompresses what the install's patched tables still point at into
//! nfsh's own store (see [`frostbite::store`]), and reports how big it came out.

use std::fmt::Write as _;
use std::path::Path;

use frostbite::Result;
use frostbite::install::Install;
use frostbite::oodle::Oodle;
use frostbite::store::{self, Imported};

use crate::inventory::size;

pub fn run(root: &Path, dir: &Path) -> Result<Imported> {
    let install = Install::open(root)?;
    let oodle = Oodle::load(root)?;
    let names = &install.superbundles;
    store::import(&install, &oodle, names, dir, &mut |s| eprintln!("{s}"))
}

pub fn markdown(i: &Imported) -> String {
    let packs: u64 = i.packs.iter().sum();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} superbundles, {} bundles, {} distinct entries; the index holds a {} ebx \
         dictionary.\n",
        i.superbundles,
        i.bundles,
        i.entries,
        size(i.dictionary)
    );
    let _ = writeln!(out, "| file | count | bytes | size |");
    let _ = writeln!(out, "|------|------:|------:|-----:|");
    let rows = [
        ("packs", i.packs.len(), packs),
        ("index", 1, i.index),
        ("store", i.packs.len() + 1, packs + i.index),
    ];
    for (name, count, bytes) in rows {
        let _ = writeln!(out, "| {name} | {count} | {bytes} | {} |", size(bytes));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_totals_packs_and_index() {
        let imported = Imported {
            superbundles: 2,
            bundles: 5,
            entries: 9,
            packs: vec![1 << 30, 1 << 29],
            index: 1 << 20,
            dictionary: 256 << 10,
        };
        assert_eq!(
            markdown(&imported),
            "2 superbundles, 5 bundles, 9 distinct entries; the index holds a 0.25 MiB \
             ebx dictionary.\n\n\
             | file | count | bytes | size |\n\
             |------|------:|------:|-----:|\n\
             | packs | 2 | 1610612736 | 1.50 GiB |\n\
             | index | 1 | 1048576 | 1.00 MiB |\n\
             | store | 3 | 1611661312 | 1.50 GiB |\n"
        );
    }
}
