# Baseline

The numbers the viewer has to beat, measured against the original Need for Speed: Heat
on the same camera path and the same PC (BRIEF.md, "Success criteria"). Both halves are
measured on `bench/paths/garage-drive.path` ("The same path").

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

## The same path, 2026-11-13 (nfsh 0.33.0)

`bench/baseline.sh bench/paths/garage-drive.path 494`: the original's recorded drive (25 s,
1,505 frames headless) through zone 494, which `nfsh level D:\nfsh-store X,Z` gives as
the one zone whose placements span the path's start, middle and end (x 3883..4576,
z 2222..2937, 1,805 placements). Midday, the zone with its roads and ground, 1280x720.
Times in ms.

| Path | Mode | Mean | Median | p99 | 1% low | Max | Load, warm (s) | Load, first (s) |
|------|------|------|--------|-----|--------|-----|----------------|-----------------|
| garage-drive | window | 6.97 (143 FPS) | 6.76 | 14.43 | 16.20 (62 FPS) | 21.06 | 42.08 (42.08, 41.26, 42.55) | 47.84 |
| garage-drive | headless, 0.32.0 | 17.68 (57 FPS) | 17.01 | 31.80 | 33.69 (30 FPS) | 40.61 | 49.71 (49.71, 52.10, 42.87) | 52.96 |
| garage-drive | headless, one run | 7.19 (139 FPS) | 6.86 | 11.92 | 14.05 (71 FPS) | 16.78 | | 41.42 |
| garage-drive | headless, 0.34.0, idle | 6.83 (146 FPS) | 6.57 | 10.35 | 11.96 (84 FPS) | 13.70 | 8.02 (8.35, 7.76, 8.02) | 7.79 |
| garage-drive | headless, 0.35.0, idle | 6.41 (156 FPS) | 6.28 | 10.98 | 12.22 (82 FPS) | 14.07 | 8.30 (8.46, 8.10, 8.30) | 37.29 |
| garage-drive | Heat, Low, window | | 114.0 (8.8 FPS) | p95 145.1 | | 188.0 | 88.8 (Play Solo to the garage) | |

- **The window run: 16.9 times the original's frame rate by the median** (6.76 against
  114.0 ms), and a 1% low of 16.2 ms, under Heat's best frame (92.8 ms at the garage). Not
  the same things drawn: nfsh draws one zone with its roads and ground, no car, traffic,
  sky dome, neighbouring zones, particles or post-processing; Heat at Low draws all of
  them.
- **Headless is as fast as the window; 0.32.0's 17.0 ms doesn't come back.** Three
  headless runs each of 0.34.0 and 0.35.0 on an idle PC, one after the other on
  2026-11-13, took 6.57 and 6.28 ms by the median (rows below), like the one run after
  the window runs (6.86 ms). The renderer is the same as 0.32.0's, so its 17.0 ms was the
  PC that day, not nfsh: something else was using the GPU, or the iGPU ran at lower
  clocks. Not known which.
- **Each window run loaded cold, about 42 s**, though they ran one after another. A
  later window run, after two headless ones, loaded in 9.9 s: the window isn't what
  empties the file cache. What does isn't known (the PC has 7.9 GB, and WSL's own memory
  grows and shrinks).

### Where zone 494's load goes

The viewer prints each stage's time and, since 0.34.0, what it read from the store
(`nfsh view` on stderr). Runs on an idle PC, headless unless marked; the stages after
the upload (lighting, the GPU, the pipelines and the first frame) are what is left of
the load.

| Run | Read (meshes, ground) | Upload (textures) | The rest | Load |
|-----|-----------------------|-------------------|----------|------|
| cold | 20.0 s | 16.6 s | 4.8 s | 41.4 s |
| warm | 3.2 s | 8.3 s | 1.1 s | 12.6 s |
| warm | 3.2 s | 6.8 s | 2.1 s | 12.1 s |
| window, cold | 14.4 s | 20.5 s | 2.7 s | 37.6 s |

