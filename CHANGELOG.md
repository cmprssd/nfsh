# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.64.0] - 2026-11-18

### Added

- Layer 3's sand waves get a scale: they are sampled at 0.08 turned by a right angle,
  which the compiler writes as a `mad` by zero. A shader's coordinates are now read
  as their coefficients on the world position (through `mul`, `mad`, `add` and `dp2`,
  negated sources included), so a turn is told from a mirror; no other layer's scale
  changed. FORMATS also says how the waves are mixed with the footprints, and what the
  road sets (layers 8 and 9) draw: a warm grey from their diffuse's red overlaid on
  the global colour's luminance, never their purple (docs/FORMATS.md, "Shader
  programs"). The layers store test checks them.

## [0.63.0] - 2026-11-18

### Added

- What each channel of the terrain layers' textures goes into, read from all 282
  layer programs: an SMD map's red is the smoothness and its green scales the global
  colour (and in five layers gives target 1's value); no program reads its blue or
  alpha. A normal map's blue is a height that weights the layers or gives target 1's
  value, and a road set's diffuse green is its smoothness. Tracing an output now
  follows both arms of a program's branches, so each program's first layer counts
  (docs/FORMATS.md, "Shader programs"). A store test checks them.

## [0.62.0] - 2026-11-18

### Added

- What the textures the engine supplies to the terrain decals hold: the shader
  database's parameter blocks name their constants, and the transform a texture's
  coordinates go through tells it apart. The one fetched at 16 texels is the terrain's
  heightfield (9 decal programs build the ground's normal from it, as the layers do),
  the one sampled once its global colour (17 programs); the layer shaders are
  supplied the same two and their masks. Target 2's second value is the smoothness;
  target 1's stays unnamed (docs/FORMATS.md, "Terrain decals"). Store tests check
  them.

## [0.61.0] - 2026-11-18

### Fixed

- The ground no longer cracks where a tile meets one drawn coarser: the finer tile's
  vertices along their shared side are moved onto the coarser side's straight lines,
  and vertices in the same place agree. A store test checks the ground under zone 495
  has no crack.

## [0.60.0] - 2026-11-18

### Changed

- The ground's tiles are walked as their chunks come off the disk, each leaf drawn once
  its own chunk and its ancestors' have come, instead of after the last read: cold,
  waiting for the ground falls from 0.95 to 0.7 s and zone 494 loads in 21.1 s instead
  of 21.6 s; warm is the same, and the picture byte for byte the same
  (docs/BASELINE.md, "The ground walked as its chunks come").

## [0.59.0] - 2026-11-18

### Added

- How the terrain's tiles join where detail changes is read (docs/FORMATS.md,
  "Terrain", "Where detail changes"; `Heightfield::edge`, `terrain::edge_gap`): with
  each leaf at its finest (its finer chunk's heights when it has them), a tile's edge
  next to a larger tile lies on the larger edge's straight lines, within 2 units
  (1/16 m) on 252 of 278 edges and within 5 on all but 2, so a crack there is at most
  a few centimetres. A store test checks every edge.

## [0.58.0] - 2026-11-18

### Added

- How the terrain decals' programs use their textures is read (docs/FORMATS.md,
  "Terrain decals"; `Decals::coverage`, `dxbc::output_sources`): the decals write the
  terrain layer shaders' own targets, packed the same way (normal, colour as its
  square root, two values, premultiplied by the coverage), and each one's coverage is
  one channel of its own textures, scaled by the vertex's opacity on 36 of the 49
  shaders. The three tyre-track shaders pick red, green or blue of one shared mask. A
  store test checks every program.

## [0.57.0] - 2026-11-18

### Added

- The terrain decals' shaders' textures are read (docs/FORMATS.md, "Terrain decals";
  `Decals::textures`): all 49 shaders the decals name are in Palm City's shader
  database, binding 65 textures by slot (diffuse, normal, masks, a shared noise), each
  named by its ebx. A store test checks them.

## [0.56.0] - 2026-11-18

### Added

- The terrain's decals are read (docs/FORMATS.md, "Terrain decals"; `decals::Decals`):
  the res the terrain entity's `DecalsResource` names holds 2,678 meshes laid on the
  ground (tyre tracks, dirt roads, rivers, fields, road cracks and the like), each with
  its colour, scatter mask and displacement shaders, its bounds and its vertices (x and
  z, the ground's normal and a tangent, texture coordinates and opacity) and
  triangles. A store test checks that they lie on the ground: their heights span the
  ground's under them and their normals are the ground's.

## [0.55.0] - 2026-11-17

### Added

- The level's ground height grid is read (docs/FORMATS.md, "Terrain";
  `terrain::GroundHeights`): the `GroundHeightData` in `levels/mainlevel/default`
  holds the least height of the ground in each of 64 × 64 squares of 256 m, scaled to
  its `HeightSpan`. A store test checks it against the tiles' height bounds: all of
  the sea floor and 3,292 of 3,305 squares of land are within 0.1 m.

