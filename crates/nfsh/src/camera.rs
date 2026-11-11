//! The original game's camera, read from its memory to record a path for the baseline
//! (docs/BASELINE.md, "The original's half"). It only reads: the process is opened for
//! reading memory and querying it, nothing else, and nothing is written to it.
//!
//! Where the camera sits is found the way a memory scanner finds anything: every
//! 16-byte-aligned 4x4 matrix of floats whose 3x3 part is a rotation is a candidate,
//! and the owner's turns of the camera narrow them down. `scan` keeps the candidates
//! near a point; `changed` keeps those that moved since the last look (the owner
//! turned the camera); `same` keeps those that did not (the owner held it still).
//! Each step writes the candidates back to their file. The addresses hold only while
//! the game runs, so the file is for one session and never goes in the repo.
//!
//! A camera matrix can be stored four ways, its `Layout`; each candidate is one
//! address read one way. `record` then polls one candidate and writes a path
//! (`path.rs`), a key whenever the camera moves.

// The commands run on Windows only; elsewhere the logic is reached by its tests alone.
#![cfg_attr(not(windows), allow(dead_code))]

use std::fmt::Write as _;

/// A 4x4 matrix as it sits in memory: `m[row][column]`, rows 16 bytes apart.
pub type Matrix = [[f32; 4]; 4];

/// How a camera matrix is stored. Frostbite's world space is y up and left-handed,
/// its transforms rows of right, up, forward and position (`render::camera`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// The camera's transform: rows right, up, forward, then its position.
    Transform,
    /// The view matrix for row vectors (`p * V`): the transform's inverse.
    View,
    /// The transform transposed: columns right, up, forward, position.
    TransformColumns,
    /// The view matrix for column vectors (`V * p`).
    ViewColumns,
}

impl Layout {
    pub const ALL: [Layout; 4] = [
        Layout::Transform,
        Layout::View,
        Layout::TransformColumns,
        Layout::ViewColumns,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Layout::Transform => "transform",
            Layout::View => "view",
            Layout::TransformColumns => "transform-columns",
            Layout::ViewColumns => "view-columns",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == name)
    }
}

const TOLERANCE: f32 = 2e-3;

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn row(m: &Matrix, i: usize) -> [f32; 3] {
    [m[i][0], m[i][1], m[i][2]]
}

fn column(m: &Matrix, j: usize) -> [f32; 3] {
    [m[0][j], m[1][j], m[2][j]]
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= TOLERANCE
}

/// Whether the 3x3 part is a rotation (or a mirrored one): unit rows, at right angles.
/// Orthonormal rows make orthonormal columns too, so this holds for every layout.
fn is_rotation(m: &Matrix) -> bool {
    let r = [row(m, 0), row(m, 1), row(m, 2)];
    m.iter().flatten().all(|v| v.is_finite())
        && (0..3).all(|i| near(dot3(r[i], r[i]), 1.0))
        && near(dot3(r[0], r[1]), 0.0)
        && near(dot3(r[0], r[2]), 0.0)
        && near(dot3(r[1], r[2]), 0.0)
}

/// The camera's position and forward vector, if `m` read as `layout` is a camera
/// matrix at all.
pub fn pose(m: &Matrix, layout: Layout) -> Option<([f32; 3], [f32; 3])> {
    if !is_rotation(m) {
        return None;
    }
    let last_column_free = (0..3).all(|i| near(m[i][3], 0.0));
    let last_row_free = (0..3).all(|j| near(m[3][j], 0.0)) && near(m[3][3], 1.0);
    let pose = match layout {
        Layout::Transform if last_column_free => (row(m, 3), row(m, 2)),
        Layout::View if last_column_free => {
            let t = row(m, 3);
            ([0, 1, 2].map(|j| -dot3(t, row(m, j))), column(m, 2))
        }
        Layout::TransformColumns if last_row_free => (column(m, 3), column(m, 2)),
        Layout::ViewColumns if last_row_free => {
            let t = column(m, 3);
            ([0, 1, 2].map(|j| -dot3(t, column(m, j))), row(m, 2))
        }
        _ => return None,
    };
    Some(pose)
}

/// Yaw and pitch in degrees of a forward vector, as `render::Camera` takes them: yaw
/// from +z towards +x, pitch up.
pub fn angles(forward: [f32; 3]) -> (f32, f32) {
    let yaw = forward[0].atan2(forward[2]).to_degrees();
    let flat = forward[0].hypot(forward[2]);
    let pitch = forward[1].atan2(flat).to_degrees();
    (yaw.rem_euclid(360.0), pitch.clamp(-89.0, 89.0))
}

