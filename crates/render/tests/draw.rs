//! Draws on the real device (lavapipe on Linux, the GPU on Windows) and reads the
//! pixels back.

use frostbite::level::{Mesh, Placement, Transform, Zone};
use frostbite::mesh::Geometry;
use nfsh_render::{Camera, Gpu, Target, ZoneRenderer, scene};

const W: u32 = 64;
const H: u32 = 48;

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

/// A 2 m square facing -z, placed at `z` metres ahead of a camera at the origin.
fn square_zone(placed: &[f32]) -> (Zone, Geometry) {
    let zone = Zone {
        meshes: vec![Mesh {
            name: "square".into(),
            kind: "RigidMeshAsset".into(),
            partition: [0; 16],
        }],
        placements: placed
            .iter()
            .map(|&z| {
                let mut transform = Transform::IDENTITY;
                transform.trans = [0.0, 0.0, z];
                Placement { mesh: 0, transform }
            })
            .collect(),
        ..Zone::default()
    };
    let square = Geometry {
        positions: vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
        uvs: None,
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    (zone, square)
}

#[test]
fn draws_a_placed_mesh_where_the_camera_sees_it() {
    let gpu = Gpu::new_headless().unwrap();
    let (zone, square) = square_zone(&[5.0]);
    let scene = scene::build(&zone, |_| Ok(vec![square.clone()]));
    let renderer = ZoneRenderer::new(&gpu, &scene, Target::COLOR_FORMAT).unwrap();
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;

    // Straight at it: the middle is lit, a corner is the black background.
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent))
        .unwrap();
    let middle = pixel(&rgba, W / 2, H / 2);
    assert!(middle[0] > 40 && middle[3] == 255, "middle {middle:?}");
    assert_eq!(pixel(&rgba, 0, 0), [0, 0, 0, 255]);

    // Turned away: nothing.
    let away = Camera::new([0.0; 3], 180.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &away, extent))
        .unwrap();
    assert!(rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));
}

#[test]
fn an_empty_scene_draws_nothing() {
    let gpu = Gpu::new_headless().unwrap();
    let scene = scene::build(&Zone::default(), |_| unreachable!());
    let renderer = ZoneRenderer::new(&gpu, &scene, Target::COLOR_FORMAT).unwrap();
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent))
        .unwrap();
    assert!(rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));
}