## [0.54.0] - 2026-11-17

### Added

- The surface table's last two bytes are read (docs/FORMATS.md, "Terrain";
  `terrain::property`): a packed material is a pair, the material and a modifier, the
  modifier another row of the level's material grid that holds race physics alone
  (`VstFairlySlow`, `VstSlow`, `VstFairlyFast` or `VstMedium` on the ground). A store
  test checks every entry's modifier row.

## [0.53.0] - 2026-11-17

### Added

- The terrain's surface map is named (docs/FORMATS.md, "Terrain"; `Surfaces::table`,
  `terrain::material`): its 16 values are the ground's physics materials, each entry a
  material of the level's material grid (race physics, tyre and debris sounds: dirt,
  grass, forest floor, sugarcane, asphalt...), and the trailing value its default. A
  store test checks the table against the `MaterialGridData` ebx.

## [0.52.0] - 2026-11-17

### Added

- The terrain's scattering is read (docs/FORMATS.md, "Scattering";
  `scatter::Scattering`): the 69 meshes Palm City's 31 terrain layers scatter over the
  ground (grass, bushes, flowers, rocks, twigs, lily pads), each with its mesh set,
  density per quality level, scale range, wind and dissolve settings, and four
  distances. A store test checks every type against the terrain's ebx and its mesh.
  nfsh does not draw them yet.

### Fixed

- docs/FORMATS.md: a terrain layer's index is its bit in the shader names (and its
  place in the scattering), not its place in the `TerrainData`'s `TerrainLayers`, as
  the scattering showed. Also what was found about the 35 × 35 grid after a
  heightfield's coarse heights (it follows the ground's curvature; not decoded).

## [0.51.0] - 2026-11-17

### Added

- The terrain readers read the streaming tree's last three blocks (docs/FORMATS.md,
  "Terrain"). Kind 3 is a surface map: 4-bit values every 2 m over the land, likely the
  ground's surface kinds, indexing a 16-entry table, in run-length coded rows
  (`terrain::Surfaces`). Kinds 2 and 4 are quadtrees with no data of their own
  (`terrain::Outline`); what they are for is not known. A store test checks all three
  and decodes every row of the surface map. nfsh does not use them yet.

## [0.50.0] - 2026-11-17

### Added

- The terrain readers read each layer record's cell pyramid (docs/FORMATS.md,
  "Terrain"): which of the square's 1 + 4 + 16 + 64 cells the layer touches
  (`LayerRecord::touches`), with a tile's pages readable one by one (`Pages::mask`).
  A store test checks it against every page. It also settles that a tile's pages
  come in their layers' ascending order, which nfsh already assumed. nfsh does not
  use the cells yet.

## [0.49.0] - 2026-11-17

### Added

- The terrain readers read each heightfield's coarse heights (docs/FORMATS.md, "After
  the heights"): five grids from 2 × 2 to 17 × 17 points over the tile, each closer to
  the heights than the last and leaning below them (`terrain::coarse_heights`). A store
  test checks them on every heightfield. What Heat uses them for is not known yet, and
  nfsh does not use them.

## [0.48.0] - 2026-11-17

### Added

- The terrain readers read two more of Palm City's terrain parts (docs/FORMATS.md,
  "Terrain"): each heightfield's height bounds, the least and greatest heights over the
  tile and its 4, 16 and 64 blocks (`terrain::height_bounds`), and a tile's finer
  chunk, its four quadrants' heights, pages and colour at twice its detail
  (`terrain::Finer`, `Tile::finer`, `Tree::finer_pages`). A store test checks them on
  every tile. nfsh does not draw the finer chunks yet.

## [0.47.0] - 2026-11-17

### Added

