# Baseline

The numbers the viewer has to beat, measured against the original Need for Speed: Heat
on the same camera path and the same PC (BRIEF.md, "Success criteria"). Both halves are
measured on `bench/paths/garage-drive.path` (nfsh headless so far; "The same path").

## Machine

Intel Core i5-7400T (4 cores, 2.4 GHz), Intel HD Graphics 630 (driver 31.0.101.2141),
7.9 GB of memory, Windows. The store is on drive D: (USB).

## The path

`bench/paths/speedway.path`: 40 seconds through zone 495, the speedway, from high above
the stands down to the track, along the barrels, low over the asphalt and back up. It
passes the history views' cameras (`bench/history-views.txt`). A path is one key per
line, `T X Y Z YAW PITCH` (seconds, metres, degrees, as `nfsh view --camera` takes
them), and the camera moves in straight lines between keys.

## How nfsh is measured

`bench/baseline.sh` builds nfsh for Windows from the checkout and runs
`nfsh view D:\nfsh-store 495 --path bench/paths/speedway.path` four times: a warm-up,
then three runs whose medians are the result. Each run prints:

- **Load**: from the viewer's start to its first frame: reading the zone, its roads and
  ground, uploading their textures, building the pipelines and drawing a frame.
- **Frame times** of every frame after the first along the path: mean, median, p99,
  **1% low** (the mean of the slowest 1% of frames) and max.

Two modes:

- **window** (`MODE=window`): a 1280x720 window, vsync off, the time between frames,
  as a player sees it. This is the one to compare with the original. It opens a window,
  so it refuses while the PC is in use unless `NFSH_ALLOW_WINDOW=1`.
- **headless** (default): the same path without a window, a frame every 1/60 s of path
  time (2,400 frames, the same every run), each recorded, drawn and read back before
  the next starts. Being one frame at a time, it is an upper bound on the window's, and
  it needs no one's screen.

## Results, 2026-11-11 (nfsh 0.31.0)

Midday, zone 495 with the roads and ground under it, 1280x720. Times in ms.

| Path | Mode | Mean | Median | p99 | 1% low | Max | Load, warm (s) | Load, first (s) |
|------|------|------|--------|-----|--------|-----|----------------|-----------------|
| speedway | window | 11.63 (86 FPS) | 11.54 | 17.13 | 20.55 (49 FPS) | 26.23 | 24.19 (30.78, 24.19, 16.45) | 27.28 |
| speedway | headless | 11.81 (85 FPS) | 11.83 | 16.05 | 18.56 (54 FPS) | 26.37 | 10.36 (11.00, 10.36, 7.73) | 10.53 |

- The window and headless frame times agree: the GPU is the limit, about 11.5 ms a frame.
- **Load varies with the file cache.** Headless runs right after one another load in 8 to
  11 s; the first two headless runs of the day, with nothing cached, took 32 and 36 s.
  The window runs ranged from 16 to 31 s, and the store's files did not stay in the
  7.9 GB of memory between them. The original's load will be measured the same way, a
  warm-up and then three runs.
- The window's warm-up ran slower (14.81 ms mean) than the three runs after it.

## The same path, 2026-11-12 (nfsh 0.32.0)

`bench/baseline.sh bench/paths/garage-drive.path 494`: the original's recorded drive (25 s,
1,505 frames headless) through zone 494, which `nfsh level D:\nfsh-store X,Z` gives as
the one zone whose placements span the path's start, middle and end (x 3883..4576,
z 2222..2937, 1,805 placements). Midday, the zone with its roads and ground, 1280x720.
Times in ms.

| Path | Mode | Mean | Median | p99 | 1% low | Max | Load, warm (s) | Load, first (s) |
|------|------|------|--------|-----|--------|-----|----------------|-----------------|
| garage-drive | headless | 17.68 (57 FPS) | 17.01 | 31.80 | 33.69 (30 FPS) | 40.61 | 49.71 (49.71, 52.10, 42.87) | 52.96 |
| garage-drive | Heat, Low, window | | 114.0 (8.8 FPS) | p95 145.1 | | 188.0 | 88.8 (Play Solo to the garage) | |

- **6.7 times the original's frame rate by the median** (17.0 against 114.0 ms), but
  not the same things drawn: nfsh draws one zone with its roads and ground, no car,
  traffic, sky dome, neighbouring zones, particles or post-processing; Heat at Low
  draws all of them. The window run, the one comparable with Heat's, isn't done yet:
  it opens a window on the owner's screen.
- **Load is five times zone 495's** (about 50 s against 10 s headless). Zone 494 spans
  about 700 m by 700 m; which part of its load (reading, uploading, pipelines) costs
  most isn't measured yet.
