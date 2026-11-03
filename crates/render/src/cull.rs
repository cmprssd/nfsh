//! Culling and LODs: each placement's box in world space, made once from its mesh's
//! vertices, tested against the camera's view each frame, so only the placements
//! that can be seen are drawn, each at the LOD its distance chooses, and none that
//! cover too little of the screen. Conservative: a box that cannot be read (no
//! vertices, not finite) is always drawn, at its first LOD.

use crate::scene::{Instance, Lods, Scene};

/// A box in world space: its least and greatest corner.
pub type Bounds = [[f32; 3]; 2];

/// One box per instance of `scene`: around every vertex the draws that use it place
/// with it (the sections of one mesh share its placements); `None` where no draw
/// uses the instance or its box is not finite.
pub fn instance_bounds(scene: &Scene) -> Vec<Option<Bounds>> {
    let mut bounds: Vec<Option<Bounds>> = vec![None; scene.instances.len()];
    for d in &scene.draws {
        let Some(local) = draw_bounds(scene, d.first_index, d.index_count, d.vertex_offset) else {
            continue;
        };
        let first = d.first_instance as usize;
        let last = (first + d.instance_count as usize).min(scene.instances.len());
        for (b, instance) in bounds[first..last]
            .iter_mut()
            .zip(&scene.instances[first..last])
        {
            let world = place(&local, instance);
            *b = Some(b.map_or(world, |b| union(&b, &world)));
        }
    }
    bounds
        .into_iter()
        .map(|b| b.filter(|b| b.iter().flatten().all(|v| v.is_finite())))
        .collect()
}

/// The box around the vertices one draw's indices name, in its mesh's space; `None`
/// when it names none that exist.
fn draw_bounds(scene: &Scene, first: u32, count: u32, offset: i32) -> Option<Bounds> {
    let indices = scene
        .indices
        .get(first as usize..first as usize + count as usize)?;
    let mut b: Option<Bounds> = None;
    for &i in indices {
        let Some(&p) = usize::try_from(i64::from(i) + i64::from(offset))
            .ok()
            .and_then(|i| scene.positions.get(i))
        else {
            continue;
        };
        b = Some(b.map_or([p, p], |b| union(&b, &[p, p])));
    }
    b
}

fn union(a: &Bounds, b: &Bounds) -> Bounds {
    [
        [0, 1, 2].map(|k| a[0][k].min(b[0][k])),
        [0, 1, 2].map(|k| a[1][k].max(b[1][k])),
    ]
}

/// The box around `local` as `instance` places it (as the shader does: the vertex's
/// x, y and z times the instance's right, up and forward, plus its origin).
pub fn place(local: &Bounds, instance: &Instance) -> Bounds {
    let axes = [0, 4, 8].map(|at| [instance[at], instance[at + 1], instance[at + 2]]);
    let origin = [instance[3], instance[7], instance[11]];
    let centre = [0, 1, 2].map(|k| (local[0][k] + local[1][k]) / 2.0);
    let half = [0, 1, 2].map(|k| (local[1][k] - local[0][k]) / 2.0);
    let mid = [0, 1, 2].map(|k| origin[k] + (0..3).map(|j| centre[j] * axes[j][k]).sum::<f32>());
    let reach = [0, 1, 2].map(|k| (0..3).map(|j| half[j] * axes[j][k].abs()).sum::<f32>());
    [
        [0, 1, 2].map(|k| mid[k] - reach[k]),
        [0, 1, 2].map(|k| mid[k] + reach[k]),
    ]
}

/// The camera's view as the planes a visible point is on the inner side of: left,
/// right, top, bottom and near (the far end is at infinity).
pub struct Frustum([[f32; 4]; 5]);

impl Frustum {
    /// From a column-major view-projection matrix (clip = m × (x, y, z, 1)), with
    /// depth `z_near / distance` as [`crate::Camera::view_proj`] makes it: inside is
    /// -w ≤ x ≤ w, -w ≤ y ≤ w and z ≤ w.
    pub fn new(m: &[[f32; 4]; 4]) -> Self {
        let row = |r: usize| [m[0][r], m[1][r], m[2][r], m[3][r]];
        let (x, y, z, w) = (row(0), row(1), row(2), row(3));
        let add = |a: [f32; 4], b: [f32; 4], s: f32| [0, 1, 2, 3].map(|k| a[k] + s * b[k]);
        Self([
            add(w, x, 1.0),
            add(w, x, -1.0),
            add(w, y, 1.0),
            add(w, y, -1.0),
            add(w, z, -1.0),
        ])
    }

    /// Whether any of `b` can be in view: false only when it lies wholly outside one
    /// plane (a box near a corner of the view may be kept though unseen).
    pub fn sees(&self, b: &Bounds) -> bool {
        self.0.iter().all(|p| {
            // The corner furthest along the plane's normal.
            let far = [0, 1, 2].map(|k| if p[k] >= 0.0 { b[1][k] } else { b[0][k] });
            // A box that cannot be read (NaN) is kept.
            let along = p[0] * far[0] + p[1] * far[1] + p[2] * far[2] + p[3];
            along >= 0.0 || along.is_nan()
        })
    }
}