- `nfsh verify` checks every entry of an install against the SHA-1 its bundles list, a
  new "SHA-1 checked" column, and `nfsh import` refuses an install whose bytes don't
  match, so a damaged install is reported instead of copied into the store. Heat's SHA-1s
  are of the stored, still compressed bytes (a bundle's copy of a chunk's end lists the
  whole chunk's); a store's frames keep their own checksums.

## [0.46.0] - 2026-11-17

### Changed

- The load decodes the textures straight into a 128 MiB ring of cached GPU-visible memory
  that the GPU copies from, instead of into buffers of their own and then a staging copy:
  the warm load of zone 494 is about 0.3 s faster (3.56 s against 3.88 s), the cold one
  unchanged, the peak working set 106 MiB higher (docs/BASELINE.md). Devices without
  cached host-visible memory keep the old path.

## [0.45.0] - 2026-11-17

### Added

- `nfsh view` draws the zones around the one named: one ring by default, `--rings N` for
  more, `--rings 0` for the zone alone. The neighbours come from the game's own zone
  streamer lists (docs/FORMATS.md). One ring costs about 1.5 times the load and twice
  the memory (docs/BASELINE.md).

### Changed

- `nfsh level STORE X,Z` keeps the zones' extents in the store (`zone-extents.nfsh`): a
  repeat call takes about a second instead of reading all 205 zones again.

## [0.44.2] - 2026-11-16

### Fixed

- **Trees no longer draw their shadow casters.** 251 sections of the city's vegetation
  (material shader `shaders/shadowmeshalpha`) are geometry that only casts the tree's
  shadow; nfsh drew them as if they were seen, as extra leaf cards inside the canopy.
  They are now left out: not decoded, not drawn, not uploaded (at the speedway, 75
  draws and 55,508 triangles fewer).

## [0.44.1] - 2026-11-16

### Fixed

- **Meshes that store a normal without a tangent are lit by it** (vertex element 0x06,
  four halves; 1,646 sections of the city, mostly cables, then vegetation's trunks,
  cloth and leaves), where they took their triangles' normal. The cables look the same
  (they are flat ribbons); the rest are shaded smoothly.

## [0.44.0] - 2026-11-16

### Added

- **The sky.** `nfsh view` draws the time of day's sky behind everything, where the
  viewer used to clear to black: Heat's own photographed panorama of the upper half of
  the sky, over its gradient below the horizon, and the sun's disc, as bright as the
  data says (its textures times `LuminanceScale` are cd/m²) and exposed like the rest.
  Midday is a clear blue sky with cumulus near the horizon, tealnight a teal sky with
  stars. `--no-sky` leaves it out (black, its textures not loaded), the first of the
  rendering features' off switches. It costs nothing measurable in a load, and about
  4 MiB at the peak (`docs/BASELINE.md`, "The sky").
- **`nfsh ebx` names each import's partition**, where the store lists it.

## [0.43.0] - 2026-11-15

### Added

- **`nfsh view --path FILE --headless --frames DIR`** writes the path's frames to the
  folder DIR as PNGs, 30 a second of path time (`00000.png` onwards), for a video. The
  frame times printed at the end are the frames' own: each is written after it is timed.
- **The progress page is rehauled**: fly-through videos of the speedway by day and night
  and of the drive recorded from Heat, a gallery of eleven views, every release's picture
  of a history view in a strip, a grid of how much each view changed at each release,
  nfsh against Heat, load time and peak memory by release, and the sizes from Heat's
  install to nfsh's installs against the discs. `bench/media.sh` renders the videos and
  gallery; `bench/results.tsv` holds the measurements the page charts.

## [0.42.0] - 2026-11-15

### Changed

- **The store's index is decoded on every core.** Opening the store, at the start of
  every load, decodes its bundles on all the processor's threads and sorts its
  partitions on them too, instead of on one thread: alone, decoding went from 0.38 to
  0.51 s to 0.20 to 0.28 s. Zone 494 loads about 0.2 s faster warm (median 4.02 to
  3.83 s) and 0.3 s faster cold, with the same picture, byte for byte, and the same
  reads; the peak memory is about 6 MiB higher. `nfsh view` prints how long opening the
  store took.

## [0.41.0] - 2026-11-15

### Changed

- **The ground's tiles are read while its layers are worked out, and walked faster.**
  `nfsh view` reads the ground tiles under a zone as soon as the zone's extent is known,
  no longer waiting for the ground's detail layers, which only weight them. The walk
  that turns the tiles into the ground's mesh no longer allocates for each vertex. Zone
  494's read stage no longer waits for the ground when warm (0.2 s before) and waits
  0.3 s less when cold. The picture is the same, byte for byte, with the same reads
  and peak memory.

## [0.40.0] - 2026-11-15

### Changed

- **A zone's meshes are built on threads.** `nfsh view` decompresses and decodes each
  mesh's LODs on all but one of the processor's threads, while the main thread adds
  them to the scene in order. Building zone 494's meshes went from 1.1 s to 0.5 s warm,
  and its whole warm load from about 4.56 s to 4.10 s. Cold loads take as long as
  before (the disk is the limit), with the same reads. The picture is the same, byte for
  byte, and the load's peak memory is unchanged.

## [0.39.0] - 2026-11-15

### Changed

- **A zone's textures are uploaded in batches.** `nfsh view` copies them through one
  reused staging buffer in two halves, several textures to a submit, while the GPU copies
  the other half. It no longer makes a staging buffer and waits for the GPU for each
  texture. Uploading zone 494's 600 textures went from 1.2 to 1.3 s to 0.6 to 0.7 s warm.
  The texture stage saves only 0.1 to 0.2 s, because it now waits on decoding. Cold loads
  take as long as before. The load's peak memory is about 35 MiB lower, and the picture is
  the same, byte for byte.

## [0.38.0] - 2026-11-15

### Changed

- **A warm load decodes a zone's textures on threads.** `nfsh view` decodes each batch
  of textures read ahead on all but one of the processor's threads while the last one
  uploads them. Zone 494's texture stage went from 4.3 to 4.5 s to 2.2 to 2.3 s warm,
  and its whole warm load from about 7.1 s to 4.9 s; cold loads take as long as before
  (the disk is the limit). The picture is the same, byte for byte; the load's peak memory
  grows by about 60 MiB.
- `nfsh view` prints the texture stage's parts: finding the textures, waiting for their
  chunks, decoding, uploading.

## [0.37.0] - 2026-11-14

### Changed

- **A cold load walks a zone's blueprints from memory, and works out the ground on
  threads.** `nfsh view` reads every partition the walk of the zone and of the roads near
  it may need first, in rounds, each in the packs' order; the ground's shader programs
  and tiles are sorted the same way; and the ground's layers and tiles are worked out on
  threads while the zone is read and its meshes built. Zone 494's read stage went from
  7,549 reads to 1,280, and from 12.2 to 14.0 s to 8.0 to 8.2 s replayed on a cold disk;
  in the viewer, loads alternating with 0.36.0's on a busy PC took 8.1 to 9.7 s against
  14.6 to 19.5 s. The picture is the same, byte for byte.
- `nfsh view` prints the read stage's parts and how long each took.

## [0.36.0] - 2026-11-13

### Changed

- **A cold load reads a zone's textures in the packs' order, while it uploads them.**
  `nfsh view` finds every texture first, uploads them in the order they lie in the packs,
  and reads them on a second thread 64 MiB at a time: zone 494's textures went from 1,800
  reads and 19 to 21 s on a cold cache to 570 reads and 11 s, and its whole cold load
  from 35 to 39 s to about 28.5 s. The picture is the same, byte for byte; warm loads take
  as long as before, and the load's peak memory grows by about 60 MiB.

## [0.35.0] - 2026-11-13

### Changed

- **A cold load reads a zone's meshes in the packs' order.** `nfsh view` reads them in
  three rounds (their partitions, then mesh sets and lod groups, then every LOD's
  chunk), merging neighbours into one read: zone 494's meshes went from 3,169 reads and
  13.4 s to 829 reads and 5.0 s on a cold cache, and its read stage from about 21 s to
  14 s. Warm loads take as long as before, about 8 s.

### Added

- `nfsh replay STORE TRACE [--gap BYTES]` reads again what a read trace holds, past
  Windows' file cache, and times it: a cold disk's cost, as often as wanted. nfsh writes
  the trace (a `FILE OFFSET SIZE` line for each store read) to the file
  `NFSH_READ_TRACE` names.
- Why cold loads were slow (docs/BASELINE.md, "Seeks, not reads"): D: is a hard disk,
  about 9 ms a random read, and the readers hopped between packs.
- Three headless runs on an idle PC: 6.3 to 6.6 ms by the median on garage-drive.path;
  0.32.0's 17.0 ms was the PC that day.

## [0.34.0] - 2026-11-13

### Added

- `nfsh view` says how much it read from the store, and in how many reads, for the
  zone's meshes and ground and for its textures.
- nfsh's window run on `bench/paths/garage-drive.path` (docs/BASELINE.md, "The same
  path"): 6.76 ms median a frame and 16.2 ms 1% low, against Heat's 114.0 ms at Low.
- Where zone 494's load goes: about 12 s warm, near zone 495's; its ~42 s are a cold
  file cache, most of it 9,889 small reads for 37 MiB of meshes and ground.

## [0.33.0] - 2026-11-12

### Added

- `nfsh level STORE X,Z` lists the streaming zones whose placements span the point
  (x and z in metres, as a camera path's), smallest first, or the nearest five when
  none does.
- nfsh's numbers on the original's recorded drive, `bench/paths/garage-drive.path`, in
  zone 494 (docs/BASELINE.md, "The same path"): 17.0 ms median a frame headless on the
  HD 630, against Heat's 114.0 ms at Low; load about 50 s.

## [0.32.0] - 2026-11-12

### Added

- `nfsh camera scan|changed|same|show|record`: reads the running original game's camera
  from its memory (read only, Windows) and records it as a camera path, for the baseline.
- The original's half of the baseline (docs/BASELINE.md): Heat's frame times at Low,
  1280x720 (median 96-114 ms on the HD 630), its load from Play Solo to the garage
  (88.8 s), and a recorded drive, `bench/paths/garage-drive.path`.

### Fixed

- The camera reader looks down the game's camera matrix's -z, as Heat stores it, and a
  recording skips a read that jumps over 100 m (a copy cleared for one poll) and starts
  just before the first move.
- Making or destroying a GPU device is done one thread at a time: the Vulkan loader on
  Linux crashed when several did it at once (the draw tests, about one run in eight
  under load).

## [0.31.0] - 2026-11-11

### Added

- `nfsh view --path FILE` flies a recorded camera path (`T X Y Z YAW PITCH` a line)
  instead of the keys, closes at its end, and prints the time to the first frame and the
  frame times along it: mean, median, p99, 1% low and max. `--headless` draws the path
  without a window, a frame every 1/60 s of path time.
- `bench/baseline.sh` measures nfsh on `bench/paths/speedway.path` (zone 495, 40 s),
  headless or in a window, and `docs/BASELINE.md` has the first numbers: 11.6 ms a frame
  on average, 20.6 ms 1% low, on the owner's HD 630. The original Heat is not measured yet.

## [0.30.0] - 2026-11-11

### Changed

- The import stores BC1, BC2, BC3 and BC5 textures with each block's fields gathered (end
  points together, indices together), which zstd compresses better; reading puts the
  blocks back, byte for byte. Still a frame per tier. The store's index is version 5,
  which older releases refuse; 0.30.0 still reads a version 4 store. Needs a re-import.

## [0.29.0] - 2026-11-10

### Changed

- The import puts each of a cinematic's 8 dialogue chunks in its language's part: the
  language whose `Win32/loc` table lists it (English, in the owner's install), or the new
  part `others`, the languages the install has no table for. An English install now leaves
  out the other 7 languages' cinematic lines; an install of any language without a table
  keeps `others` too. Needs a re-import.

## [0.28.0] - 2026-11-10

### Added

- `nfsh install` can cut cutscene video (`--cut video`), voices (`voices`: the spoken
  lines and the cinematics' audio) and music (`music`), each into its own files, and
  `--bare-minimum` cuts all three as well. A chunk is cut only when nothing else uses it.
  The store is unchanged: no re-import.
- Measured: base plus English at the bare minimum is 4.97 GiB, down from 8.49; with every
  tier it is still 20.46 GiB (`docs/SIZES.md`, "Installs").

## [0.27.0] - 2026-11-09

### Added

- `nfsh install STORE DEST` copies the store into an install with only what the player
  keeps: the base, the languages chosen (`--languages en,de`), and every tier but the ones
  cut (`--cut mip0,lod0`, or `--bare-minimum` for the top three texture mips and the meshes'
  LOD 0 and 1). It writes one file per part and tier, so a tier can be added or removed
  later as whole files.
- Every command reads an install as it reads a store. A texture missing its top mips is
  drawn from the mips it has, and a mesh missing its first LODs from the first it has; a
  texture keeps its smallest mip and a mesh its last LOD.
- `nfsh verify` checks an install, listing the entries it leaves out apart from failures.
- Measured: base plus English is 20.46 GiB with every tier and 8.49 GiB at the bare
  minimum, which draws the speedway with 1.2 million triangles instead of 6.4 and 11.7 MiB
  of textures instead of 694 (`docs/SIZES.md`, "Installs").

## [0.26.0] - 2026-11-07

### Changed

- `nfsh import` writes a smaller store: 23.08 GiB instead of 28.07, which fits a 25 GB
  Blu-ray with every language kept and nothing lost. Identical content is stored once, a
  bundle's copy of a chunk's end is read from the whole chunk, and the chunks nothing names
  are left out. Every entry is still checked byte for byte against the install.
- Each texture's chunk is stored as a frame per tier (top mip, second, third, the rest) and
  each frame in the index is tagged with its part (the base or a language) and its tier, so
  a later install can choose what to copy. The import reports the size of each part and tier.
- The store's index is version 4. Releases before 0.26.0 can't read a store made by this
  one; this one still reads older stores. Re-import to get the smaller store.
- `nfsh verify` on a store also checks that the base plus each language finds everything it
  refers to.
- `nfsh sizes-trial` measures a frame per tier, with the tiers' sizes by texture size.

## [0.25.0] - 2026-11-06

### Added

- `nfsh sizes STORE` reports what a store's bytes are: by kind, textures by format and
  size, meshes by LOD, identical bytes, chunks nothing names, and what streaming Palm
  City takes. It reads every entry, about 16 minutes on the owner's PC.
- `nfsh sizes-trial STORE` recompresses a sample of the textures and meshes with their
  fields gathered and compares the sizes, writing nothing.
- `docs/SIZES.md`: the results. The game can fit a 25 GB Blu-ray without losing
  quality; none of those savings are built yet.

## [0.24.1] - 2026-11-06

### Fixed

- `nfsh view` draws near roads at their true brightness, so they no longer turn
  abruptly lighter about 75 m away, where the far roads' baked colour takes over.
  The road's greys were being decoded as sRGB, which the game doesn't do, and made
  them 4 to 5 times too dark. Cracks and road markings follow the same fix.

## [0.24.0] - 2026-11-03

### Changed

- `nfsh view` draws each placed mesh at the level of detail the game's data gives
  for its distance (its lod group's switch distances, times the mesh's own scale),
  and leaves out meshes too small on screen to matter, by the same data. Far roads
  take their baked colour, as the game draws them. On an Intel HD 630 at 1280x720
  the speedway's views take 6.9 to 12.3 ms of GPU time instead of 7.4 to 20.2 (the
  asphalt view 39% less); near meshes look as before, far ones are their simpler
  versions.

## [0.23.0] - 2026-11-03

### Changed

- `nfsh view` draws only what the camera can see: each frame it leaves out the
  placed meshes, and the 128 m blocks of ground, that lie wholly outside the view.
  The picture is unchanged (the history views are identical to the pixel), and on
  an Intel HD 630 at 1280x720 a frame of the speedway takes 7.8 to 19.8 ms of GPU
  time instead of about 31, by view: 1.6 to 4 times faster.

## [0.22.1] - 2026-11-03

### Fixed

- `nfsh view`'s title no longer shows an absurd GPU time (once read as 10¹⁴ ms) when
  the GPU's timestamp counter wraps between a frame's start and end: the counter is
  read only as wide as the GPU says it is, and a reading longer than a frame can last
  is left out of the mean.

## [0.22.0] - 2026-11-02

### Added

- `nfsh view` lays each ground layer's far detail over it, as the game does: a large
  map (repeating every 50 m to 1 km, by layer) overlaid on the layer's colour and on
  its normal map, at the scale and strength the game's own shaders use for that
  layer. It adds faint large-scale light and dark patches and undulation; on the
  speedway's gentle layers about 12% of the ground's pixels change slightly.

## [0.21.2] - 2026-11-02

### Fixed

- Where a road gives way to the ground at its edges, `nfsh view` lights that ground by
  the ground's own slope, as the ground beside it is lit, instead of the road's. On
  the flat speedway the change is slight; it shows where a road meets a bank.

## [0.21.1] - 2026-11-02

### Fixed

- `nfsh view`'s night is night: the sky's light now dims with the moon as it does
  with the sun, instead of staying as bright as at midday, so the night view is about
  a third darker overall and what faces away from the moon goes dark. Midday is
  unchanged.

### Not yet

- Street lights, the night's own sky and colour grading, and the precomputed light
  (Enlighten) the game adds at night.

## [0.21.0] - 2026-11-02

### Changed

- `nfsh view` lights the ground by normals taken from its heights: hills and banks
  shade smoothly instead of in facets 2 m across, with no seam where one terrain tile
  meets the next, and the layers' normal maps bend those normals.

## [0.20.0] - 2026-11-02

### Changed

- `nfsh view` draws the ground's normal maps: each of its layers' grass, gravel,
  sand and forest floor has its own fine relief, which catches the sun as in the
  game, blended between layers as their colours are, and carried on under the road's
  edges where the asphalt gives way to the ground.

### Not yet

- The far detail normals most of the ground's layers also sample (a large rock's and
  a "mid distance details" map), and the road sets' layers.

## [0.19.0] - 2026-11-01

### Changed

- `nfsh view` draws the normal maps on the meshes and the roads: the fine relief the
  art paints in (seams and bolts on metal, grain on wood, the joints in concrete, the
  cracks and grit of the asphalt) now catches the sun and the sky as in the game,
  where those surfaces were flat before.

### Not yet

- The ground's layers have normal maps too; they come next. The ground is still lit
  by its triangles, a vertex every 2 m.

## [0.18.0] - 2026-11-01

### Changed

- `nfsh view` lights the meshes by their own normals, read from the game's meshes,
  instead of each triangle's: barrels, tyres, poles, tanks and every other curved
  shape are now smooth, as in the game, rather than faceted, and edges the art
  rounded off catch the light as it meant them to.

### Not yet

- The normal maps (the fine bumps and grooves in the asphalt, the ground and the
  meshes) come next. The ground is still lit by its triangles, a vertex every 2 m.

## [0.17.0] - 2026-11-01

### Changed

- `nfsh view` blends the roads into the ground at their edges, as the game does: the
  asphalt now ends in a ragged, crumbling edge where the grass and dirt take over,
  instead of a straight line, and the ground shows through in the low spots of
  worn asphalt. The ground seen there is the same grass and earth, with the same
  detail, as just beside the road.

### Not yet

- The asphalt's wet look and its normal maps are not drawn yet. Roads drawn with the
  game's wet-road shader keep their straight edge: that shader has no such blend.

## [0.16.0] - 2026-11-01

### Changed

- `nfsh view` draws the roads' asphalt as the game layers it, instead of one flat
  grey: worn and damaged patches where each road's own mask says, cracks and lighter
  repairs from its detail colours, and the tyre-polished wheel paths. The speedway's
  older inner track and its new lanes now look different.

### Not yet

- The asphalt's blend into the grass at a road's edges, its wet look and its normal
  maps are not drawn yet.

## [0.15.0] - 2026-11-01

### Changed

- `nfsh view` draws the ground sharp up close: each terrain layer's own texture
  (dry grass, earth, gravel, sand) is laid over the ground's colours as detail,
  weighted by the layer's masks and repeated at the scale the game's own terrain
  shaders use (field grass every 10 m, mega grass every 5 m). From afar the colours
  stay the same as before.

### Not yet

- The roads' terrain layers (their own maps hold something other than plain colour),
  the layers' normal maps, and the ground's scattered grass and bushes are not drawn
  yet; at most eight layers get detail in one view.

## [0.14.0] - 2026-11-01

### Changed

- `nfsh view` draws the ground in its own colours instead of plain grey: the dry
  grass and earth around the speedway, its worn infield, sand and the sea's edge, from
  the colour each terrain tile carries (one texel every 2 m on the finest tiles).

### Not yet

- Up close the ground is soft: the sharp detail of its layers (grass, gravel, sand
  textures blended by their masks) and the ground's scattered grass and bushes are not
  drawn yet.

## [0.13.0] - 2026-11-01

### Changed

- `nfsh view` draws the roads in their own colours: the asphalt is dark, from its
  road layer and tint, and road lines, arrows, cracks and tyre marks are laid over it
  as decals, white or yellow as each line says, instead of pale grey roads with pink
  marks.

### Not yet

- The asphalt shows one layer: the damaged layer, dirt, puddles and the mask that
  places them over each road, and the road normal maps, are not drawn yet.

## [0.12.0] - 2026-11-01

### Added

- `nfsh view` draws the ground under the zone and 150 m around it: Palm City's
  terrain heights, so the infield, the hills and the ground between the roads are no
  longer black. It is drawn in plain grey for now, a vertex every 2 m.

### Not yet

- The terrain's colours (its layers and their masks), and where tiles of different
  detail meet, small cracks can show.

## [0.11.0] - 2026-11-01

### Added

- `nfsh view` draws the roads under and around the zone: the speedway's track, the
  streets, car parks and bridges come from Palm City's drivable-surface zones (zone
  495 gains 341 pieces). Their asphalt is drawn paler than in the game for now.