/// `yaw` moved by whole turns to lie within half a turn of `previous`, so a path
/// turning through north reads 350 to 370 and not 350 to 10 (`path.rs`).
pub fn unwrap_yaw(previous: f32, yaw: f32) -> f32 {
    yaw + ((previous - yaw) / 360.0).round() * 360.0
}

/// Reads a matrix from 64 little-endian bytes.
pub fn matrix(bytes: &[u8; 64]) -> Matrix {
    let mut m = [[0.0; 4]; 4];
    for (k, v) in m.iter_mut().flatten().enumerate() {
        *v = f32::from_le_bytes(bytes[k * 4..k * 4 + 4].try_into().unwrap());
    }
    m
}

/// One candidate: an address in the game's memory, read as `layout`, and the matrix
/// it held at the last look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub address: u64,
    pub layout: Layout,
    pub matrix: Matrix,
}

impl Candidate {
    pub fn pose(&self) -> Option<([f32; 3], [f32; 3])> {
        pose(&self.matrix, self.layout)
    }
}

/// Every candidate in `bytes` (memory starting at `base`, scanned at 16-byte steps)
/// whose camera lies within `radius` metres of `centre`.
pub fn find(base: u64, bytes: &[u8], centre: [f32; 3], radius: f32) -> Vec<Candidate> {
    let mut found = Vec::new();
    let skip = (16 - (base % 16) as usize) % 16;
    let mut at = skip;
    while at + 64 <= bytes.len() {
        let m = matrix(bytes[at..at + 64].try_into().unwrap());
        if is_rotation(&m) {
            for layout in Layout::ALL {
                if let Some((p, _)) = pose(&m, layout) {
                    let d = [0, 1, 2].map(|i| p[i] - centre[i]);
                    if dot3(d, d) <= radius * radius {
                        found.push(Candidate {
                            address: base + at as u64,
                            layout,
                            matrix: m,
                        });
                    }
                }
            }
        }
        at += 16;
    }
    found
}

/// Whether the camera turned by more than `degrees` or moved by more than `metres`
/// between two looks.
pub fn moved(before: &Matrix, after: &Matrix, layout: Layout, degrees: f32, metres: f32) -> bool {
    match (pose(before, layout), pose(after, layout)) {
        (Some((p0, f0)), Some((p1, f1))) => {
            let d = [0, 1, 2].map(|i| p1[i] - p0[i]);
            let cos = dot3(f0, f1).clamp(-1.0, 1.0);
            cos.acos().to_degrees() > degrees || dot3(d, d).sqrt() > metres
        }
        _ => false,
    }
}

