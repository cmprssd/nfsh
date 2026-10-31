//! `nfsh level`: Palm City's level graph from a store, as text. Without a zone, the
//! bundles the level loads; with one, what that zone places.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use frostbite::Result;
use frostbite::level::{Level, SubLevel, Zone};
use frostbite::store::Store;

/// The level, its superbundle, and the prefix its zones' bundle names share.
pub const ROOT: &str = "levels/mainlevel/mainlevel";
pub const SUPERBUNDLE: &str = "Win32/levels/mainlevel/mainlevel";
const ZONE: &str = "levels/mainlevel/worldart_zs_";

/// The zone bundle `zone` names: a number (`495`) or a whole bundle name.
pub fn zone_bundle(zone: &str) -> String {
    match zone.bytes().all(|b| b.is_ascii_digit()) {
        true => format!("{ZONE}{zone}"),
        false => zone.to_owned(),
    }
}

/// Whether a sub-level is a streaming zone itself (not one of its lighting variants
/// or regions): `worldart_zs_<n>`.
pub fn is_zone(name: &str) -> bool {
    name.strip_prefix(ZONE)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub fn run(dir: &Path, zone: Option<&str>) -> Result<String> {
    let store = Store::open(dir)?;
    let mut level = Level::new(&store, SUPERBUNDLE)?;
    match zone {
        None => Ok(sub_levels(&level.sub_levels(ROOT)?)),
        Some(zone) => {
            let name = zone_bundle(zone);
            Ok(placements(&name, &level.zone(&name)?))
        }
    }
}

fn sub_levels(all: &[SubLevel]) -> String {
    let mut out = String::new();
    let found: Vec<&SubLevel> = all.iter().filter(|s| s.found).collect();
    let mut zones: Vec<&str> = found
        .iter()
        .filter(|s| is_zone(&s.name))
        .map(|s| &s.name[ZONE.len()..])
        .collect();
    zones.sort_by_key(|z| z.parse::<u32>().unwrap_or(u32::MAX));
    let _ = writeln!(
        out,
        "{ROOT}: {} bundles, {} streaming zones",
        found.len(),
        zones.len()
    );
    let mut depths: BTreeMap<usize, usize> = BTreeMap::new();
    for s in &found {
        *depths.entry(s.depth).or_default() += 1;
    }
    for (depth, n) in depths {
        let _ = writeln!(out, "  depth {depth}: {n} bundles");
    }
    let _ = writeln!(out, "\nsub-levels (not zones or their variants):");
    for s in found.iter().filter(|s| !s.name.starts_with(ZONE)) {
        let _ = writeln!(out, "  {}{}", "  ".repeat(s.depth), s.name);
    }
    let _ = writeln!(out, "\nzones: {}", zones.join(" "));
    let missing = all.len() - found.len();
    if missing > 0 {
        let _ = writeln!(out, "\n{missing} links name a bundle the store lacks:");
        for s in all.iter().filter(|s| !s.found) {
            let _ = writeln!(out, "  {} (from {})", s.name, s.from);
        }
    }
    out
}

fn placements(name: &str, zone: &Zone) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{name}: {} placements of {} meshes",
        zone.placements.len(),
        zone.meshes.len()
    );
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for m in &zone.meshes {
        *kinds.entry(&m.kind).or_default() += 1;
    }
    for (kind, n) in kinds {
        let _ = writeln!(out, "  {n} {kind}");
    }
    if zone.missing > 0 {
        let _ = writeln!(out, "{} references the store can't resolve", zone.missing);
    }
    if !zone.skipped.is_empty() {
        let _ = writeln!(out, "\nnot followed:");
        let mut skipped: Vec<_> = zone.skipped.iter().collect();
        skipped.sort_by_key(|&(what, n)| (std::cmp::Reverse(*n), what));
        for (what, n) in skipped {
            let _ = writeln!(out, "  {n} {what}");
        }
    }
    let _ = writeln!(
        out,
        "\nplacements (position; right, up, forward axes; mesh):"
    );
    let v = |v: [f32; 3]| format!("{:.3} {:.3} {:.3}", v[0], v[1], v[2]);
    for p in &zone.placements {
        let t = &p.transform;
        let _ = writeln!(
            out,
            "  {} | {} | {} | {} | {}",
            v(t.trans),
            v(t.right),
            v(t.up),
            v(t.forward),
            zone.meshes[p.mesh].name
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_are_named_by_number_or_bundle() {
        assert_eq!(zone_bundle("495"), "levels/mainlevel/worldart_zs_495");
        assert_eq!(
            zone_bundle("levels/mainlevel/worldart_zs_mainlevel_region_city"),
            "levels/mainlevel/worldart_zs_mainlevel_region_city"
        );
        assert!(is_zone("levels/mainlevel/worldart_zs_495"));
        assert!(!is_zone("levels/mainlevel/worldart_zs_495_midday"));
        assert!(!is_zone(
            "levels/mainlevel/worldart_zs_mainlevel_region_city"
        ));
        assert!(!is_zone("levels/mainlevel/worldart_zs_"));
    }
}