### Not yet

- The terrain (the grass and earth between the roads) is still black: its format is
  partly worked out (docs/FORMATS.md, "Terrain").

## [0.10.0] - 2026-10-31

### Added

- `nfsh view` draws the zone textured: each mesh section with its material's colour
  texture, uploaded block-compressed as stored. Trees and bushes are cut out where
  their texture is clear. A texture that can't be read or sampled is drawn grey and
  listed when the viewer starts, never a crash.
- Lit by Palm City's own time of day: the sun's direction, colour and strength, the
  sky's colours and the camera's exposure come from the game's midday and night
  lighting. N switches between midday and tealnight; `--night` starts at tealnight.
- `--camera X,Y,Z,YAW,PITCH` starts the window, or takes the screenshot, from a given
  point rather than above the zone.

## [0.9.0] - 2026-10-31

### Added

- `nfsh view STORE ZONE` opens a window on one streaming zone of Palm City (a number
  such as 495, as `nfsh level` takes it) and flies through it: mouse to look, W A S D
  to move, Space and C up and down, Shift to go faster, Esc to quit. The title shows
  the frame rate, the frame time and the GPU's time per frame. `--vsync` waits for the
  display; without it frames come as fast as the GPU draws them.
- `nfsh view STORE ZONE --shot PNG` draws one frame of the whole zone without a window
  and writes it as a PNG.
