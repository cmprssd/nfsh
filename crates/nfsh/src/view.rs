//! `nfsh view`: one streaming zone of Palm City with the roads and ground under it,
//! textured and lit, in a window to fly
//! through: mouse to look, W A S D to move, Space and C up and down, Shift to go
//! faster, N to switch between midday and tealnight, Esc to quit. The title shows
//! the time of day, the frame rate and frame time. With `--shot`, one frame is drawn
//! without a window and written as a PNG. With `--path`, the camera flies a recorded
//! path (crate::path) instead, and the frame times along it and the time to the first
//! frame are printed at its end; `--headless` draws that path without a window.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use frostbite::layers;
use frostbite::level::Level;
use frostbite::level::xz_bounds;
use frostbite::lighting::{self, Lighting};
use frostbite::mesh::Meshes;
use frostbite::store::Store;
use frostbite::terrain::{self, Chunks, Ground, Tree};
use frostbite::texture::{Asset, Textures};
use nfsh_render::scene::{self, Scene};
use nfsh_render::zone::{DetailTextures, Loaded};
use nfsh_render::{Camera, FrameLoop, Gpu, Light, Presented, Swapchain, Target, ZoneRenderer, vk};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::fly::{self, Keys};
use crate::level::{SUPERBUNDLE, zone_bundle};
use crate::path::{Path as CameraPath, Stats};

/// The window's first size, and the screenshot's.
const SIZE: (u32, u32) = (1280, 720);
/// How far beyond the zone's own placements its drivable surfaces are taken from,
/// in metres: a road piece's origin can sit this far from the art over it.
const ROAD_MARGIN: f32 = 150.0;
/// The ground is drawn under the zone's placements and this far beyond them, in
/// metres, with a vertex every `GROUND_SPACING` metres (its tiles have one a metre).
const GROUND_MARGIN: f32 = ROAD_MARGIN;
const GROUND_SPACING: f32 = 2.0;
/// How often the title's frame rate is updated.
const TITLE_EVERY: Duration = Duration::from_millis(500);
/// The path time between two frames drawn with `--headless`, in seconds: every run
/// draws the same frames.
const HEADLESS_STEP: f64 = 1.0 / 60.0;

pub struct Options {
    pub store: PathBuf,
    pub zone: String,
    pub shot: Option<PathBuf>,
    pub vsync: bool,
    /// Start at tealnight rather than midday.
    pub night: bool,
    /// Where the camera starts, and its yaw and pitch in degrees, rather than the
    /// overview.
    pub camera: Option<[f32; 5]>,
    /// A camera path to fly instead of the keys, and whether to draw it without a
    /// window.
    pub path: Option<PathBuf>,
    pub headless: bool,
}

/// Parses `X,Y,Z,YAW,PITCH`.
fn camera(s: &str) -> Option<[f32; 5]> {
    let v: Vec<f32> = s
        .split(',')
        .map(|n| n.trim().parse().ok())
        .collect::<Option<_>>()?;
    let v: [f32; 5] = v.try_into().ok()?;
    v.iter().all(|n| n.is_finite()).then_some(v)
}

/// Parses `STORE ZONE [--shot PNG] [--vsync] [--night] [--camera X,Y,Z,YAW,PITCH]
/// [--path FILE [--headless]]`: a path is not a screenshot, and only a path is drawn
/// headless.
pub fn options(args: &[String]) -> Option<Options> {
    let (mut positional, mut shot, mut vsync, mut night) = (Vec::new(), None, false, false);
    let (mut start, mut path, mut headless) = (None, None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--vsync" => vsync = true,
            "--night" => night = true,
            "--camera" => start = Some(camera(it.next()?)?),
            "--shot" => shot = Some(PathBuf::from(it.next()?)),
            "--path" => path = Some(PathBuf::from(it.next()?)),
            "--headless" => headless = true,
            _ if a.starts_with("--") => return None,
            _ => positional.push(a),
        }
    }
    let [store, zone] = positional[..] else {
        return None;
    };
    if (path.is_some() && shot.is_some()) || (headless && path.is_none()) {
        return None;
    }
    Some(Options {
        store: store.into(),
        zone: zone.clone(),
        shot,
        vsync,
        night,
        camera: start,
        path,
        headless,
    })
}