/// Candidates as a file: per candidate the address (8 bytes), the layout (1) and the
/// matrix (64), little-endian.
pub fn encode(candidates: &[Candidate]) -> Vec<u8> {
    let mut out = Vec::with_capacity(candidates.len() * 73);
    for c in candidates {
        out.extend_from_slice(&c.address.to_le_bytes());
        out.push(c.layout.code());
        for v in c.matrix.iter().flatten() {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Candidate>, String> {
    if !bytes.len().is_multiple_of(73) {
        return Err("not a candidates file".into());
    }
    bytes
        .as_chunks::<73>()
        .0
        .iter()
        .map(|c| {
            Ok(Candidate {
                address: u64::from_le_bytes(c[..8].try_into().unwrap()),
                layout: Layout::from_code(c[8]).ok_or("not a candidates file")?,
                matrix: matrix(c[9..].try_into().unwrap()),
            })
        })
        .collect()
}

/// The candidates as text, up to `limit` of them, with where each camera is.
pub fn listing(candidates: &[Candidate], limit: usize) -> String {
    let mut out = format!("{} candidates\n", candidates.len());
    for c in candidates.iter().take(limit) {
        if let Some((p, f)) = c.pose() {
            let (yaw, pitch) = angles(f);
            let _ = writeln!(
                out,
                "{:#014x} {:<17} at {:9.2} {:8.2} {:9.2} yaw {:6.1} pitch {:5.1}",
                c.address,
                c.layout.name(),
                p[0],
                p[1],
                p[2],
                yaw,
                pitch
            );
        }
    }
    if candidates.len() > limit {
        let _ = writeln!(out, "... and {} more", candidates.len() - limit);
    }
    out
}

/// A path being recorded: a key whenever the camera moves, times from the first key.
pub struct Recording {
    text: String,
    first: Option<f64>,
    last: Option<(f64, [f32; 5])>,
    keys: usize,
}

impl Recording {
    pub fn new(header: &str) -> Self {
        let mut text = String::new();
        for line in header.lines() {
            let _ = writeln!(text, "# {line}");
        }
        Self {
            text,
            first: None,
            last: None,
            keys: 0,
        }
    }

    /// Adds the camera seen at `seconds` (any clock) if it moved since the last key.
    /// Returns whether it did.
    pub fn add(&mut self, seconds: f64, position: [f32; 3], forward: [f32; 3]) -> bool {
        let (mut yaw, pitch) = angles(forward);
        if let Some((_, k)) = self.last {
            yaw = unwrap_yaw(k[3], yaw);
        }
        let key = [position[0], position[1], position[2], yaw, pitch];
        if let Some((t, k)) = self.last {
            let still = (0..3).all(|i| (k[i] - key[i]).abs() < 0.005)
                && (k[3] - key[3]).abs() < 0.01
                && (k[4] - key[4]).abs() < 0.01;
            if still || seconds <= t {
                return false;
            }
        }
        let first = *self.first.get_or_insert(seconds);
        let _ = writeln!(
            self.text,
            "{:.4} {:.3} {:.3} {:.3} {:.3} {:.3}",
            seconds - first,
            key[0],
            key[1],
            key[2],
            key[3],
            key[4]
        );
        self.last = Some((seconds, key));
        self.keys += 1;
        true
    }

    pub fn keys(&self) -> usize {
        self.keys
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(windows)]
pub mod process {
    //! The game's process, opened to read its memory and nothing else.

    use super::Matrix;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, PAGE_READWRITE,
        PAGE_WRITECOPY, VirtualQueryEx,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    pub const EXE: &str = "NeedForSpeedHeat.exe";

    pub struct Process(HANDLE);

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: the handle came from OpenProcess and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn pid(exe: &str) -> Option<u32> {
        // SAFETY: plain Win32 calls; the entry's size is set before use and the
        // snapshot is closed.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut found = None;
            let mut ok = Process32FirstW(snap, &mut entry) != 0;
            while ok {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
                if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(exe) {
                    found = Some(entry.th32ProcessID);
                    break;
                }
                ok = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            found
        }
    }

    impl Process {
        pub fn open() -> Result<Self, String> {
            let pid = pid(EXE).ok_or_else(|| format!("{EXE} is not running"))?;
            // SAFETY: a plain Win32 call; read and query rights only.
            let handle =
                unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, 0, pid) };
            if handle.is_null() {
                return Err(format!(
                    "could not open {EXE} to read it: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self(handle))
        }

        /// Reads `buffer.len()` bytes at `address`; false if any of it is not readable.
        pub fn read(&self, address: u64, buffer: &mut [u8]) -> bool {
            let mut done = 0usize;
            // SAFETY: the buffer is ours and its length is passed.
            let ok = unsafe {
                ReadProcessMemory(
                    self.0,
                    address as *const _,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut done,
                )
            };
            ok != 0 && done == buffer.len()
        }

        pub fn matrix(&self, address: u64) -> Option<Matrix> {
            let mut bytes = [0u8; 64];
            self.read(address, &mut bytes)
                .then(|| super::matrix(&bytes))
        }

        /// The committed, writable regions (where a camera that changes must live), as
        /// (base, size).
        pub fn regions(&self) -> Vec<(u64, usize)> {
            let mut regions = Vec::new();
            let mut address = 0u64;
            loop {
                // SAFETY: the info struct is ours and its size is passed.
                let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
                let got = unsafe {
                    VirtualQueryEx(
                        self.0,
                        address as *const _,
                        &mut info,
                        size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                };
                if got == 0 {
                    break;
                }
                let base = info.BaseAddress as u64;
                let writable = info.Protect & (PAGE_READWRITE | PAGE_WRITECOPY) != 0
                    && info.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0;
                if info.State == MEM_COMMIT && writable {
                    regions.push((base, info.RegionSize));
                }
                let next = base.saturating_add(info.RegionSize as u64);
                if next <= address {
                    break;
                }
                address = next;
            }
            regions
        }
    }
}

/// `nfsh camera ...`: the commands, on Windows only.
pub fn run(args: &[String]) -> Result<String, String> {
    #[cfg(windows)]
    {
        commands::run(args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Err("nfsh camera reads the running game, on Windows only".into())
    }
}

#[cfg(windows)]
mod commands {
    use super::process::Process;
    use super::*;
    use std::time::{Duration, Instant};

    /// Turns smaller than this, or moves shorter than `METRES`, count as holding still.
    const DEGREES: f32 = 2.0;
    const METRES: f32 = 0.25;

    fn numbers<const N: usize>(text: &str) -> Result<[f32; N], String> {
        let v: Vec<f32> = text
            .split(',')
            .map(|n| n.trim().parse::<f32>().ok().filter(|n| n.is_finite()))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("not numbers: {text}"))?;
        v.try_into()
            .map_err(|_| format!("needs {N} numbers: {text}"))
    }

    fn load(file: &str) -> Result<Vec<Candidate>, String> {
        decode(&std::fs::read(file).map_err(|e| format!("{file}: {e}"))?)
    }

    fn save(file: &str, candidates: &[Candidate]) -> Result<(), String> {
        std::fs::write(file, encode(candidates)).map_err(|e| format!("{file}: {e}"))
    }

    fn scan(game: &Process, centre: [f32; 3], radius: f32) -> Vec<Candidate> {
        const CHUNK: usize = 4 << 20;
        let mut found = Vec::new();
        let mut buffer = vec![0u8; CHUNK + 64];
        for (base, size) in game.regions() {
            let mut at = 0usize;
            while at < size {
                // Chunks overlap by 64 bytes so no matrix is split between two.
                let len = (size - at).min(CHUNK + 64);
                if game.read(base + at as u64, &mut buffer[..len]) {
                    let mut got = find(base + at as u64, &buffer[..len], centre, radius);
                    got.retain(|c| c.address < base + (at + CHUNK) as u64);
                    found.extend(got);
                }
                at += CHUNK;
            }
        }
        found
    }

    /// Reads every candidate again and keeps those whose move matches `want_moved`,
    /// with what they hold now.
    fn filter(game: &Process, candidates: Vec<Candidate>, want_moved: bool) -> Vec<Candidate> {
        candidates
            .into_iter()
            .filter_map(|c| {
                let now = game.matrix(c.address)?;
                pose(&now, c.layout)?;
                let m = moved(&c.matrix, &now, c.layout, DEGREES, METRES);
                (m == want_moved).then_some(Candidate { matrix: now, ..c })
            })
            .collect()
    }

    pub fn run(args: &[String]) -> Result<String, String> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match args[..] {
            ["scan", file, "--near", point] | ["scan", file, "--near", point, "--radius", _] => {
                let radius = match args.get(5) {
                    Some(r) => r.parse().map_err(|_| format!("not a radius: {r}"))?,
                    None => 2000.0,
                };
                let game = Process::open()?;
                let found = scan(&game, numbers::<3>(point)?, radius);
                save(file, &found)?;
                Ok(listing(&found, 20))
            }
            [step @ ("changed" | "same"), file] => {
                let game = Process::open()?;
                let kept = filter(&game, load(file)?, step == "changed");
                save(file, &kept)?;
                Ok(listing(&kept, 20))
            }
            ["show", file] => {
                let game = Process::open()?;
                let now: Vec<Candidate> = load(file)?
                    .into_iter()
                    .filter_map(|c| {
                        Some(Candidate {
                            matrix: game.matrix(c.address)?,
                            ..c
                        })
                    })
                    .collect();
                Ok(listing(&now, 50))
            }
            ["record", address, layout, out, "--seconds", seconds] => {
                let address = u64::from_str_radix(address.trim_start_matches("0x"), 16)
                    .map_err(|_| format!("not a hex address: {address}"))?;
                let layout =
                    Layout::from_name(layout).ok_or_else(|| format!("not a layout: {layout}"))?;
                let seconds: f64 = seconds
                    .parse()
                    .map_err(|_| format!("not seconds: {seconds}"))?;
                let game = Process::open()?;
                let mut recording = Recording::new(&format!(
                    "Recorded from the original game's camera by nfsh camera record ({}),\n\
                     T X Y Z YAW PITCH, seconds from the first move.",
                    layout.name()
                ));
                let start = Instant::now();
                let mut first_move: Option<Instant> = None;
                while first_move.map_or(start.elapsed().as_secs_f64() < 600.0, |t| {
                    t.elapsed().as_secs_f64() < seconds
                }) {
                    let now = start.elapsed().as_secs_f64();
                    let (p, f) = game
                        .matrix(address)
                        .and_then(|m| pose(&m, layout))
                        .ok_or("the camera's address no longer holds a camera")?;
                    // The first key is where the camera rests; time starts when it moves.
                    if recording.add(now, p, f) && recording.keys() == 2 {
                        first_move = Some(Instant::now());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                std::fs::write(out, recording.text()).map_err(|e| format!("{out}: {e}"))?;
                Ok(format!("{} keys written to {out}\n", recording.keys()))
            }
            _ => Err(USAGE.into()),
        }
    }

    const USAGE: &str = "usage: nfsh camera scan FILE --near X,Y,Z [--radius M]
       nfsh camera changed FILE | same FILE | show FILE
       nfsh camera record ADDRESS LAYOUT PATH --seconds S";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera transform at `p` looking along yaw/pitch, as `render::Camera` builds it.
    fn transform(p: [f32; 3], yaw: f32, pitch: f32) -> Matrix {
        let c = nfsh_render::camera::Camera::new(p, yaw, pitch, 16, 9);
        let [f, r, u] = c.axes();
        [
            [r[0], r[1], r[2], 0.0],
            [u[0], u[1], u[2], 0.0],
            [f[0], f[1], f[2], 0.0],
            [p[0], p[1], p[2], 1.0],
        ]
    }

    fn transpose(m: &Matrix) -> Matrix {
        let mut t = [[0.0; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                t[i][j] = m[j][i];
            }
        }
        t
    }

    /// The inverse of a rigid transform in row-vector form.
    fn view(m: &Matrix) -> Matrix {
        let mut v = [[0.0; 4]; 4];
        for i in 0..3 {
            for j in 0..3 {
                v[i][j] = m[j][i];
            }
        }
        let t = row(m, 3);
        for (j, v) in v[3].iter_mut().take(3).enumerate() {
            *v = -dot3(t, row(m, j));
        }
        v[3][3] = 1.0;
        v
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn every_layout_gives_back_the_camera() {
        let p = [3700.0, 130.0, 2900.0];
        let t = transform(p, 30.0, -45.0);
        let stored = [
            (Layout::Transform, t),
            (Layout::View, view(&t)),
            (Layout::TransformColumns, transpose(&t)),
            (Layout::ViewColumns, transpose(&view(&t))),
        ];
        for (layout, m) in stored {
            let (pos, f) = pose(&m, layout).unwrap_or_else(|| panic!("{layout:?}"));
            assert!(close(pos, p), "{layout:?} {pos:?}");
            let (yaw, pitch) = angles(f);
            assert!(
                (yaw - 30.0).abs() < 0.01 && (pitch + 45.0).abs() < 0.01,
                "{layout:?}"
            );
        }
    }

    #[test]
    fn a_scaled_or_skewed_matrix_is_no_camera() {
        let mut m = transform([0.0; 3], 10.0, 5.0);
        m[0][0] *= 1.1;
        assert!(Layout::ALL.iter().all(|&l| pose(&m, l).is_none()));
        assert!(pose(&[[0.0; 4]; 4], Layout::Transform).is_none());
    }

    #[test]
    fn find_keeps_aligned_cameras_near_the_point() {
        let near = transform([10.0, 2.0, 0.0], 90.0, 0.0);
        let far = transform([9000.0, 2.0, 0.0], 90.0, 0.0);
        let mut bytes = vec![0xffu8; 16];
        for m in [near, far] {
            for v in m.iter().flatten() {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        // Memory starting 8 bytes before a 16-byte boundary: the first matrix sits
        // at 0x1008 + 16 = 0x1018, which is not aligned, so nothing is found...
        assert!(find(0x1008, &bytes, [0.0; 3], 100.0).is_empty());
        // ...and at an aligned base the near one is, read as a transform (and, being
        // near the origin, as a view too: only the owner's turns tell those apart).
        let found = find(0x1000, &bytes, [0.0; 3], 100.0);
        assert!(found.iter().all(|c| c.address == 0x1010));
        assert!(found.iter().any(|c| c.layout == Layout::Transform));
        assert_eq!(decode(&encode(&found)).unwrap(), found);
    }

    #[test]
    fn moved_tells_a_turn_from_stillness() {
        let a = transform([0.0; 3], 10.0, 0.0);
        let b = transform([0.0; 3], 25.0, 0.0);
        assert!(moved(&a, &b, Layout::Transform, 5.0, 0.5));
        assert!(!moved(&a, &a, Layout::Transform, 5.0, 0.5));
    }

    #[test]
    fn a_recording_keys_movement_only_and_unwraps_yaw() {
        let mut r = Recording::new("test");
        let f = |yaw: f32| {
            let (s, c) = yaw.to_radians().sin_cos();
            [s, 0.0, c]
        };
        assert!(r.add(5.0, [0.0; 3], f(350.0)));
        assert!(!r.add(5.1, [0.0; 3], f(350.0)));
        assert!(r.add(5.5, [1.0, 0.0, 0.0], f(10.0)));
        assert_eq!(r.keys(), 2);
        let path = crate::path::Path::parse(r.text()).unwrap();
        assert_eq!(path.duration(), 0.5);
        let end = path.at(0.5);
        assert!((end[3] - 370.0).abs() < 0.01, "{end:?}");
    }
}