- The renderer behind it (`nfsh-render`, Vulkan 1.3): every mesh the zone places is
  drawn from its first LOD, untextured and lit by a fixed sun, with one instanced draw
  per mesh. A mesh that cannot be read is listed and left out, never a crash.

## [0.8.0] - 2026-10-31

### Added

- `nfsh texture STORE NAME` prints a texture (a `TextureAsset`, as a mesh's materials
  name them): its kind, size, format, texture group and each mip's size.
  `nfsh texture STORE NAME DDS` also writes it, every mip and layer, to the file DDS
  as a .dds, which opens in common image viewers and tools.
- The texture reader behind it (`frostbite::texture`) reads a texture's header and
  its pixels from the store, block-compressed as stored (BC1, BC3, BC4, BC5, BC6H,
  BC7), and lists the textures each of a mesh's materials names (diffuse, normal and
  so on). It checks every header and mip size and never panics on damaged data. A
  test reads every texture Palm City's placed meshes use: 8,015 textures, 4.7 GiB of
  pixels, all their mips.

## [0.7.0] - 2026-10-31

### Added

- `nfsh mesh STORE NAME` prints a mesh (a `RigidMeshAsset` or `CompositeMeshAsset`,
  as `nfsh level` lists them): its LODs, each LOD's sections with their materials,
  triangle and vertex counts, vertex layout and bounds, and decodes the first LOD.
  `nfsh mesh STORE NAME OBJ` also writes that LOD to the file OBJ as a Wavefront
  .obj, which opens in any 3D viewer.