/// The camera `o` asks for, else the overview of `scene`.
fn start_camera(o: &Options, scene: &Scene, (width, height): (u32, u32)) -> Camera {
    match o.camera {
        Some([x, y, z, yaw, pitch]) => Camera::new([x, y, z], yaw, pitch, width, height),
        None => scene::overview(scene, width, height),
    }
}

/// The times of day the viewer switches between: their names and light.
struct Times {
    names: [&'static str; 2],
    lights: [Light; 2],
}

impl Times {
    /// Midday and tealnight from `store`; a time whose lighting can't be read is lit
    /// by [`Lighting::FALLBACK`], and says so.
    fn read(store: &Store) -> Self {
        let read = |name: &str, partition: &str| {
            let l = Lighting::read(store, partition).unwrap_or_else(|e| {
                eprintln!("  {name}: {e}; lit by a default sun instead");
                Lighting::FALLBACK
            });
            Light::new(&l)
        };
        Self {
            names: ["midday", "tealnight"],
            lights: [
                read("midday", lighting::MIDDAY),
                read("tealnight", lighting::TEALNIGHT),
            ],
        }
    }
}

pub fn run(o: &Options) -> Result<String, String> {
    let started = Instant::now();
    let path = match &o.path {
        Some(file) => {
            let text =
                std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            Some(CameraPath::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?)
        }
        None => None,
    };
    let name = zone_bundle(&o.zone);
    let store = Store::open(&o.store).map_err(|e| e.to_string())?;
    let scene = load(&store, &name).map_err(|e| e.to_string())?;
    let times = Times::read(&store);
    match (&o.shot, &path) {
        (Some(png), _) => shot(o, &store, &scene, &times, png),
        (None, Some(path)) if o.headless => headless(o, &store, &scene, &times, path, started),
        _ => window(o, &name, &store, &scene, &times, path.as_ref(), started),
    }
}

/// The line that says how long the viewer took from its start to its first frame.
fn load_line(started: Instant, first_frame: Instant) -> String {
    format!(
        "load: {:.2} s to the first frame\n",
        (first_frame - started).as_secs_f64()
    )
}

/// Draws `path` without a window, a frame every [`HEADLESS_STEP`] of path time, and
/// returns the time to the first frame and the frame times. A frame's time is its
/// recording, drawing and read back, one at a time: an upper bound on a window's,
/// where the CPU records a frame while the GPU draws the one before.
fn headless(
    o: &Options,
    store: &Store,
    scene: &Scene,
    times: &Times,
    path: &CameraPath,
    started: Instant,
) -> Result<String, String> {
    let gpu = Gpu::new_headless().map_err(|e| e.to_string())?;
    let renderer = renderer(&gpu, store, scene, Target::COLOR_FORMAT)?;
    let mut target = Target::new(&gpu, SIZE.0, SIZE.1).map_err(|e| e.to_string())?;
    let extent = target.color.extent;
    let light = &times.lights[usize::from(o.night)];
    let frames = (path.duration() / HEADLESS_STEP).floor() as usize + 1;
    let (mut first, mut ms) = (None, Vec::with_capacity(frames));
    for i in 0..frames {
        let [x, y, z, yaw, pitch] = path.at(i as f64 * HEADLESS_STEP);
        let camera = Camera::new([x, y, z], yaw, pitch, SIZE.0, SIZE.1);
        let begin = Instant::now();
        target
            .render(|cmd| renderer.record(cmd, &camera, extent, light))
            .map_err(|e| e.to_string())?;
        let end = Instant::now();
        // The first frame is part of the load (pipelines warm up); it is not timed.
        match first {
            None => first = Some(end),
            Some(_) => ms.push((end - begin).as_secs_f64() * 1000.0),
        }
    }
    let stats = Stats::of(&ms).ok_or("the path is too short to time")?;
    Ok(format!(
        "{}{} (headless, {}, {})\n",
        load_line(started, first.unwrap_or(started)),
        stats.line(),
        gpu.name,
        times.names[usize::from(o.night)]
    ))
}

/// Uploads `scene` with its textures from `store`, and prints what was uploaded and
/// what is drawn grey.
fn renderer(
    gpu: &std::sync::Arc<Gpu>,
    store: &Store,
    scene: &Scene,
    format: vk::Format,
) -> Result<ZoneRenderer, String> {
    let start = Instant::now();
    let disk = frostbite::cas::read_so_far();
    // The chunks are read on another thread while the textures are decoded and
    // uploaded here; the scope ends once both are done.
    let renderer = std::thread::scope(|scope| upload(gpu, store, scene, format, scope))?;
    let normals = scene.detail.iter().filter(|d| d.normal.is_some()).count();
    let ground = scene.detail.len() + normals + scene::far_maps(&scene.detail).len();
    let wanted = scene.textures.len() + ground;
    eprintln!(
        "{} textures ({} of them ground detail), {:.1} MiB uploaded in {:.1} s; {}",
        wanted.saturating_sub(renderer.missing.len()),
        ground,
        renderer.texture_bytes as f64 / f64::from(1 << 20),
        start.elapsed().as_secs_f64(),
        read_since(disk)
    );
    for (texture, why) in &renderer.missing {
        eprintln!("  drawn grey: {texture}: {why}");
    }
    Ok(renderer)
}

/// [`renderer`]'s textures, read ahead on a thread of `scope`, and the upload.
fn upload<'scope>(
    gpu: &std::sync::Arc<Gpu>,
    store: &'scope Store,
    scene: &Scene,
    format: vk::Format,
    scope: &'scope std::thread::Scope<'scope, '_>,
) -> Result<ZoneRenderer, String> {
    let mut textures = Textures::new(store).map_err(|e| e.to_string())?;
    let far_maps = scene::far_maps(&scene.detail);
    let order = plan_textures(&mut textures, scope, scene, &far_maps);
    let mut by_name = |name: &str| -> frostbite::Result<Loaded> {
        let texture = textures.by_name(name)?;
        let (texture, pixels) = textures.pixels(&texture)?;
        Ok((name.to_owned(), texture, pixels))
    };
    let detail = scene
        .detail
        .iter()
        .map(|d| DetailTextures {
            diffuse: by_name(&d.texture),
            normal: d.normal.as_deref().map(&mut by_name),
        })
        .collect();
    let far: Vec<_> = far_maps.iter().map(|name| by_name(name)).collect();
    let load = |guid: &[u8; 16]| -> frostbite::Result<Loaded> {
        let (name, texture) = textures.by_partition(guid)?;
        let (texture, pixels) = textures
            .pixels(&texture)
            .map_err(|e| frostbite::Error(format!("{name}: {e}")))?;
        Ok((name, texture, pixels))
    };
    ZoneRenderer::new(gpu, scene, format, load, &order, detail, far).map_err(|e| e.to_string())
}

