# Baseline

The numbers the viewer has to beat, measured against the original Need for Speed: Heat
on the same camera path and the same PC (BRIEF.md, "Success criteria"). nfsh's half is
measured; the original's is not yet (see "The original's half").

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

## The original's half

Not measured yet. The original has never run on this PC: the Steam version needs the
EA app and an EA sign-in, and a first start opens on the story's prologue rather than
free roam. Heat has no free camera or console, so the plan (the owner's choice,
2026-11-11) is:

- **The camera path is recorded from the original.** A small tool reads the camera's
  position and angles from the running game's memory while the owner drives or uses
  photo mode, and writes it in the path format above. It reads and never writes, and
  the repo holds only our description of where the camera sits. nfsh then plays that
  recorded path.
- **Frame times** of the original by PresentMon (external, from the swapchain's
  presents), on the same path, windowed 1280x720 at the original's own settings, which
  will be recorded here.
- **Load time** to the first frame of Palm City from the game's start, by the same
  external clock.
- **Screenshots** of the original's window at chosen moments of the path, compared with
  nfsh's frames at the same cameras (as swjkja does). These will differ in what nfsh
  doesn't draw yet: traffic, the player's car, the sky's dome (the viewer clears to
  black above the horizon) and the other zones in view.

## Success criteria

| Criterion | State |
|-----------|-------|
| Lower average and 1%-low frame times on the same path | nfsh measured; the original not yet |
| The same look by automated screenshot comparison | not started: needs the original's frames |
| Lower load time to the first frame of Palm City | nfsh measured (zone 495 only); the original not yet |
| nfsh's copy of the viewer's data is smaller | nfsh's install, base plus English with every tier, is 18.89 GiB ([SIZES.md](SIZES.md)); the viewer's own share is not split out yet |