- The mesh reader behind it (`frostbite::mesh`) reads a mesh's geometry from the
  store: positions, the first texture coordinates and triangles, for every LOD. It
  checks every pointer, stream and index and never panics on damaged data. A test
  reads every mesh Palm City places: 5,475 meshes, 14,112 LODs, 48.8 million
  triangles. Breakable props (composite meshes) are read whole, as they stand before
  anything breaks them.

## [0.6.0] - 2026-10-31

### Added

- `nfsh level STORE` prints Palm City's level graph: the 2,034 bundles the level loads,
  found by following its sub-level links, and its 232 streaming zones.
- `nfsh level STORE ZONE` (a zone number such as `495`) prints what that zone places:
  every mesh with its position and axes in world space, following mesh groups,
  layers, prefabs and blueprints across partitions, plus a count of the objects not
  drawn yet (lights, volumes, effects). Across the whole city that is 259,999 placed
  meshes, with every reference resolved.
- `nfsh reindex STORE` adds every ebx's partition GUID to a store's index, so a
  reference into another partition is found at once. Stores made by 0.4.0 and 0.5.0
  still open; run it on them once (about 90 seconds). New imports do it themselves.

## [0.5.0] - 2026-10-31

### Added

- `nfsh ebx STORE NAME` prints one ebx partition from a store as text: its imports,
  then every instance with its fields, structs, arrays, enums by member name, and
  references to other instances or imports. It reads the store only, so it works on
  any system.