/// Which LOD a placement with the box `b` (in world space) and `lods` is drawn at,
/// seen from `eye` by a view whose half width and height have the tangents
/// `tangents` ([`crate::Camera::tangents`]); `None` when it covers less of the screen
/// than its lod group's cut. See `docs/DECISIONS.md`, "LODs".
///
/// The distance is to the nearest point of the box (0 inside it: a road piece is
/// near wherever on it the camera is); the share of the screen is that of the
/// sphere around the box, its radius times the mesh's cull scale, as if it were
/// ahead at that distance, in percent: the cut read as a fraction would drop a
/// barrel 14 m away. A box that cannot be read draws the first LOD.
pub fn choose(b: Option<&Bounds>, lods: &Lods, eye: [f32; 3], tangents: [f32; 2]) -> Option<u8> {
    let Some(b) = b else {
        return Some(0);
    };
    let outside = [0, 1, 2].map(|k| (b[0][k] - eye[k]).max(eye[k] - b[1][k]).max(0.0));
    let distance = outside.iter().map(|v| v * v).sum::<f32>().sqrt();
    let s = &lods.switches;
    if s.cull_area > 0.0 {
        let radius = (0..3)
            .map(|k| (b[1][k] - b[0][k]).powi(2))
            .sum::<f32>()
            .sqrt()
            / 2.0;
        let radius = radius * s.cull_scale;
        // From inside its box a mesh is drawn.
        if distance > 0.0 {
            let screen = 4.0 * tangents[0] * tangents[1] * distance * distance;
            let percent = 100.0 * std::f32::consts::PI * radius * radius / screen;
            if percent < s.cull_area {
                return None;
            }
        }
    }
    let lod = s.lod(distance, usize::from(lods.count));
    Some(u8::try_from(lod).unwrap_or(u8::MAX))
}

