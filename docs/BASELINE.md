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
  empties the file cache. Most likely other programs' memory does: on 2026-11-14,
  loads stayed cold with Chrome open (2.5 GB free) and warm without it (5 GB free;
  see the idle-PC table under 0.37.0).

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

**On an idle PC (2026-11-14).** Zone 494, headless along garage-drive.path, 0.37.0
against 0.36.0, loads alternating, with Chrome closed and 4.9 to 5.1 GB free. Cold
loads followed reading 12 GiB of other packs from D: (the old 0.4.0 store, different
packs each time).

| Zone 494 | Read stage, warm | Load, warm | Read stage, cold | Load, cold |
|---|---|---|---|---|
| 0.36.0 | 2.5, 2.5, 2.6 s | 7.35, 7.45, 7.68 s | 14.1, 13.9 s | 27.36, 26.79 s |
| 0.37.0 | 1.7, 4.6, 1.7 s | 6.74, 9.66, 6.67 s | 8.7, 8.7 s | 21.62, 21.65 s |

- **Cold, 0.37.0 saves 5.3 s of the read stage and 5.4 s of the load**, as the
  replayed reads foretold (12.2-14.0 s to 8.0-8.2 s). The cold texture stage is 10.4
  to 10.5 s in both.
- **Warm, the read stage goes from 2.5 s to 1.7 s.** The build of the meshes (1.1 s)
  is now most of it. The 4.6 s load was partly cold: its walk took 3.3 s against
  0.4 s, its ground layers 3.1 s against 0.6 s. It came right after a cold load of
  0.36.0, whose reads differ.
- **The store stays in the file cache when the PC has memory to spare.** With Chrome
  open (2.5 GB free) every load of the evening was close to cold, and warm loads took
  20 to 25 s. With 5 GB free they took 6.7 to 7.7 s. The earlier "the store doesn't
  stay cached" was memory pressure from other programs.
- The warm load's texture stage (4.2 to 4.4 s) is now its largest part.

### The meshes' chunks read behind their decoding (tried, not released)

The idea was to read the LOD chunks (694 reads, about 5 s cold) on a second thread,
as 0.36.0 does for textures, while the meshes are decoded in their chunks' order, each
once its last chunk had come, and then put the scene together in the zone's order. The
ground tiles' chunks were read first on the main thread, so the disk read one thing at
a time. Zone 494's `--shot` was byte for byte 0.37.0's, so it worked, but it saved
nothing:

| Zone 494's read stage, alternating, PC in use | Reads | Read stage |
|---|---|---|
| 0.37.0 | 1,280 | 9.4, 8.6, 8.7 s |
| the chunks read behind (2 MiB batches) | 1,312 | 9.2, 9.4, 9.6 s |

- **The disk is the limit, and 0.37.0 already keeps it busy.** The build's 1.8 s
  hides behind 0.37.0's ground tiles thread, which reads its 169 reads (about 1.1 s
  cold) at the same time. Reading behind hid the decoding (about 1.2 s), but the ground
  tiles' reads moved onto the main line (1.0-1.2 s), and putting the scene together
  (0.6 s) can't start before the last model has come. The disk's own time (mesh sets
  0.5 s, ground tiles 1.1 s, chunks about 4.5 s) is the floor, and both versions sit
  within about 0.6 s of it.
- None of these loads was warm: the meshes' reads took 4.5-4.9 s each time, as
  cold, with 2.5 GB free and Chrome busy. Warm, there is nothing to hide the build
  behind. On the idle PC, 0.37.0's warm loads read the meshes in 0.1 s and build them
  in 1.1 s. Cold, nothing shortens this stage short of laying a zone's entries out
  together in the packs.

### The textures decoded on threads (0.38.0)