- The ebx reader behind it (`frostbite::ebx`) checks a partition's whole layout before
  anything is read and never panics on damaged data. A test reads every value of all
  725,119 ebx partitions in the owner's store.

## [0.4.0] - 2026-10-31

### Changed

- `nfsh import` now recompresses: every entry is decompressed with the install's
  Oodle library and stored as one zstd frame (level 19; ebx with a dictionary trained
  during the import), on every core. The owner's install comes out at 28.07 GiB, down
  from 29.49 GiB with the Oodle blocks kept, in about two hours. The self-check now
  decompresses both sides and compares every entry. The import runs on Windows only,
  since it needs Oodle, and prints its progress pack by pack. Stores written by 0.3.0
  (version 1) are refused; import again.

### Added

- `nfsh verify STORE` checks an nfsh store the way it checks an install, on any system
  and without the game's files.

## [0.3.0] - 2026-10-31

### Added

- `nfsh import [INSTALL] STORE`: copies every entry the patched install still uses into
  nfsh's own store in the folder STORE (an index and packs of about 1 GiB, format in
  `docs/FORMATS.md`), leaving behind the data the patch replaced. Entries keep their
  blocks as is. The import then reopens the store and checks it against the install,
  entry by entry, and prints its size. It runs on Linux and Windows (no decompression
  needed), and refuses a folder that holds anything but an earlier store.

## [0.2.0] - 2026-10-31

### Added

- `nfsh verify [INSTALL]` (Windows): decompresses every entry the install's tables
  point at, using the install's own Oodle library (`oo2core_6_win64.dll`, never
  shipped), checks each against its listed size, and prints a table of entries, stored
  and decompressed bytes and failures.

## [0.1.0] - 2026-10-30

### Added

- `nfsh inventory [INSTALL]`: reads a Need for Speed: Heat install (Data/ with Patch/
  applied) and prints, as Markdown, what it holds: files on disk by kind, how much of
  each cas archive current entries still use, every superbundle's bundles and
  entries, and resource types with stored and decompressed sizes. The owner's install
  is recorded in `docs/INVENTORY.md`.
