# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

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
