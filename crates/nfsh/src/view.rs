//! `nfsh view`: one streaming zone of Palm City, textured and lit, in a window to fly
//! through: mouse to look, W A S D to move, Space and C up and down, Shift to go
//! faster, N to switch between midday and tealnight, Esc to quit. The title shows
//! the time of day, the frame rate and frame time. With `--shot`, one frame is drawn
//! without a window and written as a PNG.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use frostbite::level::Level;
use frostbite::lighting::{self, Lighting};
use frostbite::mesh::Meshes;
use frostbite::store::Store;
use frostbite::texture::Textures;
use nfsh_render::scene::{self, Scene};
use nfsh_render::zone::Loaded;
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

/// The window's first size, and the screenshot's.
const SIZE: (u32, u32) = (1280, 720);
/// How far beyond the zone's own placements its drivable surfaces are taken from,
/// in metres: a road piece's origin can sit this far from the art over it.
const ROAD_MARGIN: f32 = 150.0;
/// How often the title's frame rate is updated.
const TITLE_EVERY: Duration = Duration::from_millis(500);

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

/// Parses `STORE ZONE [--shot PNG] [--vsync] [--night] [--camera X,Y,Z,YAW,PITCH]`.
pub fn options(args: &[String]) -> Option<Options> {
    let (mut positional, mut shot, mut vsync, mut night) = (Vec::new(), None, false, false);
    let mut start = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--vsync" => vsync = true,
            "--night" => night = true,
            "--camera" => start = Some(camera(it.next()?)?),
            "--shot" => shot = Some(PathBuf::from(it.next()?)),
            _ if a.starts_with("--") => return None,
            _ => positional.push(a),
        }
    }
    let [store, zone] = positional[..] else {
        return None;
    };
    Some(Options {
        store: store.into(),
        zone: zone.clone(),
        shot,
        vsync,
        night,
        camera: start,
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
    let name = zone_bundle(&o.zone);
    let store = Store::open(&o.store).map_err(|e| e.to_string())?;
    let scene = load(&store, &name).map_err(|e| e.to_string())?;
    let times = Times::read(&store);
    match &o.shot {
        Some(png) => shot(o, &store, &scene, &times, png),
        None => window(o, &name, &store, &scene, &times),
    }
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
    let mut textures = Textures::new(store).map_err(|e| e.to_string())?;
    let load = |guid: &[u8; 16]| -> frostbite::Result<Loaded> {
        let (name, texture) = textures.by_partition(guid)?;
        let pixels = textures
            .pixels(&texture)
            .map_err(|e| frostbite::Error(format!("{name}: {e}")))?;
        Ok((name, texture, pixels))
    };
    let renderer = ZoneRenderer::new(gpu, scene, format, load).map_err(|e| e.to_string())?;
    eprintln!(
        "{} textures, {:.1} MiB uploaded in {:.1} s",
        scene.textures.len() - renderer.missing.len(),
        renderer.texture_bytes as f64 / f64::from(1 << 20),
        start.elapsed().as_secs_f64()
    );
    for (texture, why) in &renderer.missing {
        eprintln!("  drawn grey: {texture}: {why}");
    }
    Ok(renderer)
}

/// The zone's placements and meshes, ready to upload; prints what it read and what
/// it could not.
fn load(store: &Store, name: &str) -> frostbite::Result<Scene> {
    let start = Instant::now();
    let mut level = Level::new(store, SUPERBUNDLE)?;
    let mut zone = level.zone(name)?;
    let roads = level.add_drivable_surfaces(&mut zone, ROAD_MARGIN)?;
    let mut meshes = Meshes::new(store)?;
    let scene = scene::build(&zone, |m| scene::first_lod(&mut meshes, m));
    eprintln!(
        "{name}: {} placements ({roads} of them drivable surfaces) of {} meshes; {} vertices, {} triangles, {} draws; read in {:.1} s",
        zone.placements.len(),
        zone.meshes.len(),
        scene.positions.len(),
        scene.triangles(),
        scene.draws.len(),
        start.elapsed().as_secs_f64()
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
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if let Some(e) = app.error {
        return Err(e);
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
    fn frame(&mut self, title: &str, light: &Light) -> nfsh_render::Result<Duration> {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        // A stall (the window dragged, a breakpoint) must not fling the camera away.
        fly::fly(&mut self.camera, self.keys, dt.as_secs_f32().min(0.1));
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
                match view.frame(&title, &self.times.lights[time]) {
                    Ok(dt) => {
                        self.frames += 1;
                        self.total += dt;
                    }
                    Err(e) => self.fail(event_loop, e),
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
        // A missing value, an unknown flag, or the wrong number of arguments.
        assert!(options(&args("store 495 --shot")).is_none());
        assert!(options(&args("store 495 --fast")).is_none());
        assert!(options(&args("store")).is_none());
        assert!(options(&args("store 495 extra")).is_none());
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
