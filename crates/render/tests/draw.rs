//! Draws on the real device (lavapipe on Linux, the GPU on Windows) and reads the
//! pixels back.

use frostbite::level::{Mesh, Placement, Transform, Zone};
use frostbite::mesh::{Frame, Geometry};
use frostbite::texture::{Kind, Texture};
use nfsh_render::scene::{self, Asphalt, Edge, Section, Shading};
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
        indices: vec![0, 1, 2, 0, 2, 3],
        ..Geometry::default()
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
    let renderer = ZoneRenderer::new(&gpu, &scene, Target::COLOR_FORMAT, load, Vec::new()).unwrap();
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
        ..Section::default()
    }
}

/// Plain grey, without a texture.
fn grey(g: &Geometry) -> Section {
    Section {
        geometry: g.clone(),
        ..Section::default()
    }
}

/// Plain grey, every vertex taking `normal` as the mesh's own.
fn grey_facing(normal: [f32; 3]) -> impl Fn(&Geometry) -> Section {
    move |g| {
        let mut g = g.clone();
        let frame = Frame {
            normal,
            tangent: [1.0, 0.0, 0.0],
            sign: 1.0,
        };
        g.frames = Some(vec![frame; g.positions.len()]);
        grey(&g)
    }
}

#[test]
fn a_mesh_is_lit_by_its_own_normals() {
    let no_texture = |_: &[u8; 16]| unreachable!();
    let (flat, _) = middle(grey, no_texture);
    // Facing the sun, as the square itself does: lit the same.
    assert_eq!(middle(grey_facing([0.0, 0.0, -1.0]), no_texture).0, flat);
    // Turned 60 degrees from it: half the sun, the same sky.
    let s = 60f32.to_radians();
    let (turned, _) = middle(grey_facing([s.sin(), 0.0, -s.cos()]), no_texture);
    assert_eq!((flat, turned), ([213, 213, 215, 255], [184, 184, 188, 255]));
    // Pointing to the side not seen: turned to the side seen.
    assert_eq!(middle(grey_facing([0.0, 0.0, 1.0]), no_texture).0, flat);
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
    let renderer = ZoneRenderer::new(
        &gpu,
        &scene,
        Target::COLOR_FORMAT,
        |_| unreachable!(),
        Vec::new(),
    )
    .unwrap();
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
fn asphalt_shows_its_damaged_layer_where_the_mask_is_dark() {
    const WHITE: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
    const BLACK: [u8; 8] = [0; 8];
    let asphalt = Shading::Asphalt(Asphalt {
        tint: [[0.4; 3]; 2],
        tiling: [[1.0; 2]; 2],
        blend: [0.17, 0.88],
        tyre: [0.5, 0.88],
        tyre_tint: [0.5; 3],
        bias: 0.21,
        detail: [[0.5; 3]; 5],
        edge: None,
    });
    let section = |g: &Geometry| Section {
        geometry: Geometry {
            uvs2: g.uvs.clone(),
            ..g.clone()
        },
        texture: Some([1; 16]),
        layers: [2, 3, 4, 5, 0, 0].map(|t| (t > 0).then_some([t; 16])),
        shading: asphalt,
    };
    // A white clean layer, a black damaged one, a full height, no noise; the mask's
    // red is `mask`.
    let draw = |mask| {
        middle(section, move |guid| {
            Ok(bc1(
                0x36,
                match guid[0] {
                    1 | 3 => WHITE,
                    4 => mask,
                    _ => BLACK,
                },
            ))
        })
    };
    let (clean, missing) = draw(RED);
    assert_eq!(missing, []);
    let (damaged, _) = draw(BLACK);
    // White tinted 0.4 (203 here), against black lifted to the darkest asphalt, 0.04
    // (57: lit and tonemapped, a grey of 0.04 is far from black).
    assert!(clean[0] > 180 && clean[0] == clean[1], "clean {clean:?}");
    assert!(
        damaged[0] < 80 && damaged[0] == damaged[1],
        "damaged {damaged:?}"
    );
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
    let renderer = ZoneRenderer::new(
        &gpu,
        &scene,
        Target::COLOR_FORMAT,
        |_| unreachable!(),
        Vec::new(),
    )
    .unwrap();
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent, &LIGHT))
        .unwrap();
    assert!(rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));
}