| Zone | Read stage | Upload stage |
|------|------------|--------------|
| 494 | 37.0 MiB in 9,889 reads (3.8 KiB a read) | 608.9 MiB in 1,800 reads (346 KiB a read) |
| 495 | 43.9 MiB in 9,558 reads (4.7 KiB a read) | 509.0 MiB in 1,185 reads (440 KiB a read) |

- **Zone 494 isn't slow to load; a cold cache is.** Warm, it loads in about 12 s,
  near zone 495's 8 to 11 s; 0.32.0's "about 50 s" were cold loads. The store holds
  0.63 GiB for this zone, of 23 GiB.
- **Cold, the read stage costs about 17 s for 37 MiB.** The textures add 8 to 14 s for
  609 MiB, about 45 to 75 MB/s.
- The three runs that followed, while the owner was using the PC, took 46 to 62 s and
  aren't in the table; their read counts were the same.

### Seeks, not reads: the read stage in the packs' order (0.35.0)

A cold file cache can't be had on demand (emptying Windows' needs an administrator), so
the cold half is measured with `nfsh replay`: nfsh writes each store read to the file
`NFSH_READ_TRACE` names, and `nfsh replay STORE TRACE` reads the same bytes again with
the cache bypassed (`FILE_FLAG_NO_BUFFERING`, whole 4 KiB sectors), as many times as
wanted. As traced, zone 494's read stage replays in 21.1 to 23.9 s, close to its cold
17 to 21 s in the viewer.

**D: is a hard disk.** Read in 1 MiB pieces one after another it gives 93 MiB/s; 500
random 4 KiB reads anywhere in the store take 9.3 ms each, and within 50 MB of each
other 7.0 ms. An SSD, even over USB, answers a random read in well under 1 ms. So what a
cold load pays for is the distance between reads, and their order, more than their
number.

| The read stage's reads (zone 494, unbuffered) | Reads | MiB read | Time |
|---|---|---|---|
| as nfsh 0.34.0 reads them | 9,889 | 75.7 | 21.1-23.9 s |
| the same, sorted, merged when they touch | 6,001 | 59.1 | 7.5 s |
| merged across gaps up to 4 KiB | 2,000 | 46.9 | 6.5 s |
| merged across gaps up to 64 KiB | 1,067 | 62.8 | 6.4 s |
| merged across gaps up to 1 MiB | 604 | 193.6 | 6.2-6.5 s |
| in 0.34.0's order, through a cache of 64 MiB in 64 KiB pieces | 1,600 | 126.9 | 12.9 s |
| as nfsh 0.35.0 reads them | 7,549 | 78.6 | 12.4-14.7 s |

(The MiB column counts whole 4 KiB sectors, as a read past the cache must.)

- **The hypothesis holds, with the seek in place of the read:** sorted, the same bytes
  read in under a third of the time. Past a 4 KiB gap, merging more saves little: the
  ~6 s left are seeks from one pack to the next (the stage reads from 18 of the 23
  packs) and the bytes themselves. Caching nearby bytes without sorting saves less than
  half, because the reads hop between packs.
- **0.35.0 reads each zone's meshes in three rounds**, each in the packs' order with
  neighbours merged across 64 KiB: the meshes' partitions, then their mesh sets and lod
  groups, then every LOD's chunk. The meshes were 3,169 of the stage's reads and 13.4 s
  of its cold time; now 829 reads and 5.0 s. Each mesh's partition was also read three
  times; now once.

| Part of the read stage, cold (unbuffered) | Reads | As read | Sorted, 64 KiB gaps |
|---|---|---|---|
| the zone's own placements | 254 | 1.3 s | |
| the roads near it (every drivable-surface zone walked) | 6,000 | 3.4 s | 105 reads, 0.8 s |
| meshes, 0.34.0 | 3,169 | 13.4 s (in the viewer) | |
| meshes, 0.35.0 (three rounds) | 829 | 5.0 s | |
| ground layers | 284 | 1.6 s | 45 reads, 0.2 s |
| ground tiles | 182 | 1.5 s | 169 reads, 1.1 s |
| textures (the upload stage) | 1,802 | 17.3 s | 552 reads, 9.2 s |