/// Finds every texture `renderer` loads (the ground's detail and far maps, then the
/// scene's) and has `textures` read their chunks on a thread of `scope` in that order,
/// the scene's in the packs' order: the order to upload the scene's textures in, by
/// index.
fn plan_textures<'scope>(
    textures: &mut Textures<'scope>,
    scope: &'scope std::thread::Scope<'scope, '_>,
    scene: &Scene,
    far_maps: &[String],
) -> Vec<usize> {
    let mut assets = Vec::new();
    for d in &scene.detail {
        assets.push(Asset::Name(&d.texture));
        assets.extend(d.normal.as_deref().map(Asset::Name));
    }
    assets.extend(far_maps.iter().map(|n| Asset::Name(n)));
    let ground = assets.len();
    assets.extend(scene.textures.iter().map(Asset::Partition));
    let chunks = textures.locate(&assets);
    let (ground, own) = chunks.split_at(ground);
    let order = pack_order(own);
    let queue = ground.iter().chain(order.iter().map(|&i| &own[i]));
    textures.read_ahead(scope, queue.flatten().copied());
    order
}

/// The indices of `chunks` in the packs' order, those not found last.
fn pack_order(chunks: &[Option<frostbite::Location>]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..chunks.len()).collect();
    order.sort_by_key(|&i| (chunks[i].is_none(), chunks[i]));
    order
}

