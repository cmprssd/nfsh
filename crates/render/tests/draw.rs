//! Draws on the real device (lavapipe on Linux, the GPU on Windows) and reads the
//! pixels back.

use frostbite::level::{Mesh, Placement, Transform, Zone};
use frostbite::mesh::Geometry;
use frostbite::texture::{Kind, Texture};
use nfsh_render::scene::{self, Section, Shading};
use nfsh_render::zone::Loaded;
use nfsh_render::{Camera, Gpu, Light, Target, ZoneRenderer};

const W: u32 = 64;
const H: u32 = 48;

/// A sun behind the camera, shining straight at what it looks at.
const LIGHT: Light = Light {
    sun: [0.0, 0.0, -1.0],
    sun_light: [1.0; 3],
    sky: [0.3, 0.3, 0.4],
    ground: 0.05,
};

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
        uvs: Some(vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
        colours: None,
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    (zone, square)
}

/// A 4×4 texture of Heat's `format`, every mip one BC1 block.
fn bc1(format: u32, block: [u8; 8]) -> Loaded {
    let texture = Texture {
        kind: Kind::Flat,
        format,
        flags: 0,
        width: 4,
        height: 4,
        layers: 1,
        mip_sizes: vec![8; 3],
        streamed: 0,
        chunk: [0; 16],
        group: "Props".into(),
    };
    (format!("bc1_{format:#x}"), texture, block.repeat(3))
}

/// Solid red: both end colours red (5:6:5), every index 0.
const RED: [u8; 8] = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0];
/// Transparent: end colours in the order that gives 1-bit alpha, every index 3.
const CLEAR: [u8; 8] = [0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// Draws the square at 5 m, as `section` makes it, with the textures from `load`;
/// returns the middle pixel and what was reported missing.
fn middle(
    section: impl Fn(&Geometry) -> Section,
    load: impl FnMut(&[u8; 16]) -> frostbite::Result<Loaded>,
) -> ([u8; 4], Vec<(String, String)>) {
    middle_of(|g| vec![section(g)], load)
}

/// [`middle`], for a mesh of several sections, each the same square.
fn middle_of(
    sections: impl Fn(&Geometry) -> Vec<Section>,
    load: impl FnMut(&[u8; 16]) -> frostbite::Result<Loaded>,
) -> ([u8; 4], Vec<(String, String)>) {
    let gpu = Gpu::new_headless().unwrap();
    let (zone, square) = square_zone(&[5.0]);
    let scene = scene::build(&zone, |_| Ok(sections(&square)));
    let renderer = ZoneRenderer::new(&gpu, &scene, Target::COLOR_FORMAT, load).unwrap();
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent, &LIGHT))
        .unwrap();
    (pixel(&rgba, W / 2, H / 2), renderer.missing.clone())
}

fn textured(texture: u8, shading: Shading) -> impl Fn(&Geometry) -> Section {
    move |g| Section {
        geometry: g.clone(),
        texture: Some([texture; 16]),
        shading,
    }
}

/// Plain grey, without a texture.
fn grey(g: &Geometry) -> Section {
    Section {
        geometry: g.clone(),
        ..Section::default()
    }
}

#[test]
fn draws_a_placed_mesh_where_the_camera_sees_it() {
    let gpu = Gpu::new_headless().unwrap();
    let (zone, square) = square_zone(&[5.0]);
    let section = Section {
        geometry: square,
        ..Section::default()
    };
    let scene = scene::build(&zone, |_| Ok(vec![section.clone()]));
    let renderer =
        ZoneRenderer::new(&gpu, &scene, Target::COLOR_FORMAT, |_| unreachable!()).unwrap();
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;

    // Straight at it: the middle is lit grey, a corner is the black background.
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent, &LIGHT))
        .unwrap();
    let middle = pixel(&rgba, W / 2, H / 2);
    assert!(middle[0] > 40 && middle[3] == 255, "middle {middle:?}");
    assert_eq!(pixel(&rgba, 0, 0), [0, 0, 0, 255]);

    // Turned away: nothing.
    let away = Camera::new([0.0; 3], 180.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &away, extent, &LIGHT))
        .unwrap();
    assert!(rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));
}

#[test]
fn a_texture_colours_what_it_is_on() {
    let (m, missing) = middle(textured(1, Shading::Colour), |_| Ok(bc1(0x37, RED)));
    assert_eq!(missing, []);
    assert!(m[0] > 100 && m[1] < 20 && m[2] < 20, "middle {m:?}");
}

