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
    let s = &i.saved;
    let _ = writeln!(out, "\nNot stored again, in the install's stored bytes:\n");
    let _ = writeln!(out, "| what | entries | size |\n|------|------:|-----:|");
    let rows = [
        ("identical to another entry", s.same),
        ("a chunk's end, read from the whole chunk", s.in_whole),
        ("chunks nothing names", s.dropped),
    ];
    for (what, saved) in rows {
        let _ = writeln!(
            out,
            "| {what} | {} | {} |",
            saved.entries,
            size(saved.stored)
        );
    }
    for (title, rows) in [("part", &i.parts), ("tier", &i.tiers)] {
        let _ = writeln!(out, "\n| {title} | size |\n|------|-----:|");
        for (name, bytes) in rows {
            let _ = writeln!(out, "| {name} | {} |", size(*bytes));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use frostbite::plan::{Saved, Summary};

    #[test]
    fn markdown_totals_packs_and_index() {
        let imported = Imported {
            superbundles: 2,
            bundles: 5,
            entries: 9,
            packs: vec![1 << 30, 1 << 29],
            index: 1 << 20,
            dictionary: 256 << 10,
            saved: Summary {
                same: Saved {
                    entries: 4,
                    stored: 1 << 20,
                },
                in_whole: Saved::default(),
                dropped: Saved::default(),
            },
            parts: vec![("base".into(), 1 << 30)],
            tiers: vec![("whole".into(), 1 << 30)],
        };
        assert_eq!(
            markdown(&imported),
            "2 superbundles, 5 bundles, 9 distinct entries; the index holds a 0.25 MiB \
             ebx dictionary.\n\n\
             | file | count | bytes | size |\n\
             |------|------:|------:|-----:|\n\
             | packs | 2 | 1610612736 | 1.50 GiB |\n\
             | index | 1 | 1048576 | 1.00 MiB |\n\
             | store | 3 | 1611661312 | 1.50 GiB |\n\n\
             Not stored again, in the install's stored bytes:\n\n\
             | what | entries | size |\n\
             |------|------:|-----:|\n\
             | identical to another entry | 4 | 1.00 MiB |\n\
             | a chunk's end, read from the whole chunk | 0 | 0.00 MiB |\n\
             | chunks nothing names | 0 | 0.00 MiB |\n\n\
             | part | size |\n\
             |------|-----:|\n\
             | base | 1.00 GiB |\n\n\
             | tier | size |\n\
             |------|-----:|\n\
             | whole | 1.00 GiB |\n"
        );
    }
}