/// What has been read from the store since `then`, a [`frostbite::cas::read_so_far`].
fn read_since(then: (u64, u64)) -> String {
    frostbite::cas::flush_trace();
    let now = frostbite::cas::read_so_far();
    disk_line(now.0 - then.0, now.1 - then.1)
}

fn disk_line(bytes: u64, reads: u64) -> String {
    format!(
        "{:.1} MiB read from the store in {reads} reads",
        bytes as f64 / f64::from(1 << 20)
    )
}

/// The zone's placements and meshes, ready to upload; prints what it read and what
/// it could not.
///
/// The disk and the processor take turns: the ground's layers, mostly decoding and
/// independent of the zone, are worked out on a thread from the start, and the ground
/// tiles on another once the zone's extent is known and its meshes are read, while
/// the meshes are built from memory.
fn load(store: &Store, name: &str) -> frostbite::Result<Scene> {
    let start = Instant::now();
    let disk = frostbite::cas::read_so_far();
    std::thread::scope(|s| {
        let layers = s.spawn(|| timed(|| layers::textures(store)));
        let mut parts = Parts::new();
        let mut level = Level::new(store, SUPERBUNDLE)?;
        // The zone and every drivable-surface zone, whose roads near it are added.
        let mut walked = vec![name.to_owned()];
        walked.extend(level.drivable_zones());
        let partitions_read = level.prefetch(&walked);
        let mut zone = level.zone(name)?;
        let roads = level.add_drivable_surfaces(&mut zone, ROAD_MARGIN)?;
        drop(level);
        parts.done(&format!("walk ({partitions_read} partitions)"));
        let mut meshes = Meshes::new(store)?;
        let partitions: Vec<[u8; 16]> = zone.meshes.iter().map(|m| m.partition).collect();
        meshes.prefetch(&partitions);
        parts.done("meshes read");
        let bounds = xz_bounds(&zone);
        let ground = s.spawn(move || {
            // Without the layers' detail the ground is still drawn; without the ground,
            // the zone.
            let (textures, layers_took) =
                joined(layers).unwrap_or_else(|e| (Err(frostbite::Error(e)), Duration::ZERO));
            let detail = match textures {
                Ok(textures) => scene::ground_detail(&textures),
                Err(e) => {
                    eprintln!("  ground drawn without detail: {e}");
                    Default::default()
                }
            };
            let keys: Vec<u16> = detail.keys().copied().collect();
            let (ground, tiles_took) =
                timed(|| bounds.map(|(lo, hi)| ground(store, lo, hi, &keys)));
            (detail, ground, layers_took, tiles_took)
        });
        let noise = scene::texture_partition(store, scene::ASPHALT_NOISE);
        let mut scene = scene::build(&zone, |m| scene::lods(&mut meshes, m, noise));
        drop(meshes);
        parts.done("meshes built");
        let (detail, ground, layers_took, tiles_took) = joined(ground).unwrap_or_else(|e| {
            (
                Default::default(),
                Some(Err(frostbite::Error(e))),
                Duration::ZERO,
                Duration::ZERO,
            )
        });
        parts.done("waiting for the ground");
        match ground {
            Some(Ok(g)) => {
                eprintln!(
                    "  ground: {} tiles, {} triangles, detail layers {:?}",
                    g.tiles,
                    g.indices.len() / 3,
                    g.detail
                );
                for (tile, why) in &g.failed {
                    eprintln!("  ground tile {tile}: {why}");
                }
                let slots = g
                    .detail
                    .iter()
                    .filter_map(|l| detail.get(l).cloned())
                    .collect();
                scene::add_ground(&mut scene, &g, slots);
            }
            Some(Err(e)) => eprintln!("  ground not drawn: {e}"),
            None => {}
        }
        eprintln!(
            "{name}: {} placements ({roads} of them drivable surfaces) of {} meshes; {} vertices, {} triangles, {} draws; read in {:.1} s; {}",
            zone.placements.len(),
            zone.meshes.len(),
            scene.positions.len(),
            scene.triangles(),
            scene.draws.len(),
            start.elapsed().as_secs_f64(),
            read_since(disk)
        );
        eprintln!(
            "  read stage: {}; on threads, ground layers {:.1} s, ground tiles {:.1} s",
            parts.done.join(", "),
            layers_took.as_secs_f64(),
            tiles_took.as_secs_f64()
        );
        for (mesh, why) in &scene.failed {
            eprintln!("  not drawn: {mesh}: {why}");
        }
        if !scene.failed.is_empty() {
            eprintln!(
                "  {} meshes not drawn, {} placements left out",
                scene.failed.len(),
                scene.placements_left_out
            );
        }
        Ok(scene)
    })
}