#[test]
fn a_cutout_leaves_out_what_its_alpha_does() {
    let (m, missing) = middle(textured(1, Shading::Cutout), |_| Ok(bc1(0x39, CLEAR)));
    assert_eq!(missing, []);
    assert_eq!(m, [0, 0, 0, 255]);
    // An opaque texel of a cutout is drawn.
    let (m, _) = middle(textured(1, Shading::Cutout), |_| Ok(bc1(0x39, RED)));
    assert!(m[0] > 100, "middle {m:?}");
}

#[test]
fn asphalt_is_its_textures_red_as_grey_times_its_tint() {
    let asphalt = Shading::Grey {
        tint: [1.0, 1.0, 0.5],
    };
    let (m, missing) = middle(textured(1, asphalt), |_| Ok(bc1(0x37, RED)));
    assert_eq!(missing, []);
    // Red's 1 in every channel, the blue halved (the sky adds a little blue back).
    assert!(m[0] > 100 && m[0] == m[1] && m[2] < m[1], "middle {m:?}");
}

#[test]
fn decals_blend_over_what_is_under_them() {
    let blue_paint = |opacity| Shading::Decal {
        select: [1.0, 0.0, 0.0, 0.0],
        opacity,
        colour: Some([0.0, 0.0, 1.0]),
    };
    let (plain, _) = middle_of(|g| vec![grey(g)], |_| unreachable!());
    // Listed before the road it lies on, a decal is still drawn over it: half blue
    // paint (the texture's red, weighted by select, times the opacity) over grey,
    // blended in linear light (half of 213 in sRGB is 156).
    let (m, missing) = middle_of(
        |g| vec![textured(1, blue_paint(0.5))(g), grey(g)],
        |_| Ok(bc1(0x37, RED)),
    );
    assert_eq!(missing, []);
    assert!(
        m[2] > m[0] + 40 && m[0] < plain[0] - 40 && m[0] > 20,
        "middle {m:?}, grey {plain:?}"
    );
    // Opaque paint hides the grey.
    let (m, _) = middle_of(
        |g| vec![grey(g), textured(1, blue_paint(1.0))(g)],
        |_| Ok(bc1(0x37, RED)),
    );
    assert!(m[0] < 5 && m[2] > 100, "middle {m:?}");
    // Where the selected channel is 0, nothing shows.
    let (m, _) = middle_of(
        |g| vec![grey(g), textured(1, blue_paint(1.0))(g)],
        |_| Ok(bc1(0x39, CLEAR)),
    );
    assert_eq!(m, plain);
}

#[test]
fn textures_that_fail_are_drawn_grey_and_listed() {
    for load in [
        Box::new(|_: &[u8; 16]| Err(frostbite::Error("no chunk".into())))
            as Box<dyn FnMut(&[u8; 16]) -> frostbite::Result<Loaded>>,
        Box::new(|_: &[u8; 16]| Ok(bc1(0x3A, RED))),
        Box::new(|_: &[u8; 16]| {
            let (name, t, data) = bc1(0x37, RED);
            Ok((name, t, data[..8].to_vec()))
        }),
    ] {
        let (m, missing) = middle(textured(2, Shading::Colour), load);
        assert_eq!(missing.len(), 1);
        // Grey, a little blue from the sky.
        assert!(m[0] > 40 && m[0] == m[1] && m[2] > m[1], "middle {m:?}");
    }
    let (_, missing) = middle(textured(2, Shading::Colour), |_| Ok(bc1(0x3A, RED)));
    assert_eq!(
        missing,
        [(
            "bc1_0x3a".to_owned(),
            "format 0x3a not worked out".to_owned()
        )]
    );
    let (_, missing) = middle(textured(2, Shading::Colour), |_| {
        Err(frostbite::Error("no chunk".into()))
    });
    assert_eq!(
        missing,
        [(
            "02020202020202020202020202020202".to_owned(),
            "no chunk".to_owned()
        )]
    );
}

#[test]
fn an_empty_scene_draws_nothing() {
    let gpu = Gpu::new_headless().unwrap();
    let scene = scene::build(&Zone::default(), |_| unreachable!());
    let renderer =
        ZoneRenderer::new(&gpu, &scene, Target::COLOR_FORMAT, |_| unreachable!()).unwrap();
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent, &LIGHT))
        .unwrap();
    assert!(rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));
}
