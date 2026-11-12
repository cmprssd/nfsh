//! `nfsh level`: Palm City's level graph from a store, as text. Without a zone, the
//! bundles the level loads; with one, what that zone places.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use frostbite::Result;
use frostbite::level::{Level, SubLevel, Zone, xz_bounds};
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

/// A point given as `X,Z` (metres, as Heat's positions), or `None` for anything else.
pub fn point(text: &str) -> Option<[f32; 2]> {
    let (x, z) = text.split_once(',')?;
    let p = [x.trim().parse().ok()?, z.trim().parse().ok()?];
    p.iter().all(|v: &f32| v.is_finite()).then_some(p)
}

/// One streaming zone's extent in x and z (its placements' origins).
#[derive(Debug, Clone, PartialEq)]
pub struct Extent {
    pub zone: String,
    pub lo: [f32; 2],
    pub hi: [f32; 2],
    pub placements: usize,
}

impl Extent {
    fn holds(&self, p: [f32; 2]) -> bool {
        (0..2).all(|i| self.lo[i] <= p[i] && p[i] <= self.hi[i])
    }

    fn area(&self) -> f32 {
        (self.hi[0] - self.lo[0]) * (self.hi[1] - self.lo[1])
    }

    /// How far `p` is from the extent's middle, in metres.
    fn distance(&self, p: [f32; 2]) -> f32 {
        let mid = |i: usize| (self.lo[i] + self.hi[i]) / 2.0;
        (p[0] - mid(0)).hypot(p[1] - mid(1))
    }
}

/// The zones whose extent holds `p`, smallest first (a zone the size of the city holds
/// every point and says least); when none does, the nearest `nearest` by their middle.
pub fn covering(extents: &[Extent], p: [f32; 2], nearest: usize) -> (Vec<&Extent>, bool) {
    let mut held: Vec<&Extent> = extents.iter().filter(|e| e.holds(p)).collect();
    if !held.is_empty() {
        held.sort_by(|a, b| a.area().total_cmp(&b.area()));
        return (held, true);
    }
    let mut near: Vec<&Extent> = extents.iter().collect();
    near.sort_by(|a, b| a.distance(p).total_cmp(&b.distance(p)));
    near.truncate(nearest);
    (near, false)
}

pub fn run(dir: &Path, zone: Option<&str>) -> Result<String> {
    let store = Store::open(dir)?;
    let mut level = Level::new(&store, SUPERBUNDLE)?;
    match zone {
        None => Ok(sub_levels(&level.sub_levels(ROOT)?)),
        Some(at) if point(at).is_some() => {
            let p = point(at).unwrap_or_default();
            let mut extents = Vec::new();
            for s in level.sub_levels(ROOT)? {
                if !(s.found && is_zone(&s.name)) {
                    continue;
                }
                let zone = level.zone(&s.name)?;
                if let Some((lo, hi)) = xz_bounds(&zone) {
                    let placements = zone.placements.len();
                    extents.push(Extent {
                        zone: s.name,
                        lo,
                        hi,
                        placements,
                    });
                }
            }
            Ok(zones_at(&extents, p))
        }
        Some(zone) => {
            let name = zone_bundle(zone);
            Ok(placements(&name, &level.zone(&name)?))
        }
    }
}

fn zones_at(extents: &[Extent], p: [f32; 2]) -> String {
    let mut out = String::new();
    let (zones, held) = covering(extents, p, 5);
    let _ = writeln!(
        out,
        "{} of {} zones {} x {}, z {}:",
        zones.len(),
        extents.len(),
        if held {
            "hold"
        } else {
            "are nearest to (none holds)"
        },
        p[0],
        p[1]
    );
    for e in zones {
        let _ = writeln!(
            out,
            "  {} | x {:.0}..{:.0}, z {:.0}..{:.0} | {} placements | middle {:.0} m away",
            &e.zone[ZONE.len()..],
            e.lo[0],
            e.hi[0],
            e.lo[1],
            e.hi[1],
            e.placements,
            e.distance(p)
        );
    }
    out
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

    #[test]
    fn a_point_is_two_numbers_and_a_comma() {
        assert_eq!(point("4500,2560"), Some([4500.0, 2560.0]));
        assert_eq!(point("-12.5, 30"), Some([-12.5, 30.0]));
        assert_eq!(point("495"), None);
        assert_eq!(point("levels/mainlevel/worldart_zs_495"), None);
        assert_eq!(point("1,2,3"), None);
        assert_eq!(point("nan,0"), None);
    }

    fn extent(n: u32, lo: [f32; 2], hi: [f32; 2]) -> Extent {
        Extent {
            zone: format!("{ZONE}{n}"),
            lo,
            hi,
            placements: 1,
        }
    }

    #[test]
    fn the_zones_holding_a_point_come_smallest_first() {
        let all = [
            extent(1, [-5000.0; 2], [5000.0; 2]),
            extent(2, [0.0, 0.0], [100.0, 100.0]),
            extent(3, [50.0, 50.0], [60.0, 60.0]),
            extent(4, [200.0, 0.0], [300.0, 100.0]),
        ];
        let (zones, held) = covering(&all, [55.0, 55.0], 5);
        assert!(held);
        let names: Vec<&str> = zones.iter().map(|e| &e.zone[ZONE.len()..]).collect();
        assert_eq!(names, ["3", "2", "1"]);
    }

    #[test]
    fn without_a_zone_holding_it_the_nearest_are_listed() {
        let all = [
            extent(2, [0.0, 0.0], [100.0, 100.0]),
            extent(4, [200.0, 0.0], [300.0, 100.0]),
            extent(5, [1000.0, 0.0], [1100.0, 100.0]),
        ];
        let (zones, held) = covering(&all, [180.0, 50.0], 2);
        assert!(!held);
        let names: Vec<&str> = zones.iter().map(|e| &e.zone[ZONE.len()..]).collect();
        assert_eq!(names, ["4", "2"]);
        assert_eq!(zones[0].distance([180.0, 50.0]), 70.0);
    }
}