Warm, the texture stage (4.3 to 4.5 s of a 6.9 to 7.2 s load) was most of zone 494's
load. 0.38.0's viewer prints its parts (`texture stage: finding … s, waiting for chunks
… s, decoding here … s, uploading … s, the rest … s`). With 0.37.0's code and only the
timers added, warm, on an idle PC: finding the textures 0.2 s, waiting for reads 0.1 s,
**decoding 3.0 to 3.2 s**, uploading 0.7 to 0.8 s, the rest 0.3 s. Decoding was
three-quarters of the stage, all on the thread that uploads.

0.38.0 adds a stage between the thread that reads the chunks and the one that uploads
them: each 64 MiB batch, once read, is decoded on all but one of the processor's
threads (3 of 4 here), each taking the largest chunk left next, and handed over whole.
The upload order is unchanged.

Zone 494, headless along garage-drive.path, on an idle PC with Chrome closed and 4.2
GB free, loads alternating with 0.37.0 plus the timers (2026-11-15). Cold loads came
after reading 12 GiB of other packs from D: (the old 0.4.0 store), different packs each
time.

| Zone 494 | Texture stage, warm | Load, warm | Texture stage, cold | Load, cold |
|---|---|---|---|---|
| 0.37.0 (with the timers) | 4.3, 4.5, 4.5 s | 6.93, 7.17, 7.07 s | 10.6, 10.6 s | 22.16, 22.08 s |
| 0.38.0: decoded on 3 threads | 2.2, 2.3, 2.3 s | 4.79, 4.99, 4.87 s | 10.6, 10.5 s | 22.06, 21.96 s |

- **Warm, the texture stage halves (4.4 to 2.3 s), and the load goes from about 7.1 to
  4.9 s.** The upload thread now waits 0.4 to 0.5 s for decoded chunks and decodes
  nothing itself. Uploading grew from 0.8 to 1.1 to 1.3 s: filling the staging buffers
  now shares the processor and memory bandwidth with the decoding threads.
- **Cold, nothing changes:** the stage waits on the disk (8.5 s of 10.5), as it did
  waiting for reads (5.6 s) plus decoding (2.9 s). Decoding hid behind the reads; it
  added nothing to the cold load, and only a zone's entries laid out together in the
  packs would shorten it.
- **Memory:** the peak working set of a `--shot` load went from 1,992 to 2,050 MiB: a
  decoded batch waits while one is used.
- **The same picture:** zone 494's `--shot` is byte for byte 0.37.0's.
- **What is left warm:** the read stage (about 1.7 s, the meshes' build 1.1 s of it),
  the texture stage's 2.3 s (now mostly uploading, one staging buffer and one
  wait for the GPU per texture), and about 0.9 s of pipelines and the first frame.

### The textures uploaded in batches (0.39.0)

In 0.38.0 uploading was most of the warm texture stage (1.1 to 1.3 s of 2.3 s, for
600 textures and 867 MiB): each texture had its own staging buffer, newly allocated, and
the thread waited for the GPU after each one. 0.39.0 reuses one 64 MiB staging buffer in
two halves, each with its own command buffer and fence. Textures are copied into one half,
several to a submit, while the GPU copies the other half into their images, and the thread
waits only to reuse a half and once at the end. A texture larger than a half (32 MiB)
still goes up on its own. The images are numbered as before.

Zone 494, headless along garage-drive.path, loads alternating with 0.38.0 built from its
tag (2026-11-15, at night, the PC idle, Chrome closed, 3.4 GB free once WSL's file cache
of the builds was dropped). Cold loads came after reading 12 GiB of other packs from D:
(the old 0.4.0 store) through Windows, different packs each time.

| Zone 494 | Uploading, warm | Texture stage, warm | Load, warm | Texture stage, cold | Load, cold |
|---|---|---|---|---|---|
| 0.38.0 | 1.2, 1.2, 1.3 s | 2.3, 2.2, 2.2 s | 5.09, 4.79, 4.91 s | 10.6, 10.5 s | 22.57, 21.61 s |
| 0.39.0: in batches | 0.6, 0.6, 0.7 s | 2.1, 2.1, 2.2 s | 4.88, 4.43, 4.56 s | 10.6, 10.5 s | 21.70, 21.66 s |

- **Uploading halves (1.2 to 1.3 s down to 0.6 to 0.7 s), but the stage saves only 0.1 to
  0.2 s:** waiting for decoded chunks grows from 0.4 to 0.8 to 0.9 s. The upload thread
  now outruns the three threads that decode, so decoding is the warm texture stage's
  limit. An earlier set of three pairs gave the same split (uploading 1.3 s against
  0.6 s, waiting 0.4 s against 0.8 to 0.9 s), with loads within noise of each other
  (4.57 to 7.43 s against 4.59 to 4.91 s).
- **Cold, nothing changes:** the stage waits on the disk (8.4 to 8.9 s of 10.5).
  Uploading there goes from 0.9 to 0.4 to 0.5 s, hidden behind the reads.
- **Memory:** the peak working set of a `--shot` load goes from 2,051 to 2,016 MiB: the
  per-texture staging buffers are gone, and the ring takes 64 MiB.
- **The same picture:** zone 494's `--shot` is byte for byte 0.38.0's.
- **What is left warm:** decoding the textures (the upload thread could decode too
  while it waits), the read stage's mesh build (1.1 s) and about 0.9 s of pipelines and
  the first frame.

### The upload thread decoding too (tried, not released)

In 0.39.0 the upload thread waits 0.9 to 1.0 s of the warm texture stage for the three
threads that decode. Tried: one queue of frames per load instead of whole 64 MiB
batches. The threads decode the frames in upload order, the upload thread takes each
chunk as soon as it is decoded, decodes it itself when no thread has started it, and
decodes the next frame waiting while another thread finishes the one it needs. The
upload order, and so the picture, stays the same.

Zone 494, headless along garage-drive.path, warm, loads alternating with 0.39.0 built
from its tag (2026-11-15, morning, the PC idle, 3.4 GB free).

| Zone 494, warm | Waiting for chunks | Decoding here | Uploading | Texture stage | Load |
|---|---|---|---|---|---|
| 0.39.0 | 0.9, 1.0, 1.0 s | 0.0 s | 0.6, 0.5, 0.6 s | 2.1, 2.1, 2.1 s | 4.74, 4.61, 4.88 s |
| The upload thread decodes too | 0.1, 0.1, 0.2 s | 0.5, 0.5, 0.6 s (50, 7, 53 chunks) | 0.8, 0.8, 0.6 s | 2.1, 2.1, 2.0 s | 4.83, 4.63, 4.29 s |

- **The wait goes, the stage doesn't shorten:** the upload thread swaps 0.8 s of waiting
  for 0.5 to 0.6 s of decoding, and the stage stays at 2.0 to 2.1 s, the loads within
  noise of 0.39.0's. A fourth decoder barely speeds decoding up on this 4-core processor.
  Something all the threads share sets the pace, likely memory bandwidth (decoding writes
  867 MiB, copying into the staging ring reads and writes it again, and the integrated
  GPU reads the same memory); not measured.
- Not released, and cold not timed: with no warm saving there was nothing to keep.
- **What could still shorten it:** less memory traffic rather than more threads, e.g.
  decoding each chunk straight into the staging ring instead of into its own buffer
  first. Then the rest of a warm load: the meshes' build (1.1 s) and pipelines plus the
  first frame (about 0.9 s).

### What limits decoding the textures (measured; tried, not released)

The last section suspected memory bandwidth. Measured with a throwaway mode of the
viewer: zone 494's 600 texture chunks (609 MiB stored, 860 MiB decoded) read into memory
first, then decoded in the load's 64 MiB batches on 1 to 4 threads, three times each,
the Windows build on an idle PC (2026-11-15; an i5-7400T, 4 cores without
hyperthreading, one 8 GB stick of DDR4-2400, so one memory channel). "Reused" decodes
each thread's chunks into one buffer of its own instead of a new one for each; "copy"
copies 860 MiB between two 64 MiB buffers, split over the threads.

| Zone 494, 860 MiB | 1 thread | 2 | 3 | 4 |
|---|---|---|---|---|
| Decoded into new buffers (as a load does) | 3.20 s | 2.04 s | 1.67 s | 1.58 s |
| Decoded into reused buffers | 2.49 s | 1.45 s | 1.13 s | 1.05 s |
| Copied | 0.15 s | 0.16 s | 0.18 s | 0.19 s |
| The Linux build, same PC: new buffers | 2.02 to 2.28 s | 1.22 s | 0.96 to 1.03 s | 0.92 to 0.98 s |
| The Linux build: reused buffers | 1.76 s | 1.07 s | 0.85 s | 0.80 s |

(The Linux rows were taken with the faster scatter below in both builds; with it the
Windows build's 3-thread decoding into new buffers is 1.58 s.)

- **Memory bandwidth isn't what limits decoding.** One thread copying already fills the
  memory channel (about 5.8 GB/s each way); four decoding threads move a fraction of
  that, and a fourth thread still adds 5 to 8%.
- **Two costs are the Windows build's own.** The same processor decodes the zone in
  0.96 to 1.03 s on 3 threads as a Linux build against the Windows build's 1.58 s:
  - zstd itself is about 25% slower (0.84 s against 0.67 s on one thread for the 352 MiB
    of gathered frames): the zstd-sys crate leaves out zstd's assembly Huffman decoder
    on every Windows target and builds the C with MSVC, and block-compressed textures
    (609 to 860 MiB) are mostly Huffman-coded literals;
  - new buffers cost about 0.55 s of the 3-thread 1.58 to 1.67 s, against about 0.1 s on
    Linux.
- **Scattering the gathered fields back** (BC1, BC3 and BC5 frames store each field of
  their blocks together, `docs/SIZES.md`) went over the output once per field, a 2- to
  6-byte copy at a time: 0.33 s on one thread for those 352 MiB. In one pass with the
  fields' sizes fixed when compiled it takes 0.05 s, byte for byte the same over all of
  them; but that is only about 0.09 s of decoding on 3 threads, not seen in a load.

Tried against the new buffers, on branches kept locally, neither released:

- **A pool of decode buffers**, each handed back when its texture is uploaded
  (`tried/decode-pool`): 55% of the bytes went into a buffer used before, and decoding
  in the load went from 1.77 to 1.85 s down to 1.66 to 1.76 s (three loads each), the
  loads within noise (4.54 to 4.77 s against 4.35 to 4.87 s). A buffer used before is no
  longer in the processor's cache, so it saved only the page faults, little of the gap.
- **mimalloc as the Windows build's allocator**, with the one-pass scatter
  (`tried/mimalloc`). Zone 494 headless along garage-drive.path, loads alternating with
  0.39.0 (the PC idle, 4.6 GB free for the warm loads; cold after reading 12 GiB of
  other packs, different ones each time):

| Zone 494 | Texture stage wait, warm | Load, warm | Load, cold | Peak working set (`--shot`) |
|---|---|---|---|---|
| 0.39.0 | 0.9 s | 4.42, 4.52, 4.60, 4.58 s | 21.89, 22.02 s | 2,014 MiB |
| mimalloc, one-pass scatter | 0.8 s | 4.15, 4.09, 4.00, 4.06 s | 31.98, 23.99 s | 2,496 MiB |

  - **Warm, 0.45 s faster (about 10%),** more than the texture stage's share (0.1 to
    0.2 s): the rest of the load allocates less slowly too. The picture is byte for byte
    0.39.0's.
  - **But it holds 482 MiB more at its peak**, freed memory kept for later, and cold
    loads got 2 to 10 s slower: on this 8 GB PC that memory comes out of the file cache
    the cold load is reading through. Not released: it costs cold.
- **mimalloc giving freed memory back sooner** (its `purge_delay` option: 1 s by default
  in mimalloc 3, which the crate builds). Loads alternating, the PC idle 5 minutes first
  (2026-11-15), three warm-up rounds discarded:

| Zone 494 | Load, warm | Load, cold | Peak working set (`--shot`; cold loads) |
|---|---|---|---|
| 0.39.0 | 4.52, 4.36, 4.68, 4.45 s (median 4.49) | 21.81, 21.90 s | 2,016 MiB; 2,016, 2,017 MiB |
| mimalloc, purge after 10 ms | 4.35, 4.38, 4.34, 4.60 s (median 4.37) | 21.89, 21.89 s | 2,023 to 2,038 MiB; 2,116, 2,109 MiB |
| mimalloc, purge after 1 s (its default) | 4.37, 4.00, 4.21, 4.03 s (median 4.12) | | 2,352 to 2,510 MiB |

  - **The saving is the memory it keeps.** Purged after 10 ms (0 ms and 100 ms peak at
    2,017 and 2,044 MiB), the peak is back and cold is as fast as 0.39.0's, but the warm
    saving falls from about 0.35 s to about 0.1 s, within these loads' spread, and a cold
    load still peaks about 95 MiB higher. Not released: mimalloc stays on
    `tried/mimalloc`.
  - **The one-pass scatter is kept** (0.05 s against 0.33 s for 352 MiB on one thread,
    byte for byte the same; the blocks' field tables are shared with the gather).
- **What could still help:** zstd built with its assembly decoder (clang on Windows;
  zstd-sys turns it off there), and the rest of a warm load: the meshes' build (1.1 s)
  and pipelines plus the first frame (about 0.9 s).

### The rest of a warm load, and the meshes built on threads (0.40.0)

Where a warm load of zone 494 goes besides the textures, timed with throwaway probes in
the Windows build (0.39.0's code; headless along garage-drive.path, the PC idle,
2026-11-15), the median of four warm loads:

| Part | Time | On |
|---|---|---|
| Opening the store: reading its index (0.16 GiB) | 0.12 s | the main thread |
| … decoding the index | 0.35 to 0.54 s | |
| … sorting its partitions | 0.09 s | |
| Read stage: the walk | 0.4 s | |
| … reading the meshes ahead | 0.1 s | |
| … building the meshes | 1.1 s | |
| …… decompressing their chunks | 0.24 s | |
| …… decoding their vertices (`Lod::section`) | 0.44 s | |
| …… their ebx and mesh sets, switches, materials | 0.08 s | |
| …… adding them to the scene (copying, normals and tangents packed) | about 0.33 s | |
| Texture stage (`ZoneRenderer::new`) | 2.1 s | 3 threads decode |
| … the vertex and index buffers (about 230 MiB, each its own staging buffer) | 0.14 to 0.19 s | |
| … the five pipelines | 0.001 s | |
| … descriptor sets, the draws' order | 0.005 s | |
| The GPU (`Gpu::new_headless`) | 0.06 s | |
| The first frame | 0.03 s | |

- **The "about 0.9 s of pipelines and the first frame" above was mostly opening the
  store**, which no earlier line printed. The pipelines take a millisecond: Intel's
  driver keeps its own cache of compiled shaders on disk, so a Vulkan pipeline cache of
  our own would save nothing here (it might on a first run, or another driver).
- **Opening the store is 0.5 to 0.8 s warm but 1.6 to 3.6 s when its index has left
  Windows' file cache** (the first loads after a build): the index is read whole, not
  laid out with the zone. Decoding it (0.35 to 0.54 s) is the next largest part on one
  thread.
- **The meshes' build was the largest part on one thread**, and most of it, decompressing
  and decoding, has no order to keep.

0.40.0 builds the meshes on threads: the main thread plans each mesh (`scene::plan`:
its ebx, mesh set, LOD switches, materials and which LODs to decode, with a copy of their
chunks' stored frames as read ahead), three threads (all but one) decompress and decode
them in that order (`scene::decode`, each with its own decompressor), and the main thread
adds each to the scene as it comes, in order, so the scene is the same and holds no
more than a few decoded meshes at a time. The rules for LODs an install left out, or
that fail, are unchanged.

Zone 494, headless along garage-drive.path, loads alternating with 0.39.0 built from its
tag (2026-11-15, the PC idle over an hour, 2.8 GB free; the first two warm pairs
discarded; cold after reading 12 packs of the old store, different ones each time):

| Zone 494 | Meshes built, warm | Read stage, warm | Load, warm | Load, cold | Peak working set |
|---|---|---|---|---|---|
| 0.39.0 | 1.1 s | 1.6 to 1.7 s | 4.76, 4.53, 4.53, 4.59 s (median 4.56) | 21.88, 21.63 s | 2,015 to 2,019 MiB |
| 0.40.0: on threads | 0.5 to 0.6 s | 1.3 s | 4.04, 4.13, 4.10, 4.09 s (median 4.10) | 21.92, 21.85 s | 2,016 to 2,024 MiB |

- **Warm, 0.46 s faster (10%):** the build goes from 1.1 to 0.5 s, and the read stage
  now waits 0.2 s for the ground (its tiles start only once the ground's layers are
  done, 0.6 s in, and take 0.6 s).
- **Cold, the same:** the read stage waits 1.1 s for the ground instead of 0.4 to 0.5 s;
  the disk is the limit. The same 1,280 reads (63.5 MiB) as 0.39.0: an early version
  took each chunk's frames out of the read-ahead, and a chunk two meshes share was read
  again, 29 more reads.
- **The same picture:** zone 494's `--shot` is byte for byte 0.39.0's, and the peak
  working set is the same.
- **What is left warm:** the ground (its tiles could start before its layers are done:
  they need the layers only to weight them), decoding the store's index (0.35 to 0.54 s),
  the textures' decoding (zstd's assembly decoder) and the vertex buffers' staging
  (0.14 to 0.19 s, one buffer and one wait each).

### The ground's tiles read before its layers are done (0.41.0)

In 0.40.0 the ground's thread waited for the ground's layers (0.6 s warm, 0.9 to 1.0 s
cold) before reading its tiles, though the layers only weight them. Timed apart in the
Windows build (headless along garage-drive.path, the PC idle), the tiles' 0.6 s warm
was 0.1 s of reading and 0.5 s of walking them into the mesh; cold, 1.3 s of reading
and 0.4 s of walking.

0.41.0 reads (and decompresses) the tiles' chunks as soon as the zone's extent is known,
then walks them once the layers are done. The walk no longer allocates for each vertex
and each texel of the ground's map: `Pages::weights` sums its pages without collecting
them, and `emit` keeps its vertices' indices in a grid, not a hash map. On Linux, over
81 tiles around the speedway read from memory, the walk goes from 0.145 to 0.101 s with
the same output (a hash of positions, indices and weights). The read stage now prints
the tiles' read and walk apart (`ground tiles read … s and walked … s`).

Zone 494, loads alternating with 0.40.0 built from its tag (2026-11-15, the PC idle
over 20 minutes; the first three warm pairs discarded; cold after reading 12 packs of
the old store, different ones each time):

| Zone 494 | Waiting for the ground, warm | Read stage, warm | Load, warm | Waiting for the ground, cold | Load, cold | Peak working set |
|---|---|---|---|---|---|---|
| 0.40.0 | 0.2 to 0.3 s | 1.3 s | 4.15, 3.99, 4.46, 4.06 s (median 4.11) | 1.2 to 1.3 s | 22.28, 21.62 s | 2,016 to 2,020 MiB |
| 0.41.0 | 0.0 to 0.1 s | 1.1 to 1.2 s | 4.10, 4.25, 3.81, 3.95 s (median 4.03) | 0.9 to 1.0 s | 21.87, 21.65 s | 2,014 to 2,020 MiB |

- **Warm, the read stage no longer waits for the ground:** the walk takes 0.3 s instead
  of 0.5 s and starts with the tiles already read. The load is about 0.1 s faster, at
  the edge of the noise between loads (3.8 to 4.5 s for both versions).
- **Cold, 0.3 s less waiting for the ground**, within the noise of a whole cold load.
  The disk is still the limit there: the tiles' reads (1.2 to 1.3 s) start only once the
  zone's meshes are read, and the walk (0.2 to 0.3 s) follows them. Walking each tile as
  its chunk arrives would hide most of the walk, but prefetching reads all the chunks in
  one call; not done.
- **The same 1,280 reads (63.5 MiB), the same peak working set, and `--shot` byte for
  byte 0.40.0's.**
- **What is left warm:** decoding the store's index (0.35 to 0.54 s at the start of
  every load), the textures' decoding (zstd's assembly decoder) and the vertex buffers'
  staging (0.14 to 0.19 s).

### The store's index decoded on every core (0.42.0)

Opening the store decoded its index (0.16 GiB: 22 superbundles, 20,739 bundles, 725,215
ebx, 260,398 res and 228,651 chunks, with 85.6 MB of names) on the main thread, then
sorted its 725,215 partition GUIDs, before anything else could start. Timed alone in
the Windows build with a throwaway probe, decoding took 0.38 to 0.51 s, almost all of it
the bundles, one string per asset's name (on Linux, 0.17 s).

0.42.0 first steps over the index without allocating, noting where each bundle starts
(0.03 s), then decodes the bundles on every core in runs of about 1 MiB, putting them
back in order; every check is the same, and the real index encodes back to itself byte
for byte. The partitions' list is placed by its GUIDs' first byte straight from the
index, with no list to sort first, and each run of first bytes is sorted on its own
thread. Alone, in the probe: decoding 0.20 to 0.28 s, the GUIDs 0.06 s instead of about
0.14 s. `nfsh view` now prints how long opening the store took (`store opened in … s`).

Zone 494, loads alternating with 0.41.0 built from its tag (2026-11-15, the PC idle over
five minutes; the first three warm pairs discarded; cold after reading 12 packs of the
old store, different ones each time):

| Zone 494 | Store opened, warm | Load, warm | Load, cold | Peak working set |
|---|---|---|---|---|
| 0.41.0 | (not printed) | 3.89, 3.67, 4.15, 4.19 s (median 4.02) | 21.86, 21.61 s | 2,013 to 2,018 MiB |
| 0.42.0 | 0.31 to 0.78 s | 3.48, 3.98, 3.67, 4.02 s (median 3.83) | 21.43, 21.44 s | 2,018 to 2,026 MiB |

- **Warm, about 0.2 s faster**, as the probe predicts, though within the spread between
  loads (3.5 to 4.2 s for both).
- **Cold, about 0.3 s faster** (1.9 s to open the store, most of it reading the index).
- **Opening the store still varies most with reading its index**: 0.31 to 0.78 s warm,
  with about 2 GB of memory free, as Windows keeps the 0.16 GiB file cached or not.
- **The peak working set is about 6 MiB higher** (0.3%), likely the heap the decoding
  threads leave behind; accepted (docs/DECISIONS.md).
- **The same 1,280 reads (63.5 MiB) and `--shot` byte for byte 0.41.0's.**
- **What is left warm:** the index's read and the names' strings (one allocation per
  asset; fewer would mean a shared name table, a change to `Asset` everywhere), the
  textures' decoding (zstd's assembly decoder) and the vertex buffers' staging.

### zstd built by clang, and the vertex buffers written in place (tried, not released)

The two leads left above, both measured on 2026-11-15 and neither worth releasing.

**zstd's assembly decoder, and clang.** zstd-sys turns zstd's assembly Huffman decoder off
on Windows, and zstd only enables it for compilers that define `__GNUC__`, which neither
MSVC nor clang targeting MSVC does. On Linux the same throwaway decode probe as above
(zone 494's 600 texture chunks, one thread, reused buffers), built with and without
zstd-sys's `no_asm`, took 1.81 to 1.82 s against 1.86 to 1.87 s: **the assembly is worth
about 3%**. The rest of the Windows build's gap is the C compiler: zstd built by clang
23.1.5 (the LLVM release archive's `clang.exe` as `CC_x86_64_pc_windows_msvc`, which also
turns on zstd's runtime BMI2 choice) against MSVC, two runs alternating:

| Zone 494, 860 MiB, Windows build | 1 thread, reused buffers | 3 threads, new buffers (as a load does) |
|---|---|---|
| zstd built by MSVC | 2.20 to 2.24 s | 1.58 to 1.69 s |
| zstd built by clang | 2.01 to 2.03 s | 1.52 to 1.67 s |

- **About 9% faster on one thread, about 0.05 s on a load's three**, within the spread
  between runs; with the assembly too it would be a few hundredths more. Not released: it
  would need clang on every machine that builds nfsh for that.

**The vertex and index buffers written in place.** An integrated GPU's memory is the
processor's, so on one (`DeviceKind::Integrated`) `Buffer::upload` wrote each buffer
straight into host-visible memory: no staging buffer, no copy on the GPU, no wait
(`tried/vertex-buffers-in-place`). Zone 494 headless along garage-drive.path, loads
alternating with 0.43.0 built from its tag (the PC idle five minutes first; the first
three warm pairs discarded; cold after reading other packs):

| Zone 494 | Texture stage: waiting for chunks, the rest | Load, warm | Load, cold | Peak working set |
|---|---|---|---|---|
| 0.43.0 | 0.7 to 0.9 s, 0.5 to 0.6 s | 3.65, 4.04, 4.08, 3.49 s (median 3.85) | 21.60, 21.24 s | 2,019 to 2,025 MiB |
| Written in place | 0.8 to 0.9 s, 0.3 s | 3.80, 18.82, 4.00, 3.52 s (median 3.90) | 21.54, 21.62 s | 2,089 to 2,093 MiB |

- **The buffers' 0.2 s went into waiting for decoded textures:** the staging waits had
  let the three decoding threads get ahead of the upload thread. Warm and cold loads are
  unchanged; the picture is byte for byte 0.43.0's.
- **The peak working set is 70 MiB higher**, the mapped buffers now counted to the
  process, and the 18.82 s load had lost the store's index from Windows' file cache
  (opened in 4.27 s), as when mimalloc held more memory. Not released.
- **What is left warm is decoding the textures**: the upload thread waits on the three
  decoding threads, so it has to be cheaper per byte, or start sooner.

### Which memory to decode the textures into (measured)

A load decodes each texture into a buffer of its own, then copies it into the uploader's
staging memory. Decoding straight into memory the GPU copies from would skip that copy,
if the GPU memory is as quick to write. Measured with a throwaway mode of the viewer
like the section above (zone 494's 600 chunks, 609 MiB stored, 860 MiB decoded, in 11
batches, read into memory first; the Windows build on an idle PC, 2026-11-16; best of
three, two runs within about 2% of each other). "Staging" is the memory gpu-allocator
gives `CpuToGpu` on this PC (device-local, host-visible, coherent, not cached), "cached"
its `GpuToCpu` (the same, and host-cached), "host" ordinary memory, each a ring reused
batch after batch:

| Zone 494, 860 MiB | 1 thread | 2 | 3 | 4 |
|---|---|---|---|---|
| Into new buffers (as a load does) | 2.86 s | 1.86 s | 1.56 s | 1.50 s |
| Into reused buffers | 2.24 s | 1.29 s | 1.00 s | 0.94 s |
| Into a ring of host memory | 2.16 s | 1.22 s | 0.93 s | 0.86 s |
| Into a ring of cached GPU memory | 2.19 s | 1.22 s | 0.93 s | 0.86 s |
| Into a ring of staging memory | 17.75 s | 9.89 s | 7.74 s | 7.28 s |

- **Never decode into the staging memory**: 8 times slower at every thread count. Likely
  because zstd reads back what it has just written (its matches copy earlier output),
  and every read of uncached memory goes over the bus (not measured apart).
- **Cached GPU memory is as quick as host memory**, so a ring of it saves the copy and
  the new buffers both: 1.50 s down to 0.86 s on 4 threads, 0.64 s in this probe. In a
  load, decoding overlaps reading and uploading, so less of it shows; PR #74 builds
  that ring and times it.

### The sky (0.44.0)

The sky (PRs #72 and #73) loads one time of day's textures, read ahead after the zone's
through the same loader. Zone 494, loads alternating with 0.43.0 built from its tag
(2026-11-16, the PC idle over five minutes; the first three warm pairs discarded; cold
after reading 12 packs of the old store, different ones each time):

| Zone 494 | Load, warm | Load, cold | Peak working set | Store reads |
|---|---|---|---|---|
| 0.43.0 | 3.82, 3.78, 3.75, 3.83 s (median 3.80) | 21.45, 21.52 s | 2,020 to 2,024 MiB | 570, 618.3 MiB |
| 0.44.0 (the sky) | 3.78, 3.77, 3.75, 3.70 s (median 3.77) | 21.43, 21.39 s | 2,024 to 2,029 MiB | 577, 623.0 MiB |

- **The sky costs nothing measurable in the load**: warm and cold within the spread
  between loads (the sky found in 0.01 s, its 7 reads and 4.7 MiB behind the zone's).
- **About 4 MiB more at the peak** (0.2%), its textures in memory before their upload.

### Decoding into the staging memory (PR #74, being timed)

The load decodes each texture chunk straight into a ring of cached GPU memory
(`GpuToCpu`, host-cached; no ring where the device has none, as uncached memory is 8
times slower, "Which memory to decode the textures into") and the GPU copies it from
there, its slot given back once the copy is done. A texture that doesn't fit while the
ring is full is decoded into memory of its own, as before, rather than wait. Zone 494,
loads alternating with the sky's build (main at 9620887; 2026-11-16, the PC idle over
five minutes; the first three warm pairs discarded; cold after reading 12 packs of the
old store, different ones each time):

| Zone 494 | Load, warm | Load, cold | Peak working set | Decoded into the ring, warm |
|---|---|---|---|---|
| main (the sky) | 3.65, 3.72, 3.77, 3.68 s (median 3.70) | 21.75, 21.25 s | 2,024 to 2,028 MiB | |
| 256 MiB ring | 3.50, 3.49, 3.51, 3.52 s (median 3.51) | 21.63, 21.53 s | 2,198 to 2,220 MiB | 695 of 860 MiB |

- **Warm, 0.2 s faster (5%)**, every ring load faster than its pair; the texture stage
  uploads in 1.8 s against 2.0 s.
- **Cold, unchanged**: reading the packs is what a cold load waits on.
- **But 192 MiB more at the peak** (9.5%), the ring's memory, which the mimalloc trial
  showed cold loads pay for on this 8 GB PC once it grows by hundreds of MiB.
- **A smaller ring is being timed** (64 and 128 MiB, on the night queue) before PR #74 is
  merged or closed.

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