/// The ground under `lo`..`hi` and [`GROUND_MARGIN`] around it, weighted by the
/// detail `layers`.
fn ground(store: &Store, lo: [f32; 2], hi: [f32; 2], layers: &[u16]) -> frostbite::Result<Ground> {
    let tree = Tree::read(store)?;
    let mut chunks = Chunks::new(store)?;
    let lo = lo.map(|v| v - GROUND_MARGIN);
    let hi = hi.map(|v| v + GROUND_MARGIN);
    chunks.prefetch(&terrain::ground_chunks(&tree, lo, hi));
    Ok(terrain::ground(
        &tree,
        lo,
        hi,
        GROUND_SPACING,
        layers,
        |id| chunks.read(id),
    ))
}

/// What `f` returns, and how long it took.
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    (f(), start.elapsed())
}

/// A scoped thread's result; an error if it panicked, so a bug there costs what it
/// computes, not the load.
fn joined<T>(thread: std::thread::ScopedJoinHandle<T>) -> Result<T, String> {
    thread
        .join()
        .map_err(|_| "a loading thread panicked".to_owned())
}

/// How long each part of the read stage's own thread took, in order.
struct Parts {
    done: Vec<String>,
    since: Instant,
}

impl Parts {
    fn new() -> Self {
        Self {
            done: Vec::new(),
            since: Instant::now(),
        }
    }

    /// Ends the part `what`, which began where the last one ended.
    fn done(&mut self, what: &str) {
        let now = Instant::now();
        let took = (now - self.since).as_secs_f64();
        self.done.push(format!("{what} {took:.1} s"));
        self.since = now;
    }
}

/// Draws the overview of the zone once, without a window, and writes it as a PNG.
fn shot(
    o: &Options,
    store: &Store,
    scene: &Scene,
    times: &Times,
    png: &Path,
) -> Result<String, String> {
    let night = o.night;
    let gpu = Gpu::new_headless().map_err(|e| e.to_string())?;
    let renderer = renderer(&gpu, store, scene, Target::COLOR_FORMAT)?;
    let mut target = Target::new(&gpu, SIZE.0, SIZE.1).map_err(|e| e.to_string())?;
    let camera = start_camera(o, scene, SIZE);
    let extent = target.color.extent;
    let light = &times.lights[usize::from(night)];
    let pixels = target
        .render(|cmd| renderer.record(cmd, &camera, extent, light))
        .map_err(|e| e.to_string())?;
    write_png(png, SIZE, &pixels)?;
    Ok(format!(
        "{} written ({}x{}, {}, {}, camera at {:.1?} yaw {:.1} pitch {:.1})\n",
        png.display(),
        SIZE.0,
        SIZE.1,
        gpu.name,
        times.names[usize::from(night)],
        camera.origin,
        camera.yaw,
        camera.pitch
    ))
}