/// The 2 m square, 5 m ahead, as ground in the sRGB-encoded `colour`, its vertices
/// all weighted for one detail layer, drawn with `detail`; the middle row of pixels
/// and what was reported missing.
fn ground_row(
    colour: [u8; 4],
    detail: Vec<scene::Detail>,
    loaded: Vec<frostbite::Result<Loaded>>,
) -> (Vec<[u8; 4]>, Vec<(String, String)>) {
    let gpu = Gpu::new_headless().unwrap();
    let (_, square) = square_zone(&[]);
    let ground = frostbite::terrain::Ground {
        positions: square.positions.iter().map(|p| [p[0], p[1], 5.0]).collect(),
        colours: vec![colour; 4],
        indices: square.indices.clone(),
        detail: vec![1],
        weights: vec![[255, 0, 0, 0, 0, 0, 0, 0]; 4],
        ..Default::default()
    };
    let mut scene = scene::Scene::default();
    scene::add_ground(&mut scene, &ground, detail);
    let renderer = ZoneRenderer::new(
        &gpu,
        &scene,
        Target::COLOR_FORMAT,
        |_| unreachable!(),
        loaded,
    )
    .unwrap();
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent, &LIGHT))
        .unwrap();
    let row = (0..W).map(|x| pixel(&rgba, x, H / 2)).collect();
    (row, renderer.missing.clone())
}

#[test]
fn the_ground_is_drawn_in_its_vertex_colours() {
    let (row, missing) = ground_row([255, 0, 0, 255], Vec::new(), Vec::new());
    let m = row[W as usize / 2];
    assert!(m[0] > 100 && m[1] < 20 && m[2] < 20, "middle {m:?}");
    assert!(missing.is_empty());
}

#[test]
fn a_ground_layer_adds_its_diffuse_as_detail() {
    // 8 × 4 texels: white on the left half, black on the right, so its average is
    // half; at 0.5 a metre the square's 2 m cross one of each.
    let white = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
    let black = [0; 8];
    let texture = Texture {
        kind: Kind::Flat,
        format: 0x37,
        flags: 0,
        width: 8,
        height: 4,
        layers: 1,
        mip_sizes: vec![16, 8, 8, 8],
        streamed: 0,
        chunk: [0; 16],
        group: "Props".into(),
    };
    let data = [white, black, white, white, white].concat();
    let detail = || {
        vec![scene::Detail {
            texture: "stripes".into(),
            scale: 0.5,
        }]
    };
    let dark = [120, 0, 0, 255];
    let (plain, _) = ground_row(dark, Vec::new(), Vec::new());
    let loaded = vec![Ok(("stripes".to_string(), texture, data))];
    let (row, missing) = ground_row(dark, detail(), loaded);
    assert!(missing.is_empty(), "{missing:?}");
    // Across the square, brighter where the detail is white, darker where black.
    let change: Vec<i16> = plain
        .iter()
        .zip(&row)
        .filter(|(p, _)| p[0] > 0)
        .map(|(p, r)| i16::from(r[0]) - i16::from(p[0]))
        .collect();
    assert!(change.len() > 4, "the square is on screen");
    assert!(change.iter().any(|&c| c > 15), "brighter: {change:?}");
    assert!(change.iter().any(|&c| c < -15), "darker: {change:?}");

    // A layer whose texture can't be read is left out and listed.
    let gone = vec![Err(frostbite::Error("gone".into()))];
    let (row, missing) = ground_row(dark, detail(), gone);
    assert_eq!(missing, [("stripes".to_string(), "gone".to_string())]);
    assert_eq!(row, plain);
}