- **In the viewer, cold:** the read stage took 20.4 and 21.3 s in two cold loads of
  0.34.0 and 13.8 s in 0.35.0's, which drew the same 2,103 placements in 3,432 draws.
  The whole load was 46.3 and 47.8 s, then 45.3 and 37.3 s: the texture stage, not yet
  sorted, varied from 21 to 27 s.
- **Warm, it costs nothing measurable:** 8.0 s (0.34.0) and 8.3 s (0.35.0) by the median
  of three loads each, the runs within 7.8 to 8.5 s. The rounds parse each partition and
  mesh set twice; that is at most the 0.3 s between them.
- **Left:** the textures (done in 0.36.0, below), the roads and the zone's own walk (4.7
  s, 0.8 s or so sorted: the walk follows blueprints, so its rounds mean walking again
  each time one is found), the layers and the ground tiles. On a disc, where a seek costs
  more, laying each zone's entries out together is the next step after that (org rules,
  "On a disc").

### The textures in the packs' order, read while they upload (0.36.0)

The texture stage (`nfsh view`'s upload, after the read stage) read each texture's ebx,
res and chunk as the renderer asked for it: 1,800 reads of 609 MiB, hopping between
packs. 0.36.0 finds every texture the zone needs first (the ground's detail and far
maps, then the scene's), reads their ebx and then their res in two sorted rounds, and
uploads the scene's textures in the order of their chunks in the packs. A second thread
reads the chunks in that order, 64 MiB of stored bytes at a time, each batch sorted and
merged across 64 KiB, while the first decodes and uploads; each chunk is dropped once
used.

| Zone 494's texture stage | Reads | Replayed unbuffered | In the viewer, cold | Warm |
|---|---|---|---|---|
| as 0.35.0 reads them | 1,800 | 17.1-19.5 s | 18.9, 19.5, 20.6 s | 6.3-6.4 s |
| sorted, read in 128 MiB batches before each is used (not released) | 570 | 9.4-10.3 s | 19.9 s | 7.6 s (one load, PC in use) |
| 0.36.0: sorted, read on a second thread | 570 | 9.4-11.8 s | 11.0, 11.0 s | 6.3-6.4 s |

- **Sorting alone saved nothing in the viewer.** Replayed past the cache, the sorted
  reads take half the time, but in a real cold load the stage stayed at 20 s: each
  batch waited for the disk, then the disk waited for the decoding and upload (about
  6.3 s of the stage, as warm loads show), and Windows' own read-ahead had already
  hidden part of the old order's seeks. Read on a second thread, the reads and the
  work overlap, and the stage takes about as long as its reads.
- **The whole cold load:** 35.4 to 39.4 s to the first frame in 0.35.0's three cold
  loads, 28.4 and 28.7 s in 0.36.0's; the read stage took 13.8 to 14.9 s in all of
  them. Warm, three loads each on an idle PC, alternating: 10.9 to 11.0 s for both
  (one more 0.35.0 load took 15.5 s, its read stage slow).
- **Memory:** the peak working set of a `--shot` load went from 1,927 to 1,989 MiB: one
  batch read while one waits and one is used.
- **The same picture:** the textures are uploaded in the packs' order but numbered in
  the scene's, so the draws keep their order; zone 494's `--shot` is byte for byte
  0.35.0's.
- Cold loads were had by reading 12 GiB of other files from D: first (the old 0.4.0
  store) to push the store out of Windows' file cache; each load's read stage (13.8 to
  14.9 s against about 3.6 s warm) shows it was cold.

### The walk in rounds, the ground on threads (0.37.0)

After 0.36.0 the read stage was most of what was left of a cold load: 14 s, against
about 3.6 s warm. Its parts, cold and replayed unbuffered, were the walk of the zone
and of every drivable-surface zone for its roads (about 6,250 reads, 4.7 s), the meshes
(829 reads, 5.0 s), the ground layers (284 reads, 1.6 s) and the ground tiles (182
reads, 1.5 s).

- **The walk reads ahead in rounds.** Before walking, nfsh reads the root partitions of
  the zone and of all 205 drivable-surface zones, then, round after round, every
  partition those link to through the fields the walk follows (an object list's
  objects, a blueprint's object, a reference's blueprint, a placed mesh), each round in
  the packs' order with neighbours merged. A mesh's partition is read but not followed.
  Every instance of a followed type counts, reached by the walk or not, so this reads
  what the walk reads and a little more; the walk then reads from memory. Zone 494 takes
  five rounds (67, 6,184, 35, 38 and 5 partitions): 6,329 partitions in 233 reads, 2.3 s
  replayed.
- **The ground's reads are sorted too:** the shader programs the layers decode are all
  known before the first is decoded, so they are read first (the part's 284 reads are
  now 49, 0.26 s replayed), and the tiles' chunks depend only on the terrain tree and
  the zone's extent (169 reads, 1.1 s).
- **The ground is worked out on threads.** Its layers don't depend on the zone and are
  mostly decoding (1.2 s, a fifth of it reading), so a thread works them out from the
  start; another reads and builds the ground tiles once the zone's extent is known and
  its meshes are read, while the meshes are built from memory. The ground is ready
  within 0.4 s of the meshes, or before them.

| Zone 494's read stage | Reads | Replayed unbuffered | In the viewer, PC in use |
|---|---|---|---|
| as 0.36.0 reads it | 7,549 | 12.2-14.0 s | 19.5, 14.6, 15.2 s |
| the walk and ground sorted, all on one thread (not released) | 1,280 | 8.4 s | 11.5 s |
| 0.37.0: sorted, the ground on threads | 1,280 | 8.0-8.2 s | 8.8, 9.7, 8.1, 9.2 s |

- **What is left is the meshes' LOD chunks:** 694 of the stage's 1,280 reads, 26 MiB
  scattered across the packs, about 5 s cold; sorting has already been done for them,
  and only laying a zone's entries out together in the packs (a repack, the disc's
  step) would shorten it. Building the meshes takes about 1.3 to 2.2 s after them;
  reading the chunks on a thread while the meshes are built, as 0.36.0 does for the
  textures, would save at most that.
- **The viewer column is not a cold cache on demand:** the PC was in use (another
  session's work and the owner's), with 1 to 2.5 GB free, and the store didn't stay in
  Windows' file cache between loads, so every load was close to cold and loads of the
  two releases alternated. The whole load to the first frame went from 32.9 to 34.0 s
  to 23.8 to 27.0 s in the alternating pairs.
- **The same picture:** zone 494's `--shot` is byte for byte 0.36.0's.
- The read stage prints its parts (`read stage: walk … s, meshes read … s, meshes built
  … s, waiting for the ground … s; on threads, ground layers … s, ground tiles … s`).

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
| Lower average and 1%-low frame times on the same path | on garage-drive.path, both in a window: Heat 114.0 ms median at Low, nfsh 6.76 ms median and 16.2 ms 1% low (one zone, no car, traffic or sky dome) |
| The same look by automated screenshot comparison | not started: Heat's frames can now be taken at a known camera; nfsh can't yet draw where they were taken |
| Lower load time to the first frame of Palm City | Heat 88.8 s (Play Solo to the garage); nfsh on zone 494 (the garage's): 42.1 s window with a cold file cache, 12.1 to 12.6 s warm (headless; one zone only) |
| nfsh's copy of the viewer's data is smaller | nfsh's install, base plus English with every tier, is 18.89 GiB ([SIZES.md](SIZES.md)); the viewer's own share is not split out yet |