fn write_png(path: &Path, (width, height): (u32, u32), rgba: &[u8]) -> Result<(), String> {
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::create(path).map_err(|e| error(&e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| error(&e))?;
    writer.write_image_data(rgba).map_err(|e| error(&e))?;
    writer.finish().map_err(|e| error(&e))
}

fn window(
    o: &Options,
    name: &str,
    store: &Store,
    scene: &Scene,
    times: &Times,
    path: Option<&CameraPath>,
    started: Instant,
) -> Result<String, String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = App {
        title: format!("nfsh: {name}"),
        options: o,
        store,
        scene,
        times,
        night: o.night,
        view: None,
        error: None,
        frames: 0,
        total: Duration::ZERO,
        path,
        first_frame: None,
        path_ms: Vec::new(),
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if let Some(e) = app.error {
        return Err(e);
    }
    if path.is_some() {
        let first = app
            .first_frame
            .ok_or("the window closed before a frame was drawn")?;
        let stats = Stats::of(&app.path_ms).ok_or("the window closed before the path was timed")?;
        let present = if o.vsync { "vsync" } else { "no vsync" };
        let time = times.names[usize::from(o.night)];
        return Ok(format!(
            "{}{} (window, {present}, {time})\n",
            load_line(started, first.0),
            stats.line(),
        ));
    }
    let average = app.total.as_secs_f64() * 1000.0 / app.frames.max(1) as f64;
    Ok(format!(
        "{} frames, {average:.2} ms on average\n",
        app.frames
    ))
}

struct App<'a> {
    title: String,
    options: &'a Options,
    store: &'a Store,
    scene: &'a Scene,
    times: &'a Times,
    /// Tealnight rather than midday.
    night: bool,
    view: Option<View>,
    error: Option<String>,
    /// Frames presented and the time between them, for the summary at the end.
    frames: u64,
    total: Duration,
    /// The path the camera flies, when there is one; when its first frame was
    /// presented and when it began; and the time of every frame after it, in ms.
    path: Option<&'a CameraPath>,
    first_frame: Option<(Instant, Instant)>,
    path_ms: Vec<f64>,
}

/// The open window and what draws into it. Fields drop in order: the GPU's users
/// before the window they present to.
struct View {
    renderer: ZoneRenderer,
    frames: FrameLoop,
    swapchain: Swapchain,
    window: Window,
    camera: Camera,
    keys: Keys,
    last_frame: Instant,
    /// Frame times and GPU times since the title was last updated.
    shown: (Instant, Vec<f64>, Vec<f32>),
}

impl App<'_> {
    fn open(&self, event_loop: &ActiveEventLoop) -> Result<View, String> {
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title(&self.title)
                    .with_inner_size(PhysicalSize::new(SIZE.0, SIZE.1)),
            )
            .map_err(|e| e.to_string())?;
        let (gpu, surface) = Gpu::new_for_window(
            window.display_handle().map_err(|e| e.to_string())?.as_raw(),
            window.window_handle().map_err(|e| e.to_string())?.as_raw(),
        )
        .map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let swapchain =
            Swapchain::new(&gpu, surface, (size.width, size.height), self.options.vsync)
                .map_err(|e| e.to_string())?;
        let renderer = renderer(&gpu, self.store, self.scene, swapchain.format)?;
        eprintln!(
            "{}: {:?}, {:?}, {}x{}",
            gpu.name,
            swapchain.format,
            swapchain.present_mode,
            swapchain.extent.width,
            swapchain.extent.height
        );
        let extent = swapchain.extent;
        let now = Instant::now();
        Ok(View {
            renderer,
            frames: FrameLoop::new(&gpu).map_err(|e| e.to_string())?,
            swapchain,
            camera: start_camera(self.options, self.scene, (extent.width, extent.height)),
            window,
            keys: Keys::default(),
            last_frame: now,
            shown: (now, Vec::new(), Vec::new()),
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, e: impl ToString) {
        self.error = Some(e.to_string());
        event_loop.exit();
    }
}