/// The middle pixel of the square at 5 m drawn as asphalt that gives way to a green
/// ground (its map; the ground's own square is hidden behind, at 6 m): a white clean
/// layer at full height, the shoulder's texture `shoulder` (its blue says where the
/// road shows) and the mask `mask` (its green times the height, under the dirt's
/// offset, lets the ground through).
fn asphalt_over_ground(shoulder: [u8; 8], mask: [u8; 8]) -> [u8; 4] {
    const WHITE: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
    let gpu = Gpu::new_headless().unwrap();
    let (zone, square) = square_zone(&[5.0]);
    let asphalt = Shading::Asphalt(Asphalt {
        tint: [[0.4; 3]; 2],
        tiling: [[1.0; 2]; 2],
        blend: [0.17, 0.88],
        tyre: [0.5, 0.88],
        tyre_tint: [0.5; 3],
        bias: 0.21,
        detail: [[0.5; 3]; 5],
        edge: Some(Edge {
            tiling: [1.0; 2],
            dirt: [0.216, 0.815, 0.0],
        }),
    });
    let section = Section {
        geometry: Geometry {
            uvs2: square.uvs.clone(),
            uvs3: square.uvs.clone(),
            // The road pieces' red is 0: a red of 1 keeps the road over the ground.
            colours: Some(vec![[0, 255, 255, 255]; 4]),
            ..square.clone()
        },
        texture: Some([1; 16]),
        layers: [2, 3, 4, 5, 6, 7].map(|t| Some([t; 16])),
        shading: asphalt,
    };
    let mut scene = scene::build(&zone, |_| Ok(vec![section.clone()]));
    let ground = frostbite::terrain::Ground {
        positions: square.positions.iter().map(|p| [p[0], p[1], 6.0]).collect(),
        colours: vec![[0, 255, 0, 255]; 4],
        indices: square.indices.clone(),
        map: frostbite::terrain::GroundMap {
            min: [-10.0; 2],
            texel: 20.0,
            width: 1,
            height: 1,
            colours: vec![[0, 255, 0, 255]],
            weights: vec![[0; 8]],
        },
        ..Default::default()
    };
    scene::add_ground(&mut scene, &ground, Vec::new());
    let load = move |guid: &[u8; 16]| {
        Ok(bc1(
            0x36,
            match guid[0] {
                1 | 3 | 6 => WHITE,
                4 => mask,
                7 => shoulder,
                _ => [0; 8],
            },
        ))
    };
    let renderer = ZoneRenderer::new(&gpu, &scene, Target::COLOR_FORMAT, load, Vec::new()).unwrap();
    assert_eq!(renderer.missing, []);
    let mut target = Target::new(&gpu, W, H).unwrap();
    let extent = target.color.extent;
    let camera = Camera::new([0.0; 3], 0.0, 0.0, W, H);
    let rgba = target
        .render(|cmd| renderer.record(cmd, &camera, extent, &LIGHT))
        .unwrap();
    pixel(&rgba, W / 2, H / 2)
}

#[test]
fn asphalt_gives_way_to_the_ground_where_its_shoulder_and_mask_say() {
    const WHITE: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0];
    const BLACK: [u8; 8] = [0; 8];
    // The shoulder's blue and the mask's green white: the road, grey.
    let road = asphalt_over_ground(WHITE, WHITE);
    assert!(road[0] > 180 && road[0] == road[1], "road {road:?}");
    // The shoulder black: the ground's green.
    let off = asphalt_over_ground(BLACK, WHITE);
    assert!(off[1] > 150 && off[0] < 60, "off the shoulder {off:?}");
    // The mask's green black (dirt): the ground's green too.
    let dirt = asphalt_over_ground(WHITE, BLACK);
    assert!(dirt[1] > 150 && dirt[0] < 60, "dirt {dirt:?}");
}