- The third run's 1% low (47.20 ms) and max (77.14 ms) stand out from the other two;
  the medians use all three.

## The original's half

### Results, 2026-11-12 (Heat, Steam, through the EA app)

Solo, day, the **Low** graphics preset, windowed 1280x720, vsync off, motion blur on
(the game's default), resolution scale 100%. Frame times by PresentMon 2.6.0 from Heat's
swapchain presents. Times in ms.

| Where | Frames | Median | p95 | Max | GPU busy, median | CPU busy, median |
|-------|--------|--------|-----|-----|------------------|------------------|
| Standing at the garage by Palm City Raceway | 43 | 110.3 (9.1 FPS) | 128.0 | 135.5 | 92.8 | 109.6 |
| Driving out of the garage (1st drive) | 182 | 105.9 (9.4 FPS) | 137.0 | 179.2 | 93.8 | 104.7 |
| Backing out of a ditch and driving on | 225 | 96.0 (10.4 FPS) | 124.0 | 165.4 | 83.8 | 94.7 |
| Driving `bench/paths/garage-drive.path` | 261 | 114.0 (8.8 FPS) | 145.1 | 188.0 | 102.4 | 112.9 |

- **The GPU is the limit,** as for nfsh: it is busy about 90 ms of each ~105 ms frame on
  the HD 630, even at Low. nfsh draws the speedway at 11.5 ms, but that is not yet the
  same path or the same things drawn (below).
- **Load, Play Solo to the garage's menu:** 88.8 s (84.6, 88.8, 90.4), the three runs after
  a warm-up of about 127 s. Quitting to the lobby takes 38 to 47 s. Timed from the key
  press to the garage menu's first frame by watching one pixel of the menu (its yellow
  currency box) go away and come back, every 0.25 s; the garage is in Palm City, so
  this is the closest the game gives to "the first frame of Palm City".
- **Not the speedway path.** At 9 FPS, driven by scripted keys without seeing the
  screen, the car didn't reach zone 495, 600 m away. The path recorded is a 25 s drive
  through a field east of the raceway, `bench/paths/garage-drive.path`, in zone 494;
  nfsh's numbers on it are under "The same path".
- A first start shows a warning that the Intel driver (31.0.101.2141) is older than
  recommended; the game misreads the version, and it runs.

### How the original's camera is read

`nfsh camera` reads the game's memory and never writes it.

1. `scan FILE --near X,Y,Z --radius M` keeps every aligned 4x4 matrix whose 3x3 part is a
   rotation and whose camera lies within M metres. Near the car, with a 300 m radius:
   about 19,000.
2. With the car at rest for a while, `same`. Then drive a second and stop, wait about
   10 s for the chase camera to settle, and `changed`; wait again and `same`. Three of
   these rounds leave about 160 candidates. Most of them are copies of the one camera
   (the renderer keeps it per pass), and they agree to the centimetre. Waits shorter
   than the camera's settling drop the real camera: the first search lost it that way.
3. Check the camera against the car: the chase camera sits 5.4 m behind the car along
   its heading and 1.4 m above it, pitched 3.5 degrees down at it.
4. `record ADDRESS transform PATH --seconds S` polls one copy and writes a key whenever
   it moves. A copy can read as cleared for one poll, so a read over 100 m from the
   last key is skipped. The path starts at the last look before the first move.

What was found about the matrix:

- **The game's camera matrix looks down its -z.** Its rows are left, up, back and
  position: Frostbite's transform (right, up, forward, position) turned half a turn
  about up. Cycling the views (C: chase, near, hood, bumper) moved the camera along -z,
  and -z pitches the chase view down at the car.
- **Changing view isn't a test of the camera.** An address that changed with every view
  and agreed with each view's place held the view's preset, and didn't follow the car
  when it drove off. Driving is the test.
- The addresses hold only while the game runs; each session scans again (about 50 s
  for 2 km, a few seconds for 300 m).

## Success criteria

| Criterion | State |
|-----------|-------|
| Lower average and 1%-low frame times on the same path | on garage-drive.path: Heat 114.0 ms median at Low (window), nfsh 17.0 ms median, 33.7 ms 1% low (headless, one zone; the window run is still to do) |
| The same look by automated screenshot comparison | not started: Heat's frames can now be taken at a known camera; nfsh can't yet draw where they were taken |
| Lower load time to the first frame of Palm City | Heat 88.8 s (Play Solo to the garage); nfsh 24.2 s window, 10.4 s headless (zone 495 only) |
| nfsh's copy of the viewer's data is smaller | nfsh's install, base plus English with every tier, is 18.89 GiB ([SIZES.md](SIZES.md)); the viewer's own share is not split out yet |