impl View {
    /// Moves the camera, draws and presents a frame lit by `light`, and updates the
    /// title. Returns the time since the previous frame.
    /// With `path` (the path and when its first frame began), the camera is where the
    /// path is at this time instead of where the keys take it.
    fn frame(
        &mut self,
        title: &str,
        light: &Light,
        path: Option<(&CameraPath, Instant)>,
    ) -> nfsh_render::Result<Duration> {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        match path {
            Some((path, began)) => {
                let [x, y, z, yaw, pitch] = path.at((now - began).as_secs_f64());
                (self.camera.origin, self.camera.yaw, self.camera.pitch) = ([x, y, z], yaw, pitch);
            }
            // A stall (the window dragged, a breakpoint) must not fling the camera away.
            None => fly::fly(&mut self.camera, self.keys, dt.as_secs_f32().min(0.1)),
        }
        let extent = self.swapchain.extent;
        if extent.width == 0 || extent.height == 0 {
            return Ok(dt);
        }
        (self.camera.width, self.camera.height) = (extent.width, extent.height);
        let (renderer, camera) = (&self.renderer, &self.camera);
        match self.frames.present(&self.swapchain, |cmd, extent| {
            renderer.record(cmd, camera, extent, light)
        })? {
            Presented::Drawn { gpu_ms } => {
                self.shown.1.push(dt.as_secs_f64() * 1000.0);
                self.shown.2.extend(gpu_ms);
            }
            Presented::Resize => self.resize()?,
        }
        if now - self.shown.0 >= TITLE_EVERY && !self.shown.1.is_empty() {
            self.window
                .set_title(&frame_title(title, &self.shown.1, &self.shown.2));
            self.shown = (now, Vec::new(), Vec::new());
        }
        Ok(dt)
    }

    fn resize(&mut self) -> nfsh_render::Result<()> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            // Minimised: nothing to draw until it comes back.
            self.swapchain.extent = Default::default();
            return Ok(());
        }
        self.frames.finish();
        self.swapchain.resize((size.width, size.height))
    }
}

/// The title with the frame rate, the mean frame time and the mean GPU time (when
/// the device has timestamps).
fn frame_title(title: &str, frame_ms: &[f64], gpu_ms: &[f32]) -> String {
    let mean = frame_ms.iter().sum::<f64>() / frame_ms.len().max(1) as f64;
    let gpu = match gpu_ms.len() {
        0 => String::new(),
        n => format!(
            " (GPU {:.2} ms)",
            gpu_ms.iter().map(|&g| f64::from(g)).sum::<f64>() / n as f64
        ),
    };
    format!(
        "{title} | {:.0} FPS, {mean:.2} ms{gpu}",
        1000.0 / mean.max(1e-3)
    )
}

