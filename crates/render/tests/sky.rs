//! Draws the sky on the real device (lavapipe on Linux, the GPU on Windows) behind a
//! zone and reads the pixels back.

use frostbite::level::{Mesh, Placement, Transform, Zone};
use frostbite::mesh::Geometry;
use frostbite::texture::{Kind, Texture};
use nfsh_render::scene::{self, Model, Section};
use nfsh_render::sky::{Look, Sky, Textures};
use nfsh_render::zone::Loaded;
use nfsh_render::{Camera, Gpu, Light, Target, ZoneRenderer};

const W: u32 = 64;
const H: u32 = 48;

/// A sun behind the camera, so the square in front is lit.
const LIGHT: Light = Light {
    sun: [0.0, 0.0, -1.0],
    sun_light: [1.0; 3],
    sky: [0.3, 0.3, 0.4],
    ground: 0.05,
};

/// BC1 (plain, unorm), as Heat codes it.
const BC1: u32 = 0x36;
/// Solid colours: both end colours the same (5:6:5), every index 0.
const RED: [u8; 8] = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0];
const GREEN: [u8; 8] = [0xE0, 0x07, 0xE0, 0x07, 0, 0, 0, 0];
const WHITE: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
const BLACK: [u8; 8] = [0; 8];
/// Red in its left two columns, blue in its right two: red and blue end colours,
/// indices 0, 0, 1, 1 in every row.
const RED_THEN_BLUE: [u8; 8] = [0x00, 0xF8, 0x1F, 0x00, 0x50, 0x50, 0x50, 0x50];

/// A 4×4 BC1 texture, every mip one block.
fn bc1(block: [u8; 8]) -> Loaded {
    let texture = Texture {
        kind: Kind::Flat,
        format: BC1,
        flags: 0,
        width: 4,
        height: 4,
        layers: 1,
        mip_sizes: vec![8; 3],
        streamed: 0,
        chunk: [0; 16],
        group: "Sky".into(),
    };
    ("sky".into(), texture, block.repeat(3).into())
}

/// A look whose textures' values are drawn as they are (before the tonemap), the
/// sun straight ahead without a disc.
const LOOK: Look = Look {
    scale: 1.0,
    rotation: 0.0,
    uv: [0.0, 0.0, 1.0, 1.0],
    sun: [0.0, 0.0, 1.0],
    disc_radius: 0.01,
    disc: [0.0; 3],
};

/// Draws a camera at the origin looking along +z (pitch 0) at `squares` (a 2 m grey
/// square at each distance ahead), with the sky `look` and `textures` behind.
fn draw(squares: &[f32], look: Look, textures: Textures) -> Vec<u8> {
    let gpu = Gpu::new_headless().unwrap();
    let zone = Zone {
        meshes: vec![Mesh {
            name: "square".into(),
            kind: "RigidMeshAsset".into(),
            partition: [0; 16],
        }],
        placements: squares
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
        indices: vec![0, 1, 2, 0, 2, 3],
        ..Geometry::default()
    };
    let scene = scene::build(&zone, |_| {
        Ok(Model::whole(vec![Section {
            geometry: square.clone(),
            ..Section::default()
        }]))
    });
    let zone_renderer = ZoneRenderer::new(
        &gpu,
        &scene,
        Target::COLOR_FORMAT,
        |_| unreachable!(),
        &[],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let sky = Sky::new(
        &gpu,
        Target::COLOR_FORMAT,
        vec![None, Some((look, textures))],
    )
    .unwrap();
    assert!(sky.missing.is_empty(), "{:?}", sky.missing);
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    target
        .render(|cmd| {
            zone_renderer.record(cmd, &camera, extent, &LIGHT);
            sky.record(cmd, &camera, 1);
        })
        .unwrap()
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn textures(panorama: [u8; 8], gradient: [u8; 8], mask: [u8; 8]) -> Textures {
    Textures {
        panorama: Some(bc1(panorama)),
        gradient: Some(bc1(gradient)),
        mask: Some(bc1(mask)),
    }
}

/// Mostly red, green or blue: that channel over 100, the others under 30.
fn mostly(p: [u8; 4], channel: usize) -> bool {
    (0..3).all(|c| if c == channel { p[c] > 100 } else { p[c] < 30 })
}

#[test]
fn the_panorama_is_above_the_horizon_and_the_gradient_below() {
    let rgba = draw(&[], LOOK, textures(RED, GREEN, WHITE));
    assert!(
        mostly(pixel(&rgba, W / 2, 4), 0),
        "{:?}",
        pixel(&rgba, W / 2, 4)
    );
    assert!(
        mostly(pixel(&rgba, W / 2, H - 4), 1),
        "{:?}",
        pixel(&rgba, W / 2, H - 4)
    );
}

#[test]
fn the_sky_is_drawn_only_where_nothing_else_was() {
    // A square 5 m ahead covers the middle; the sky the corners.
    let rgba = draw(&[5.0], LOOK, textures(RED, GREEN, WHITE));
    // The square's grey (a little blue from the zone's sky light), not the red sky.
    let middle = pixel(&rgba, W / 2, H / 2);
    assert!(
        middle[0] > 100 && middle[0].abs_diff(middle[1]) < 8,
        "{middle:?}"
    );
    assert!(mostly(pixel(&rgba, 2, 2), 0));
}

#[test]
fn where_the_mask_is_dark_the_gradient_shows_above_the_horizon_too() {
    let rgba = draw(&[], LOOK, textures(RED, GREEN, BLACK));
    assert!(
        mostly(pixel(&rgba, W / 2, 4), 1),
        "{:?}",
        pixel(&rgba, W / 2, 4)
    );
}

#[test]
fn the_rotation_turns_the_panorama_around_the_horizon() {
    // Looking along +z (bearing 0), u is minus the rotation: a quarter turn shows
    // the panorama's right half, three quarters its left.
    let blue = draw(
        &[],
        Look {
            rotation: 0.25,
            ..LOOK
        },
        textures(RED_THEN_BLUE, GREEN, WHITE),
    );
    assert!(
        mostly(pixel(&blue, W / 2, 4), 2),
        "{:?}",
        pixel(&blue, W / 2, 4)
    );
    let red = draw(
        &[],
        Look {
            rotation: 0.75,
            ..LOOK
        },
        textures(RED_THEN_BLUE, GREEN, WHITE),
    );
    assert!(
        mostly(pixel(&red, W / 2, 4), 0),
        "{:?}",
        pixel(&red, W / 2, 4)
    );
}

#[test]
fn the_suns_disc_is_drawn_where_the_sun_is() {
    let disc = Look {
        disc: [100.0; 3],
        ..LOOK
    };
    let black = || textures(BLACK, BLACK, WHITE);
    let ahead = draw(&[], disc, black());
    assert_eq!(pixel(&ahead, W / 2, H / 2), [255, 255, 255, 255]);
    assert_eq!(pixel(&ahead, 4, 4), [0, 0, 0, 255]);
    let behind = draw(
        &[],
        Look {
            sun: [0.0, 0.0, -1.0],
            ..disc
        },
        black(),
    );
    assert!(behind.chunks(4).all(|p| p == [0, 0, 0, 255]));
}

#[test]
fn missing_textures_draw_black_and_a_missing_mask_shows_the_panorama() {
    let rgba = draw(
        &[],
        LOOK,
        Textures {
            panorama: Some(bc1(RED)),
            gradient: None,
            mask: None,
        },
    );
    assert!(mostly(pixel(&rgba, W / 2, 4), 0));
    assert_eq!(pixel(&rgba, W / 2, H - 4), [0, 0, 0, 255]);
}