/// The runs of consecutive drawn instances among `first..first + count`, as
/// (first instance, count); `drawn` says by instance.
pub fn runs(first: u32, count: u32, drawn: impl Fn(u32) -> bool) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for i in first..first.saturating_add(count) {
        if !drawn(i) {
            continue;
        }
        match out.last_mut() {
            Some((start, n)) if *start + *n == i => *n += 1,
            _ => out.push((i, 1)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Camera;

    const IDENTITY: Instance = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    #[test]
    fn a_placement_moves_and_turns_its_box() {
        let unit = [[-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]];
        // Moved to (10, 20, 30).
        let mut moved = IDENTITY;
        (moved[3], moved[7], moved[11]) = (10.0, 20.0, 30.0);
        assert_eq!(
            place(&unit, &moved),
            [[9.0, 18.0, 27.0], [11.0, 22.0, 33.0]]
        );
        // A quarter turn about up: right is +z (x goes to z), forward is -x.
        let turned = [0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        assert_eq!(place(&unit, &turned), [[-3.0, -2.0, -1.0], [3.0, 2.0, 1.0]]);
        // Scaled by 2 along right.
        let mut wide = IDENTITY;
        wide[0] = 2.0;
        assert_eq!(place(&unit, &wide), [[-2.0, -2.0, -3.0], [2.0, 2.0, 3.0]]);
    }

    /// At the origin looking along +z (+x on its right), 55° tall, 16:9.
    fn camera() -> Camera {
        Camera::new([0.0; 3], 0.0, 0.0, 1280, 720)
    }

    fn cube(at: [f32; 3]) -> Bounds {
        [at.map(|v| v - 1.0), at.map(|v| v + 1.0)]
    }

    #[test]
    fn the_frustum_keeps_what_is_ahead_and_drops_what_is_not() {
        let c = camera();
        let f = Frustum::new(&c.view_proj());
        assert!(f.sees(&cube([0.0, 0.0, 10.0])));
        assert!(f.sees(&cube([0.0, 0.0, 5000.0])), "no far plane");
        assert!(!f.sees(&cube([0.0, 0.0, -10.0])), "behind");
        // Far to one side, and high above, at the same distance ahead.
        assert!(!f.sees(&cube([100.0, 0.0, 10.0])));
        assert!(!f.sees(&cube([-100.0, 0.0, 10.0])));
        assert!(!f.sees(&cube([0.0, 100.0, 10.0])));
        // Just inside the right edge: 16:9 at 55° tall sees x up to 0.93 z.
        assert!(f.sees(&cube([9.0, 0.0, 10.0])));
        assert!(f.sees(&cube([0.0; 3])));
        // A box that can't be read is drawn.
        assert!(f.sees(&[[f32::NAN; 3], [f32::NAN; 3]]));
    }

    #[test]
    fn every_corner_the_frustum_keeps_projects_near_the_screen() {
        // Checked against Camera::project: a cube the frustum drops has no corner on
        // screen, for cubes on a grid around the camera.
        let mut c = camera();
        (c.yaw, c.pitch) = (35.0, -20.0);
        let f = Frustum::new(&c.view_proj());
        let mut dropped = 0;
        for x in -10..=10 {
            for y in -10..=10 {
                for z in -10..=10 {
                    let b = cube([x as f32 * 7.0, y as f32 * 7.0, z as f32 * 7.0]);
                    if f.sees(&b) {
                        continue;
                    }
                    dropped += 1;
                    for corner in 0..8 {
                        let p = [0, 1, 2].map(|k| b[(corner >> k) & 1][k]);
                        if let Some((px, py, _)) = c.project(p) {
                            let on = (0.0..=1280.0).contains(&px) && (0.0..=720.0).contains(&py);
                            assert!(!on, "{p:?} is on screen at ({px}, {py})");
                        }
                    }
                }
            }
        }
        assert!(dropped > 5000, "only {dropped} of 9261 dropped");
    }

    #[test]
    fn runs_join_neighbouring_drawn_instances() {
        let v = [true, true, false, true, false, false, true, true, true];
        let drawn = |i: u32| v.get(i as usize).copied().unwrap_or(true);
        assert_eq!(runs(0, 9, drawn), [(0, 2), (3, 1), (6, 3)]);
        assert_eq!(runs(4, 2, drawn), []);
        assert_eq!(runs(7, 3, drawn), [(7, 3)]);
    }

    /// architecture_m at LodScale 2 with its cut (0.001 of the screen), three LODs.
    fn stand() -> Lods {
        Lods {
            switches: frostbite::mesh::Switches {
                distances: [100.0, 200.0, 1600.0, 1700.0, 2020.0, 2050.0],
                cull_area: 0.001,
                cull_scale: 1.0,
            },
            count: 3,
        }
    }

    #[test]
    fn the_distance_to_the_box_chooses_the_lod() {
        let t = camera().tangents();
        let lods = stand();
        // Cubes of side 2: the nearest point is 1 m nearer than the middle.
        let at = |z: f32| Some(cube([0.0, 0.0, z]));
        assert_eq!(choose(at(10.0).as_ref(), &lods, [0.0; 3], t), Some(0));
        assert_eq!(choose(at(100.9).as_ref(), &lods, [0.0; 3], t), Some(0));
        assert_eq!(choose(at(101.0).as_ref(), &lods, [0.0; 3], t), Some(1));
        // Behind or to the side counts the same: only the distance.
        assert_eq!(choose(at(-150.0).as_ref(), &lods, [0.0; 3], t), Some(1));
        assert_eq!(choose(at(250.0).as_ref(), &lods, [0.0; 3], t), Some(2));
        // A road 400 m long, the camera above one end: near.
        let road = [[-5.0, -1.0, -10.0], [5.0, 0.0, 390.0]];
        assert_eq!(choose(Some(&road), &lods, [0.0, 2.0, 0.0], t), Some(0));
        // The last LOD on, while the box covers enough of the screen.
        assert_eq!(choose(at(1500.0).as_ref(), &lods, [0.0; 3], t), None);
        let big = [[-5.0, -5.0, 1795.0], [5.0, 5.0, 1805.0]];
        assert_eq!(choose(Some(&big), &lods, [0.0; 3], t), Some(2));
        // A box that can't be read: the first LOD.
        assert_eq!(choose(None, &lods, [0.0; 3], t), Some(0));
        // One LOD and no switches: drawn whatever the distance and size.
        let far = cube([0.0, 0.0, 1e6]);
        assert_eq!(choose(Some(&far), &Lods::ONE, [0.0; 3], t), Some(0));
    }

    #[test]
    fn a_box_too_small_on_screen_is_not_drawn() {
        let c = camera();
        let t = c.tangents();
        let lods = stand();
        // A cube of side 2 (radius √3) covers 100 π·3 / (4 tx ty d²) percent of the
        // screen: the cut at 0.001 falls at d ≈ 699 m with 16:9 at 55°, d from its
        // nearest point (the middle is 1 m further).
        let tx_ty = t[0] * t[1];
        let edge = (100.0 * std::f32::consts::PI * 3.0 / (4.0 * tx_ty * 0.001)).sqrt();
        assert!((edge - 699.0).abs() < 1.0, "{edge}");
        let at = |z: f32| cube([0.0, 0.0, z + 1.0]);
        assert_eq!(choose(Some(&at(edge - 0.5)), &lods, [0.0; 3], t), Some(2));
        assert_eq!(choose(Some(&at(edge + 0.5)), &lods, [0.0; 3], t), None);
        // A cull scale of 2 doubles the size it is measured at.
        let mut scaled = lods;
        scaled.switches.cull_scale = 2.0;
        assert_eq!(choose(Some(&at(edge + 0.5)), &scaled, [0.0; 3], t), Some(2));
        // From inside the box it is drawn.
        let mut huge = lods;
        huge.switches.cull_area = 10.0;
        assert_eq!(choose(Some(&cube([0.0; 3])), &huge, [0.0; 3], t), Some(0));
        // A wider view makes the same box a smaller share of it.
        let mut wide = c;
        wide.fov_y = 90.0;
        assert_eq!(
            choose(Some(&at(edge - 0.5)), &lods, [0.0; 3], wide.tangents()),
            None
        );
    }
}