fn grab_mouse(window: &Window) {
    // Locked keeps the cursor still (not on Windows); Confined keeps it in the window.
    let _ = window
        .set_cursor_grab(CursorGrabMode::Locked)
        .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
    window.set_cursor_visible(false);
}

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.view.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(view) => {
                grab_mouse(&view.window);
                self.view = Some(view);
            }
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(view) = &mut self.view else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => {
                if let Err(e) = view.resize() {
                    self.fail(event_loop, e);
                }
            }
            WindowEvent::Focused(true) => grab_mouse(&view.window),
            WindowEvent::Focused(false) => {
                view.keys = Keys::default();
                view.window.set_cursor_visible(true);
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        repeat: false,
                        ..
                    },
                ..
            } => {
                let down = state == ElementState::Pressed;
                let k = &mut view.keys;
                match code {
                    KeyCode::Escape if down => event_loop.exit(),
                    KeyCode::KeyN if down => self.night = !self.night,
                    KeyCode::KeyW | KeyCode::ArrowUp => k.forward = down,
                    KeyCode::KeyS | KeyCode::ArrowDown => k.back = down,
                    KeyCode::KeyA | KeyCode::ArrowLeft => k.left = down,
                    KeyCode::KeyD | KeyCode::ArrowRight => k.right = down,
                    KeyCode::Space => k.up = down,
                    KeyCode::KeyC | KeyCode::ControlLeft => k.down = down,
                    KeyCode::ShiftLeft | KeyCode::ShiftRight => k.fast = down,
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let time = usize::from(self.night);
                let title = format!("{} | {}", self.title, self.times.names[time]);
                let now = Instant::now();
                let path = self
                    .path
                    .map(|p| (p, self.first_frame.map_or(now, |f| f.1)));
                let dt = match view.frame(&title, &self.times.lights[time], path) {
                    Ok(dt) => dt,
                    Err(e) => return self.fail(event_loop, e),
                };
                self.frames += 1;
                self.total += dt;
                if let Some(path) = self.path {
                    // The first frame ends the load and starts the path; each frame
                    // after it is timed until the path ends.
                    match self.first_frame {
                        None => self.first_frame = Some((Instant::now(), now)),
                        Some((_, began)) => {
                            self.path_ms.push(dt.as_secs_f64() * 1000.0);
                            if (now - began).as_secs_f64() >= path.duration() {
                                event_loop.exit();
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let (Some(view), DeviceEvent::MouseMotion { delta }) = (&mut self.view, event)
            && view.window.has_focus()
        {
            fly::look(&mut view.camera, delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(view) = &self.view {
            event_loop.set_control_flow(ControlFlow::Poll);
            view.window.request_redraw();
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(view) = &self.view {
            view.frames.finish();
        }
        self.view = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_upload_in_the_packs_order_those_not_found_last() {
        let at = |cas, offset| {
            Some(frostbite::Location {
                cas: frostbite::CasRef {
                    patch: false,
                    package: 0,
                    cas,
                },
                offset,
                size: 1,
            })
        };
        let chunks = [at(1, 0), None, at(0, 9), at(0, 2)];
        assert_eq!(pack_order(&chunks), [3, 2, 0, 1]);
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn options_take_a_store_a_zone_and_flags() {
        let o = options(&args("D:/store 495 --shot z.png --vsync --night")).unwrap();
        assert_eq!(
            (o.store, o.zone.as_str()),
            (PathBuf::from("D:/store"), "495")
        );
        assert_eq!(
            (o.shot, o.vsync, o.night),
            (Some(PathBuf::from("z.png")), true, true)
        );
        let o = options(&args("store levels/mainlevel/worldart_zs_12")).unwrap();
        assert_eq!(
            (o.shot, o.vsync, o.night, o.camera),
            (None, false, false, None)
        );
        let o = options(&args("store 495 --camera 1,2.5,-3,90,-10")).unwrap();
        assert_eq!(o.camera, Some([1.0, 2.5, -3.0, 90.0, -10.0]));
        assert!(options(&args("store 495 --camera 1,2,3,4")).is_none());
        assert!(options(&args("store 495 --camera 1,2,3,4,x")).is_none());
        assert!(options(&args("store 495 --camera 1,2,3,4,NaN")).is_none());
        let o = options(&args("store 495 --path p.path --headless --night")).unwrap();
        assert_eq!((o.path, o.headless), (Some(PathBuf::from("p.path")), true));
        let o = options(&args("store 495 --path p.path")).unwrap();
        assert_eq!((o.path, o.headless), (Some(PathBuf::from("p.path")), false));
        // A path is not a screenshot, and only a path is drawn headless.
        assert!(options(&args("store 495 --path p.path --shot z.png")).is_none());
        assert!(options(&args("store 495 --headless")).is_none());
        assert!(options(&args("store 495 --path")).is_none());
        // A missing value, an unknown flag, or the wrong number of arguments.
        assert!(options(&args("store 495 --shot")).is_none());
        assert!(options(&args("store 495 --fast")).is_none());
        assert!(options(&args("store")).is_none());
        assert!(options(&args("store 495 extra")).is_none());
    }

    #[test]
    fn the_disk_line_gives_mebibytes_and_reads() {
        assert_eq!(
            disk_line(3 << 19, 7),
            "1.5 MiB read from the store in 7 reads"
        );
    }

    #[test]
    fn the_title_shows_rate_and_times() {
        assert_eq!(
            frame_title("nfsh: z", &[4.0, 6.0], &[1.0, 2.0]),
            "nfsh: z | 200 FPS, 5.00 ms (GPU 1.50 ms)"
        );
        assert_eq!(frame_title("t", &[10.0], &[]), "t | 100 FPS, 10.00 ms");
    }
}
