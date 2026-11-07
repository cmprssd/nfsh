# Decisions

Dated entries, newest last. Each says what was asked, what was chosen and why. Entries
made mid-build by Claude are marked "decided by Claude, pending review".

## 2026-10-30: Project direction (chosen by the owner)

Settled while writing the brief (`docs/BRIEF.md`), over six rounds of questions.

- **Goal.** Need for Speed: Heat, but "heavily optimized, extremely quick, using the
  fastest tech stack possible": smaller on disk, faster, more reliable. Same nature as
  `cmprssd/swjkja`. Why: the owner remembers the original as heavily bloated, with nearly
  no optimization.
- **Approach: a clean-room engine** that reads the owner's installed Heat data, like
  OpenMW for Morrowind. Why: unlike Jedi Academy, Heat's Frostbite engine and code were
  never released, so there is nothing to port.
- **Reverse engineering covers the data files and the game executable** (disassembly
  allowed, to match physics and logic). Only the project's own descriptions of formats
  and behaviour go in the repo: never decompiled EA code, game data, or anything that
  bypasses the executable's protection.
- **Stack: Rust, Vulkan 1.3 via `ash`, as swjkja.** Why: fastest path with full control,
  and swjkja's tooling and lessons carry over.
- **Platforms: Windows and Linux.**
- **Reuse of swjkja: copy and adapt** (bench scripts, progress page, `.local-ci`,
  renderer setup), not a shared library. Why: the two projects evolve independently and
  neither can break the other.
- **License: GPLv3, public repo from day one.** No EA code is used, so the license is
  free to choose.
- **Progress page on GitHub Pages and `docs/OVERVIEW.md`**, both like swjkja's.

## 2026-10-30: Measuring "faster", "smaller" and "more reliable" (chosen by the owner)

- **Faster: same scene, same PC.** Frame time (average and 1% low) and load time on a
  recorded camera path vs. the original, recorded in `docs/BASELINE.md`. The target is
  to beat the original with the same look (automated screenshot comparison), with no
  fixed multiplier.
- **Smaller: an import step converts Heat's data into nfsh's own compact format**, after
  which the original install can be removed. Size is measured against the original
  install. Why: nfsh reads the install, which would otherwise stay the same size.
- **More reliable: no dedicated check, but it must not crash.** Robustness is a coding
  priority.
- **Minimum hardware: integrated GPUs (Intel HD 630 / Steam Deck class), if possible.**
  Testing is limited to the owner's PC.

## 2026-10-30: Scope and order (chosen by the owner)

- **MVP: a Palm City viewer** that flies through the map from the install, shows frame
  rate and frame time, and toggles day and night (both in the MVP). No audio.
- **First session: a data inventory tool** that opens the install, lists its bundles and
  assets with sizes, and records what's there in the docs. Why: the foundation for the
  import step and the viewer.
- **After the viewer: one drivable car** in free roam, measured against the original's
  feel; then races, police, campaign. Audio arrives with the car.
- **Later:** garage and customization (after driving); asset mods only, no code-mod
  interface; the project's own multiplayer (never EA's servers); importing the owner's
  original save games once the campaign exists.
- **Input: keyboard and mouse, plus gamepad** from the first drivable car.
- **Look: the original's.** A render-scale/upscaler setting for weak GPUs is allowed
  (benchmarks vs. the original stay at native resolution). Visual extras, such as the
  graphics/shader mod the owner remembers, are researched only when the project is
  basically finished, and would be optional settings.

## 2026-10-30: The owner's install

- The owner's copy is the Steam version, at
  `D:\SteamLibrary\steamapps\common\Need for Speed Heat`
  (`/mnt/d/SteamLibrary/steamapps/common/Need for Speed Heat` from WSL). Found on disk
  when the repo was created.

## 2026-10-30: The inventory tool (decided by Claude, pending review)

- **One command, `nfsh inventory [INSTALL]`, printing Markdown.** The same output reads
  in a terminal and goes into `docs/INVENTORY.md` unedited, so the doc is a real run's
  numbers rather than typed ones. Why: one format to keep correct.
- **The install path is an argument (or `NFSH_INSTALL`), with no built-in default.**
  Why: the owner's path is not everyone's, and local-ci already passes it as
  `NFSH_INSTALL`.
- **"Live" bytes are the union of the archive extents that current entries (patch
  applied) point at.** Overlapping or shared extents count once. Why: it measures
  exactly how much of each archive is still read, and so how much the patch left dead.
- **Resource type names appear only when confirmed** (the type hash matches the name
  and the assets carrying it fit); the others show the hash and one example asset.
  Why: guessed names would mislead the import step.
- **Two PRs: the format readers (`crates/frostbite`), then the command.** Why: each
  stays reviewable, and the readers are the base for the import step and the viewer.

## 2026-10-31: The overview and the progress page (decided by Claude, pending review)

- **docs/OVERVIEW.md and its counted numbers follow swjkja's** (`scripts/progress.py`,
  checked by `.local-ci`). The install's numbers are read from `docs/INVENTORY.md`, the
  recorded `nfsh inventory` run, rather than by scanning the install. Why: the check
  stays fast and runs without the game, and OVERVIEW.md can never disagree with
  INVENTORY.md.
- **The page is numbers only for now** (the install, milestones, PRs per day, lines and
  tests, crates, merged PRs), a static `index.html` plus `data.json`, without swjkja's
  `progress_page.py` image step. Why: nothing renders yet, so there is no imagery to
  convert; standard-library Python is enough. The screenshot comparisons arrive with the
  viewer, still only on `gh-pages`.
- **Served from the `gh-pages` branch, one commit force-pushed per publish**, run by
  hand after each merge (`scripts/publish-progress.sh`, which refuses unless HEAD is
  `origin/main`). Why: as swjkja; the page always matches main as merged, and the repo
  doesn't grow with each refresh.

## 2026-10-31: Releases (chosen by the owner)

- **Each shipped version gets a tag and a GitHub Release**, starting with `v0.1.0` (on
  PR #3's merge), with the CHANGELOG section as notes. Made locally, at no cost.

## 2026-10-31: Decompressing the cas blocks (chosen by the owner)

- **Oodle blocks are decompressed by the install's own `oo2core_6_win64.dll`**, loaded
  at run time from the game folder and never shipped, rather than a clean-room Rust
  decoder. Why: exact output from the start, for a fraction of the work. The cost: only
  Windows can load it, so on Linux nfsh can't read compressed entries yet (a native
  decoder, or Wine, can come later), and the tests that decompress real data run on
  the Windows side of local-ci.

## 2026-10-31: `nfsh verify` (decided by Claude, pending review)

- **A command that decompresses every distinct entry the patched tables point at** and
  checks its size: ebx and res their listed size, a bundle's chunk entry the
  blocks covering its logical range (see `docs/FORMATS.md`), and a
  chunk listed only in a `.toc` just has to decompress. It exits non-zero on any failure and lists the first 20. Why:
  it proves the block format and the decoder against the whole install, not a sample,
  and the import step will read everything the same way.
- **SHA-1s are not checked yet.** Why: no hashing code exists, and the size checks
  already catch a wrong decoder; worth adding with the import step.

## 2026-10-31: The import step's format (chosen by the owner)

- **The first import PR only drops the dead data**: live entries are copied into nfsh's
  store with their blocks unchanged (still Oodle). Why: quick and certain; recompressing
  follows in its own PR, where its size is measured against this one.
- **The recompression codec will be zstd.** Why: it reads on Linux and Windows without
  the install's Oodle library (a pure-Rust decoder exists), decodes fast, and at high
  levels with a long window is close to Oodle in size; importing is slow, but runs once.
- **A few large packs plus one index.** Why: easy to copy, few open files, and packs can
  be memory-mapped by the readers.

## 2026-10-31: The store in detail (decided by Claude, pending review)

- **Packs close at 1 GiB.** Why: about 30 packs for the install, each easy to copy, and
  every offset fits the 32-bit location the readers already use.
- **The index is our own big-endian binary format, read with the same bounds-checked
  reader as Heat's tables**, and keeps everything the bundles held: names, SHA-1s,
  sizes, res type, metadata and id, chunk ranges and the bundle's metadata DbObject
  (the readers now keep the last three, which they used to skip). Why: the install can
  go once the store replaces it, so nothing in the tables may be lost; no dependency
  is needed.
- **A store location reuses `Location`, with the pack number as the archive.** Why:
  verify, the viewer and everything else that reads entries works on a store unchanged.
- **Every import checks itself**: it reopens the store, compares the index with the
  tables it came from, and compares each copied entry byte for byte with the archive.
  Why: the install may be deleted afterwards, so the copy must be proven, and the
  check reads only, which costs a second pass but no hashing code.
- **The import refuses a folder holding anything but an earlier store**, which it
  replaces, index first. Why: it deletes files, so it must never touch the owner's.

## 2026-10-31: Recompressing the store with zstd (chosen by the owner)

Measured first, on a sample of every 4th bundle of `Win32/levels/mainlevel/mainlevel`
(its ebx, res and chunks) and every 50th chunk of `Win32/chunks0`: 696 MiB stored as
Oodle, 1,493 MiB decompressed. One zstd frame per
entry, against the Oodle blocks it replaces; decode speeds single-threaded on the
owner's PC:

| zstd | size vs Oodle | encode MB/s | C zstd decode MB/s | ruzstd decode MB/s |
|------|--------------:|------------:|-------------------:|-------------------:|
| level 3 | +12.3% | 61 | 550 | 164 |
| level 9 | +8.9% | 21 | 570 | 167 |
| level 15 | +5.3% | 7.3 | 485 | 152 |
| level 19 | +1.5% | 3.1 | 407 | 131 |
| level 19, ebx dictionary | +0.9% | 4.4 | 420 | 131 |
| one frame per bundle, level 19 | -10.6% | 3.1 | 459 | 157 |

Oodle decoded the same entries at 482 MB/s. Level 22 came out the same size as 19. By
part, level 19 with the dictionary: ebx -16.1%, res +6.5%, a bundle's chunks +4.8%,
`chunks0`'s chunks -2.2%. A frame per bundle wins on ebx (-74%) but needs the whole
bundle decoded to read one entry, and entries shared by several bundles can be in
only one frame.

- **Level 19, one frame per entry, ebx with a dictionary.** The owner chose it knowing
  the store comes out about the same size as the Oodle one: what it buys is a store
  that reads anywhere, without the install's Oodle library, at an import of about 80
  minutes instead of 27. On the whole install it did better than the sample: 28.07 GiB
  against 29.49 GiB with the Oodle blocks (-4.8%), in 117 minutes.
- **Decoded with the C zstd library (the `zstd` crate), not a pure-Rust decoder.**
  Why: it decodes about as fast as Oodle, where ruzstd is 3-4 times slower, and the
  import needs the C encoder anyway. The cost: building needs a C compiler on Linux too.
  This replaces "a pure-Rust decoder" in the entry above.

## 2026-10-31: The zstd store in detail (decided by Claude, pending review)

- **Each frame carries its decompressed size and zstd's content checksum**, and a
  reader refuses a location that isn't exactly one frame. Why: a damaged pack is
  reported, never decoded into wrong data or a huge allocation (frames over 1 GiB are
  refused); the checksum costs 4 bytes an entry.
- **The dictionary is 256 KiB, trained at import on up to 8,000 ebx spread evenly over
  the install's**, and kept in the index; a frame names its dictionary by id. Why: the
  size measured best on the sample; storing it in the index keeps the store one index
  plus packs, and the frame header already says which frames need it.
- **The import uses every core**, decompressing and compressing in batches of 256 MiB
  stored, which are then written in archive order. Why: level 19 runs at about 3 MB/s
  a core, so on one core the import would take hours.
- **The self-check decompresses both sides**: every entry from the install (Oodle)
  and from the store (zstd) must be identical. Why: the install may be deleted
  afterwards, and this proves the copy end to end, the codec included.
- **`nfsh verify` accepts a store as well as an install**, checking the same sizes.
  Why: it is the way to check a store on a machine without the game, Linux included.

## 2026-10-31: The Palm City viewer's first steps (chosen by the owner)

Reading the level first showed its shape (FORMATS.md, "Palm City"): a root partition
with the roads and sub-level references, and 232 streaming zones, each with four
lighting variants.

- **One zone first, complete (meshes and textures), then zones outward**, rather than
  the whole city untextured. Why: it looks like Heat early, so the screenshot
  comparison with the original can start before the whole map loads.
- **The day/night toggle switches midday and tealnight**, the zones' day and night
  lighting; overcast and sunset can come later as extras. Why: the brief asks for a
  toggle, and two lighting setups are half the matching work of four.
- **The viewer reads nfsh's store only**, not the install. Why: it works on Linux
  without Oodle, there is one read path to test, and load time is measured on our
  own format.

The plan, one PR each (decided by Claude, pending review):

1. An ebx reader in `crates/frostbite`: a partition's types, instances, fields,
   arrays and references, bounds-checked; a test reads every ebx in the store, and
   `nfsh ebx` prints a partition as text.
2. The level graph: from `levels/mainlevel/mainlevel` to the zones and what each
   places (meshes, transforms), reported by a command.
3. Meshes: a `RigidMeshAsset`'s geometry from its res and chunks.
4. Textures: the `Texture` res header and its chunk's pixels (block-compressed
   formats).
5. The viewer: a Vulkan window, fly camera, frame time shown, one zone drawn untextured.
6. Textured and lit, with the midday/tealnight toggle; then zones outward.

## 2026-10-31: The ebx reader (decided by Claude, pending review)

- **`Partition::parse` checks the whole layout before anything is read**: sections end
  at the meta size, every type's fields are in range, every instance fits the
  instance data, every array's stored count matches its record and its elements fit.
  Reading a value afterwards can still fail (a string, array or instance index out of
  range), and returns an error. Why: the viewer reads tens of thousands of partitions;
  a bad one should be found at once, with a message, and never crash.
- **Values are read on demand, not decoded into a tree**: a partition borrows its
  bytes, and `value`/`element` read one field at a time. Why: the level root has
  23,599 instances and the viewer needs a handful of fields from most of them.
- **Kinds not worked out are reported, not guessed**: their values come back as
  `Value::Other(kind, bytes)`, with the bytes only where the width is measured (0x09,
  0x16). Why: a wrong meaning would misread quietly; an explicit "not worked out" shows
  where the format still needs work.
- **The store test reads every value of every ebx** (Windows side, `NFSH_STORE`, like
  the install test) and checks the rules `parse` does not enforce (class references
  land on instances of the field's class, non-empty arrays have the field's type,
  fields fit their types, the header counts). Why: the rules were found on a sample of
  2,379 partitions; the test proves them on all 725,119.

## 2026-10-31: The level graph (decided by Claude, pending review)

- **Each ebx's partition GUID is kept in the store's index** (version 3: a list after
  the superbundles, 16 bytes an ebx, 11.6 MB on 155 MB), and `nfsh reindex` adds it to
  a version 2 store. Why: resolving an import means finding a partition by GUID, and
  reading every ebx's header to find one takes 80-90 seconds; a stored list opens with
  the index. The alternatives were a scan at every start (too slow for the viewer) or a
  separate cache file next to the store (two files to keep in step, against "one
  index plus packs"). The import fills the list by reading the packs back, the same
  code as `reindex`, so there is one way it is made; it adds about a minute and a half
  to a two-hour import. Version 2 stays readable so the owner's store needed no new
  import.
- **Every copy is listed; the level's own superbundle's copy is preferred.** Why:
  221,738 partitions are stored 725,215 times, nearly always identical; preferring the
  level's superbundle reads what the game would load for that level, and any copy
  serves otherwise.
- **Sub-levels are followed by `BundleName`, in every ebx of a bundle**, not only its
  root partition. Why: the WorldArt bundle's zone links are in a second partition
  (`overridenassets/…_subworlddata`), not in `worldart` itself; scanning every ebx
  reaches all 232 zones, and `ZoneStreamerNotificationEntityData`, whose `BundleName`
  names no bundle, is left out by type.
- **What is followed is a list of type names; everything else is counted, not
  guessed.** Why: the report then says what the viewer will not draw yet (lights,
  volumes, cloth: about 10,000 objects city-wide) instead of hiding it; an explicit
  list is easy to extend as the viewer grows.
- **An imported blueprint is walked once and its placements reused** wherever it is
  placed; a blueprint inside itself is cut off and counted, as is nesting past 32
  levels. Why: the same props are placed thousands of times, and damaged or unexpected
  data must never loop or crash.
- **`nfsh level STORE [ZONE]`** prints the bundle tree, or one zone's placements as
  one line each (position, axes, mesh). Why: text is what the next PRs (meshes, then
  the viewer) need to check against, and it can be diffed.

## 2026-10-31: Meshes (decided by Claude, pending review)

- **A composite mesh is drawn as one rigid mesh, its parts left in place.** Why: its
  geometry is already assembled (a palm tree, a lap counter, rendered from the decoded
  vertices, look whole), and the part data only matters once something breaks it,
  which the viewer does not do. The 6,738 per-zone uses then need no separate path.
- **Only positions and the first texture coordinates are decoded**; the other vertex
  elements are listed by usage and format, not guessed. Why: the viewer's next steps
  need positions (PR 5) and texture coordinates (PR 6); normals can be computed from
  the triangles until the tangent frame's packing is worked out, and a wrong guess
  would light everything wrongly without an error.
- **Inline geometry (character meshes) is reported as not read yet.** Why: no zone
  places one; the 322 LODs that have it are all characters.
- **The mesh reader keeps its own index of mesh set res and chunks by id**, built
  from the store's index when it opens (about 400,000 chunks, under a second), not a
  new store format. Why: the store already lists every res and chunk with its id; a
  lookup table in memory is enough, and the store format stays as it is.
- **`nfsh mesh STORE NAME [OBJ]`** prints a mesh's LODs and sections and decodes its
  first LOD; with OBJ it writes that LOD as a Wavefront .obj. Why: text can be diffed,
  and an .obj opens in any viewer for a look before nfsh has a window of its own.

## 2026-10-31: Textures (decided by Claude, pending review)

- **Pixels are kept block-compressed, as stored**, not decoded to RGBA. Why: Vulkan
  samples BC1-BC7 directly on every desktop GPU, so the viewer uploads the chunk's
  mips as they are; decoding would take memory and time for nothing. The reader's job
  is to say where each mip of each layer is.
- **The whole mip chain is read from the whole chunk** (a superbundle's chunk, or a
  bundle's chunk entry from offset 0), and a bundle's tail-only entry is not used.
  Why: every texture a placed mesh uses has its whole chunk in the store; the tail
  (the small mips) is what the game loads first while streaming, which the viewer can
  add later if load times call for it. The store test checks that the tail is the
  whole chunk's end, so either source gives the same bytes.
- **Only formats worked out are named; the rest are read but reported.** Why: the
  eleven block-compressed formats cover every texture a placed mesh uses and are
  confirmed by mip sizes and by decoding; the ~70 uncompressed look-up tables and
  noise textures are not needed yet, and a wrong guess would show wrong colours
  without an error.
- **The store's chunk and res lookups are shared** (`Store::whole_chunks`,
  `Store::res_of_type`), used by both the mesh and texture readers. Why: both need
  the same id → location maps, and the viewer will hold one of each.
- **`nfsh texture STORE NAME [DDS]`** prints the header and writes a .dds (DX10
  header, layers regrouped as .dds wants them). Why: a .dds opens in common tools for
  a visual check before nfsh has a window, and keeps the exact compressed bytes.

## 2026-10-31: The viewer's window (decided by Claude, pending review)

- **The renderer is its own crate, `crates/render` (`nfsh-render`)**, with the device,
  swapchain and frame loop adapted from swjkja's, and the window (winit) in `nfsh view`.
  Why: the readers in `frostbite` stay free of Vulkan, and the renderer can be tested
  headless without the command.
- **A zone is packed into one vertex, one index and one instance buffer, with one
  instanced indexed draw per mesh** (199 draws for zone 495's 2,171 placements). Why:
  it is the fewest draws and buffers that keep each mesh once; culling and LODs can
  come when zones outward need them.
- **Normals come from the triangles in the fragment shader** (the screen-space
  derivatives of the world position, turned to face the eye), not from the vertices.
  Why: the tangent frame's packing is not worked out (FORMATS.md, "Meshes"), and a
  face normal needs no memory and no guess; transforms may mirror, so nothing is
  culled and both sides are lit.
- **Frame time is shown in the window's title**, with the frame rate and the GPU's
  timestamps, every half second. Why: it needs no text rendering, and swjkja showed
  it is enough to read performance while flying.
- **`--shot PNG` draws a frame without a window**, from above one corner of the zone
  looking at its middle. Why: a screenshot can be checked on Linux (lavapipe) and on
  the owner's PC without opening anything on the desktop.
- **The overview frames where meshes are placed, not their vertices.** Why: some
  meshes are authored with their vertices already in world space (the speedway
  stand's steps, `bldn_cmp_speedway_stand_01_steps_02_mesh`, bounds near (3917, 96,
  3254)), and their placements then move them about 4 km away; framing the vertices
  showed the zone as a few dots. They are drawn where the data puts them.
- **The frame is sRGB** (window and screenshot alike), so the shader works in linear
  colour; a UNORM window is accepted if it has no sRGB format.

## 2026-10-31: Textured and lit (decided by Claude, pending review)

- **One draw per mesh section, each with its material's colour texture**, the mesh's
  placements shared as instances (780 draws for zone 495, from 199). Why: a section is
  what names a material; merging sections that share a texture can come with culling.
- **A material's colour is the first of `Diffuse`, `ColorAlpha`, `Diffuse_Tiling`,
  `foregroundBasecolor` it names; `ColorAlpha` (the trees and bushes) is cut out where
  its alpha is under a half.** Why: these are the colour parameters zone 495's
  materials use (722 sections name `Diffuse`, 20 `ColorAlpha`); other textures (normal,
  `SMD`, masks) wait for their shaders. Whether a `Diffuse` alpha means anything is not
  known, so it is drawn opaque. A section without texture coordinates (207 of 780, the
  position-only sections) is drawn grey.
- **A texture that can't be read, isn't flat, has an unknown format or can't be sampled
  is drawn grey and listed**, never an error. Why: the viewer must not crash, and the
  list says what is missing. Block-compressed formats are used where the device
  supports them (lavapipe and the HD 630 both do).
- **One descriptor set per texture, bound per draw**, not a bindless array. Why: it needs
  no Vulkan feature beyond 1.0 and no limit on the texture count; the draws are sorted
  by texture, so a zone binds each texture once.
- **Midday and tealnight are the visual environments `lighting_mainlevel_day_02v03_neutral_ve`
  (bundle `tod_day3`) and `lighting_mainlevel_twilight_02_neutral_ve` (`tod_night3`)**,
  and the zones' `_midday`/`_tealnight` bundles (Enlighten's precomputed light) are not
  read yet. Why: the visual environments hold the sun, sky and exposure in plain
  fields; `tod_day3` also holds the midday tunnel lighting and `tod_night3` the
  tealnight radiosity, which ties each to its time of day. Enlighten's data is a format
  of its own.
- **Light is physical where the data says so**: the sun's illuminance (lux) and colour,
  a light meter's exposure (a grey card in the sun, the compensation, then clamped to
  `MinEV`/`MaxEV`), and a filmic tonemap for `FilmicNeutral`. The sky's brightness is
  not in known units (`LuminanceScale` is 983 at midday, 30 at night), so `SkyColor`
  and `GroundColor` are used as they are, after exposure. Why: guessing a unit would
  hide that it is a guess; the image this gives looks right at both times (checked by
  screenshot), and the comparison with the original will tell.
- **The sun's `SunRotationX` is taken as the bearing from +z towards +x, and
  `SunRotationY` as its elevation.** Not checked against the game: without shadows the
  bearing barely shows. The baseline comparison will check it.
- **N switches between midday and tealnight; `--night` starts at tealnight; `--camera
  X,Y,Z,YAW,PITCH` starts somewhere other than the overview.** Why: the toggle is one
  key, and a fixed camera lets a screenshot look closely (it showed the textures'
  orientation) and will be the baseline's camera positions.

## 2026-11-01: The ground under a zone (decided by Claude, pending review)

- **The viewer adds the drivable surfaces near the zone: from all 66 drivable-surface
  zones, the placements whose origin lies within 150 m of the zone's own placements.**
  Why: roads are on their own coarser grid, and nothing found says which art zones a
  drivable zone covers; walking all 66 takes 1.5 s. A piece's origin is its middle
  (road meshes span up to ~300 m), so 150 m reaches the pieces that overlap the zone.
- **Road materials are drawn with the first colour texture, like every other mesh**,
  which draws them pale: their shaders blend layers by masks over several texture
  coordinate sets, which wait for a road shader.
- **This release draws the roads, not the terrain.** Why: the terrain's tree, tiles and
  heights are worked out (docs/FORMATS.md, "Terrain"), but not which nodes own a tile
  record; matching tiles by their samples places only part of the city, and the viewer
  must not depend on a guess. The terrain is the next task.

## 2026-11-01: Drawing the terrain (decided by Claude, pending review)

- **The ground is drawn under the zone's placements and 150 m around them, the same
  margin as the roads, with a vertex every 2 m.** Why: the tiles have a sample every
  metre, which is 2.3 million triangles under zone 495; every second sample keeps the
  hills' shape at a quarter of that (1.2 million). The spacing is one constant in the
  viewer.
- **Each leaf tile is drawn from its own heights, or its nearest ancestor's when its
  chunk has none**; holes (height 0) are left out. Where a tile meets a neighbour of
  coarser detail the edges don't share vertices, so small cracks can show; stitching
  waits for streaming the terrain by distance.
- **The ground is one draw in plain grey**: the colour comes from the terrain's layers,
  which are in the blocks and the layer-combinations res not read yet.


## 2026-11-01: Road materials (decided by Claude, pending review)

Supersedes "Road materials are drawn with the first colour texture" above.

- **A material's look is chosen by its shader and parameters**: shaders named
  `…transparentdcl…` are decals; a material naming `Tarmac_FG_Diffuse` is asphalt;
  anything else takes its first colour texture as before. Why: the shader's name says
  what kind of surface it is (docs/FORMATS.md, "Road materials"), and matching
  parameter names alone would turn the manholes (`rd_dcl_base_parallax`, which also
  has an `alphaSelector`) into decals.
- **Asphalt is its clean layer only**: `Tarmac_FG_Diffuse`'s red, read as
  sRGB-encoded, times `Tarmac_Tint_FG`. The damaged layer, dirt, the `detailMask` and
  its colours are not drawn. Why: the read gives 0.05, the material's own
  `detailColor1`, and dark asphalt in the screenshot; how the layers blend
  (`Tarmac_Blend_Offset`, `…_Contrast`, the mask's channels) is not worked out, and a
  guessed blend would hide that.
- **A decal is blended over the road after everything opaque**: its opacity is its
  texture weighted by `alphaSelector`, times `Opacity` and its vertex colour's red;
  its colour is `lineColor`, or else its texture's red (sRGB-encoded) times its
  vertex colour. It writes no depth and is pulled slightly towards the eye (depth
  bias), and decals keep the scene's order among themselves. Why: these are the
  parameters the decal materials set; the vertex colour is what tells white lines
  from yellow ones, and its red runs 0 to 255 along cracks and tyre marks like a
  fade. Blending happens after the tonemap, in the frame's linear colour.
- **A draw's look reaches the shader as a dynamic uniform** (one 256-byte slot per
  distinct look, a few dozen a zone), the push constants staying the camera and the
  light. Why: the push constants are already the 128 bytes Vulkan guarantees; a
  dynamic offset needs no feature beyond Vulkan 1.0.

## 2026-11-01: The ground's colour (chosen by the owner)

Supersedes "The ground is one draw in plain grey" above.

- **The ground is coloured by each tile's own colour image**, not by blending its
  layers: every terrain tile's chunk ends in a 68 × 68 BC1 image of its ground, the
  layers already blended (docs/FORMATS.md, "Terrain"). The owner chose this over
  reconstructing the layer blend first. Why: it gives the original's colours exactly
  and seamlessly in a small change; the blend (the per-tile masks, the layer-
  combinations res, each layer's textures) has several unknowns and comes later, as
  the detail up close.
- **The colour reaches the shader as vertex colour** (decided by Claude, pending
  review): each ground vertex takes its leaf tile's colour, blended between the four
  nearest texels' middles, or its nearest ancestor's when the leaf's chunk can't be
  read. Why: the ground has a vertex every 2 m, the finest tiles a texel every 2 m,
  so a texture per tile would add 143 images and descriptor sets for no visible
  gain; the vertex colour slot was already there.
- **The colour is read as sRGB-encoded** (decided by Claude, pending review), like
  the meshes' colour textures. Why: read that way, the speedway's grass and asphalt
  come out as in the colour mosaic and match the road meshes' own asphalt beside them.

## 2026-11-01: The ground's detail up close (chosen by the owner)

- **Each layer's tiling is read from the terrain shaders' bytecode**: Heat stores no
  `Tiling` constant for the terrain's `__2d` shaders; the tiling is compiled in
  (docs/FORMATS.md, "Terrain layers"). The owner chose reading it over one fixed
  tiling tuned by eye. Why: it is the original's value. Only the numbers we find and
  our own description of where they are go into the repo, never the bytecode or a
  disassembly of it (the public-repo rule).
- **The layer textures are detail over the colour image**: the tile's colour stays
  the ground's base, and each layer's diffuse, divided by its own average colour and
  weighted by its page, adds the fine detail. The owner chose this over mixing the
  layer textures alone. Why: from afar the colours stay exactly the original's, and
  the tints and blend that are compiled into the shaders don't have to be guessed.
- **Each layer takes its primary scale** (owner, 2026-11-01): its diffuse is drawn
  once, at the finest scale at which every shader with the layer samples it straight
  from the world position (mega grass 0.2 per metre for layer 1), or the finest
  turned one when it has no direct one; the shaders' far-scale and turned extra
  samples are left out (docs/FORMATS.md, "Shader programs"). Why: one sample per
  layer keeps the ground's shader small and fast, and the primary scale is the one
  that sets the look up close.
- **A shared texture's scale is the layer's, not the texture's** (decided by Claude,
  pending review): a scale counts for a layer only when all its shaders sample the
  diffuse at it, so mega grass is 0.2 for layer 1 and 0.25 for layer 4, and a layer
  whose shaders bind a diffuse without sampling it (layer 28) gets none. Why: two
  layers sharing one bound texture sample it at their own scales, and going by the
  texture would give one of them the other's.

## 2026-11-01: The ground's detail as drawn (decided by Claude, pending review)

- **A layer's weight is its share of the tile's pages**: the pages don't sum to 255,
  so each vertex takes each layer's page value over the sum of all the tile's pages
  there (none: no detail). Why: it is the weighted mean the colour evidence used
  (docs/FORMATS.md, "Terrain"), and it needs no guess at the shaders' blend.
- **Detail is `colour × (1 + Σ wᵢ (diffuseᵢ ÷ averageᵢ − 1))`**, the average being
  of the diffuse's 64 × 64 mip, linear. A layer drawn without detail (a road set, no
  scale, or not given a slot) counts as 1, so its share keeps the colour as it is.
  Why: from afar each detail averages to 1 and the tile's colour stays exactly the
  original's.
- **Weights per vertex, eight slots per area**: the ground's vertices (every 2 m)
  carry eight weights in the bytes the meshes use for texture coordinates; the slots
  go to the eight layers with the most area over the drawn ground (leaf tiles, by the
  pages they are drawn with), and the textures are one BC1 array. Why: no extra
  vertex buffer, image per tile or descriptor; the masks are 1 m, the colour 2 m,
  so 2 m blends lose little.
- **One diffuse per layer: its finest scaled one**, ties by name; the road sets'
  `drv_…_d` maps are left out (their channels are not plain colour). Why: the
  owner's one-sample rule, applied to the layers that have two diffuse maps.

## 2026-11-01: The asphalt's layers (decided by Claude, pending review)

The owner chose the road layers as the next task. Supersedes "Asphalt is its clean
layer only" above.

- **Asphalt is drawn as its shader draws it, as far as the colour goes**: the clean
  and damaged layers blended by the road's mask, the five detail colours laid over
  them, the wheel paths tinted (docs/FORMATS.md, "Asphalt"). Why: it is how the
  original gets its patches, cracks and worn lanes; a road was a flat grey before.
  Reading the programs to learn it follows the terrain's precedent: only our own
  description and the numbers go into the repo.
- **Left out for now**: the blend into the terrain's colour at the road's edges, the
  wet look, occlusion, the normal maps and `Dirt_Height_*`. Why: the terrain blend
  needs the ground's colour where the road is drawn (a texture nfsh doesn't have:
  the ground's colour is per vertex), and nfsh draws no normal maps anywhere yet.
- **The layers' grey stays sRGB-encoded** (as in "Road materials"), and the other
  channels (the amounts, the height, the mask) are read as they are. Why: the grey
  was checked against the screenshot; the textures are stored unorm, so whether the
  game decodes any of them is not settled, and a mask is a weight, not a colour.
- **Asphalt without all of its layers is its clean layer's grey**, as before: a
  material missing the damaged layer or the mask, or a section without the mask's
  coordinates. A material without a noise takes the shader's (`drv_road_detail_d`).
  Why: a half-drawn blend with a grey stand-in would look wrong in a new way.
- **A draw binds up to five textures**: its colour, and for asphalt its other four,
  in one descriptor set per combination of textures used (grey where a draw has
  none). The look's uniform slot grows to 512 bytes. Why: the fewest changes to
  how draws are made, sorted and bound; a zone uses a few dozen combinations.

## 2026-11-01: The asphalt's edges (decided by Claude, pending review)

The owner chose the road edges as the next task. Supersedes "Left out for now: the
blend into the terrain's colour" in "The asphalt's layers" above.

- **Asphalt gives way to the ground as its shader says**: only with
  `rd_drv_blendwithterrain` and a `Shoulder_NH`, by the shoulder's blue in the third
  texture coordinates, the vertex colour's red and the dirt (the mask's green times
  the layers' height under `Dirt_Height_Offset`) (docs/FORMATS.md, "Asphalt", step 5).
  Why: it is the original's rule, read from its programs like the layers before it;
  the wet road's shader has no such step, so those roads keep their hard edge.
- **What shows through is nfsh's own ground, exactly as drawn**: the walk that makes
  the ground's vertices also fills a map of its colour and detail weights over the
  same area, a texel every 2 m (larger when the area is over 8 km, keeping it at most
  4,096 a side), and asphalt draws the ground's colour and detail from it at each
  pixel. Why: the original samples its terrain's virtual texture, which nfsh doesn't
  have; drawing the same ground the same way leaves no seam where the road's mesh
  ends. Per-vertex colours on the road meshes would be too coarse along a road and
  can't follow an instanced mesh.
- **The clamp to 0.04 to 0.5 covers the ground's share too**, as in the original.
  Why: it is the shader's order; the ground's colours sit inside that range.
- **A road mesh without vertex colours keeps its road** (their red counts as 1).
  Why: nfsh fills a missing vertex colour with white, which decals need; no asphalt
  near zone 495 lacks them.

## 2026-11-01: The meshes' own normals

The owner chose the normal maps as the next task, and to decode the meshes' stored
tangent frame first (over a tangent frame built per pixel from screen-space
derivatives), the normal maps to follow with it.

- **Meshes are lit by their own normals** (element 0x34, docs/FORMATS.md, "Meshes"),
  smooth across a curved surface as the art made them; a section without the element
  and the ground keep the triangle's own. Why: it is the original's shading, and the normal maps need
  the same frame's tangents next. (decided by Claude, pending review: what follows)
- **Decoded on the CPU when a zone loads, uploaded as four signed bytes a vertex**,
  the fourth saying whether there is one. Why: the same four bytes a vertex as the
  packed frame, one tested decoder in `frostbite::mesh` rather than one in Rust and
  one in the shader; the tangent waits for the normal maps.
- **A placement turns a normal by its transform's cofactor**, with the sign of its
  determinant: right under any scale and under mirroring. The side a pixel is seen
  from is still the triangle's: the mesh's normal is turned to that side. Why:
  triangles are drawn from both sides, and a smooth normal can point away from the
  eye on a triangle that faces it, near a silhouette.

## 2026-11-01: Normal maps

The owner chose to draw the meshes' and the asphalt's normal maps in this step (over
the meshes' alone), the ground's layers to follow.

- **A mesh's `Normal` bends its own normal**, by the frame's tangent and bitangent
  (docs/FORMATS.md, "Normal maps": red along u, green along v decreasing, z made up),
  at the colour's coordinates. Only where the section has a frame: without one there
  is no tangent to bend along, and the triangle's normal is drawn as before. Why: it
  is how the original's surfaces get their fine relief, and the frame decoded in
  0.18.0 was for this. (decided by Claude, pending review: what follows)
- **Green's direction was settled from the maps, not the programs**, by which way
  their slopes have no curl. Why: it needs no bytecode, and it agrees with the frame
  read from the meshes; a GPU test pins it.
- **The tangent goes up as four more signed bytes a vertex**, its fourth the
  bitangent's sign; the bitangent is made in the pixel shader from the normal and the
  tangent, turned over under a mirroring placement. Why: the usual layout, the same
  four bytes as the normal; one decoder (`frostbite::mesh::Frame`) for both.
- **A draw binds its normal map at binding 10, the grey stand-in where it has none**:
  grey's red and green are a half, a flat normal, so a missing or failed map draws the
  mesh's own normal with no flag. Why: no new look field or shader branch.
- **Only `Normal`**: the other normal maps (`Dirt_BG_Normal`, `ParallaxNH`, `_NT`,
  `Normal_Tiling`, `Normal02`, `backgroundNormal`, graffiti's in its second
  coordinates) and blue are left out. Why: they appear about a hundred times among
  zone 495's 1,081 materials, beside a `Normal` or in place of it, and what each does
  needs its shader read.
- **Asphalt blends its two layers' normal maps as its colour**: as much of the clean
  one as its colour shows, each at its layer's tiling, flat where the ground shows
  through at its edges. The shoulder's red and green (also a normal) are left out.
  Why: the same split as the colour and the heights; how the original mixes them was
  not read from its programs.

## 2026-11-01: The progress page's before and after

The owner asked for the before-and-after comparisons on the progress page, and for the
page to be monochrome, as a rule for every cmprssd project (`~/projects/cmprssd/CLAUDE.md`).

- **Monochrome**: greys only, in light and dark; bars are told apart by lightness and
  hatching, the growth chart's two lines by dashes and markers, the milestones' states
  by fill, outline and dashed outline. Only the renders keep their colour.
- **Every release since 0.10.0, at five fixed views of zone 495** (`bench/history-views.txt`),
  each release rebuilt from its tag. Why: 0.10.0 is the first with `--camera`; tags
  name every visible step already, so there is no milestones list to keep up, and a
  release's label comes from this repo's milestone table. (decided by Claude, pending
  review: which views)
- **Rendered on Windows, headless, on the real GPU** with the Windows toolchain. Why:
  lavapipe takes about 6.5 minutes a shot from WSL; no window opens either way. Old
  releases read today's store: its format has only gained readers since 0.10.0.
- **A release is compared with the one before it**, any earlier one on request, and
  each shows the share of pixels it changed (more than 8 in a channel). There is no
  original to compare against until milestone 5's baseline.

## 2026-11-02: The ground's normal maps (decided by Claude, pending review)

Follows "Normal maps" above, which left the ground's layers for next.

- **Each detail layer's normal map is its diffuse's name with `_n`**, drawn at the
  diffuse's coordinates: every scaled diffuse has one at the same scale
  (docs/FORMATS.md, "Each layer's normal maps"). Why: one sample coordinate a layer,
  the owner's one-sample rule; a store test pins the pairing.
- **The ground is bent in a world-aligned frame**: its coordinates are world x and z
  times the scale, so the tangent is x and green runs towards v decreasing, -z, the
  convention the curl of all 17 maps agrees with. The normal bent is the triangle's.
  Why: the ground has no frame of its own and needs none; normals from its heights
  are a separate step.
- **The layers' red and green are weighted as their colours**: a half plus each
  layer's weight times its offset from a half, a layer without a normal map counting
  as flat. Why: the same split as the colour and as asphalt's two layers; how the
  original mixes them is compiled into its shaders and was not read.
- **Their own array at binding 11, a flag a slot**: a second BC1 array beside the
  diffuse one, and 1 in the look's `inverse_mean[i].w` where a slot has a normal map;
  a map that fails is listed and its slot drawn flat. Why: the zeros a missing slot
  holds are not a flat normal in BC1, and the flag also skips the sample.
- **Asphalt's edges take the ground's normal where the ground shows**, mixed with the
  road's by how much of the road shows. Why: the ground under the edge would
  otherwise be the only flat ground, along every road.
- **Left out**: the far detail normals (`t_vrg_wheelerdam_rock_02_n`,
  `t_terrain_mid_distance_details_n`), the layers drawn without detail (the road
  sets', the sand waves') and the SMD maps. Whether the original's coordinates swap
  x and z is not settled (its `__2d` programs sample at `r0.yx`, and which of r0's
  components is world x is not worked out); the diffuse and normal maps are drawn
  the same way, so they agree either way.

## 2026-11-02: The ground's smooth normals

Follows "The ground's normal maps" above, which bent the triangle's normal.

- **Each vertex's normal is the slope of its heights**, a central difference over the
  samples one vertex apart either side (the vertex spacing, so it is the slope the
  drawn mesh has), one-sided beside a hole or past the border, level along an axis
  with neither. Why: the smallest step that is smooth; averaging the triangles'
  normals would give the same away from tile edges but needs the neighbours' vertices.
- **Across tile edges from the heightfield's own border**: the 4 samples past each
  edge are the neighbour's (and its 4 the tile's), on all 14,852 edges between tiles of a size (a store test
  pins it; docs/FORMATS.md, "Terrain"), so both sides of an edge get the same normal
  without looking up the neighbour. A vertex spacing over 4 samples takes the
  one-sided slope at the edge.
- **Sent as the meshes' normals are** (signed bytes, w marking one present), and the
  ground's shader falls back to the triangle's where w is 0. Why: the same buffer
  and attribute the meshes use; no new binding.
- **Left out**: asphalt's edges still bend the road's own normal where the ground
  shows through, not the ground's (the ground map carries no normals).


## 2026-11-02: The sky's light follows the sun's

The night view was barely darker than midday in every release. The sun (the moon at
night, 6 lux) was exposed, but the sky's light was `SkyColor` as it is, about 0.35
at both times: at night's exposure (EV 2.25) as bright as the moon itself.

- **The sky's light is `SkyColor` (and `GroundColor`) times the sun's exposed light
  over midday's** (170,000 lux at EV 15): `SkyColor` stays what the sky gives back
  at midday, and both scale with the sun from there. Asked of the owner: keep midday
  as it is, rather than read `SkyColor` as a share of the sun's light (which brightens
  midday's shade by about 37%). Why scale with the sun: both times' skies are
  procedural, scattering the sun or moon through the same atmosphere; the sky's
  `LuminanceScale` (983 and 30) is not in known units, so it is not used.
- **Left out**: street lights, the night's colour grading and sky, Enlighten's light.
  The moon is straight overhead, so open flat ground stays moonlit.

## 2026-11-02: Asphalt's edges take the ground's normal

Follows "The ground's smooth normals" above, which left this out.

- **The ground's map carries its normals as a fourth layer** (bytes, -1 to 1 as 0 to
  255, alpha 255 where there is one), each texel's from its nearest height sample
  with the same step as the vertices', so the edge is lit as the ground beside it.
  Where the map has none (no heights, a hole), the road's own normal stands in. Why:
  the map is already sampled there; one more layer costs 4 bytes a texel and no new
  binding.
- Measured on the road-edge view (zone 495, midday): 13% of pixels change, by at
  most 5 of 255, as the speedway's ground is nearly level.

## 2026-11-02: The ground layers' axes

Follows "The ground's normal maps" above, which left open whether the original's
layer coordinates swap x and z (its `__2d` programs sample at `r0.yx`).

- **They do not: a layer's u is world x and its v world z, as nfsh draws them.** The
  programs read their input swapped into `r0`, so `r0.yx` undoes it; the ground's
  normal they build from the heightfield is (along the first, up, along the second),
  and the layers' normal maps are turned by it with red going to x and green to -z
  (docs/FORMATS.md, "Shader programs"). Why it settles it: Frostbite's world is
  (x, up, z), and the heightfield's columns run along x in nfsh's own reading, which
  the meshes and roads line up with.
- **No change to the drawing.** Not confirmed with a shot: the drawing does not
  change, and with no screenshot of the original to compare against (docs/BASELINE.md
  is not done) a shot would only show nfsh's own. Each axis's direction (whether v grows with z)
  rests on the runtime constants, which were not read; the normal maps' curl agrees
  with v growing with z.

## 2026-11-02: The ground's far detail

Follows "The ground's normal maps" above, which left the far detail normals out.

- **One far sample a layer** (owner): each weighted layer samples its own far detail
  map at its own scale, as Heat's programs do, rather than one sample a pixel for the
  heaviest layer. Why: exact where layers mix; the owner took the cost, up to 8 more
  samples a ground pixel (24 at most, from 16).
- **On the colour too** (owner): the far map's channel is overlaid on the layer's
  diffuse as on its normal map, from the same sample. Why: no extra cost, and it is
  Heat's large-scale light and dark. The colour then drifts from the tile's colour
  image by that overlay; the average used to divide the diffuse stays the diffuse's
  own.
- **Read from the programs, not typed in**: the far map, scale, channel and strength
  are what every shader with the layer overlays on that layer's diffuse and normal
  map (docs/FORMATS.md, "Shader programs"); a layer whose shaders disagree gets none.
  Where the diffuse's and the normal map's far detail are of different maps or
  scales the normal's is left out (none on the owner's store).
- **The far maps go up once each**, as a third texture array (binding 12), with each
  slot's scale, map, strengths and channel in its look; a far map that fails leaves
  its layers without far detail, and is listed. Why: two 4096² maps serve every
  layer; one per slot would be up to eight copies.
- **The overlay is done before the detail's average is divided out**, so the far
  detail changes the layer's detail as Heat changes the layer's map. A layer without
  a normal map takes the far map's red and green over a flat half, as Heat's overlay
  on a half gives.
- Measured against 0.21.2 on the history views (zone 495, the HD 630): 12% of the
  speedway view's pixels change by more than 2 of 255, at most 18; the road-edge view
  13%, at most 11; the night view by at most 8. The speedway's layers mix their far
  detail gently (colour 0.05 to 0.2, normal 0.1 to 0.3).
- **Left out**: the SMD maps and macro colour the same far sample is overlaid on, the
  blue mask hiding the turned copies' repeats (nfsh draws no turned copies), and the
  road sets' layers.

## 2026-11-03: Culling: only what is in view is drawn

Where a frame's time went on the HD 630 (1280x720, zone 495, the history views,
median GPU time over 60 frames, headless): 30 to 32 ms whatever the view, of which
the opaque meshes (5.08 million triangles) took about 21, asphalt 3 to 6, the
ground (1.17 million) 3.5 to 5, cut out and decals under 1. Drawn at 64x36, with
almost no pixels, it still took 24.5 ms: about 80% of the frame was the vertices,
and only 20 to 58% of the triangles were in view.

- **Frustum culling first, LODs after** (owner, over culling and the game's LODs
  together). Why: it changes nothing on screen, and its saving is measured; the
  LODs' switch distances are not read from the data yet.
- **Per placement, by a box made once**: each instance's box in world space is the
  union of its mesh's sections' vertices as it is placed (the sections share their
  mesh's placements), tested against the view's left, right, top, bottom and near
  planes each frame on the CPU; consecutive visible instances of a draw go in one
  draw call. A box that cannot be read is always drawn. Why: one test per
  placement (a few thousand) costs next to nothing, and the instance buffer stays as
  it is, with nothing written per frame.
- **The ground in 128 m blocks**, each triangle in the block of its middle, each
  block a draw of its own. Why: as one draw the whole ground was always in view; a
  block is about a zone tile's size.
- Measured as above, against 0.22.1: barrels 30.5 to 12.7 ms, speedway 31.4 to
  10.5, asphalt 32.4 to 19.8, road-edge 31.2 to 7.8, night 31.3 to 10.5. The five
  views' shots are identical to 0.22.0's, to the pixel. The asphalt view, which
  looks along the track at the stands, still sees 58% of the triangles: the LODs
  are for that.
- **Left out**: occlusion culling (what a building hides is still drawn), and the
  CPU's time per frame, not measured (one test per placement; the draws only become
  fewer than before, or split where a draw's visible placements are not
  consecutive).

## 2026-11-03: LODs

The asphalt view still saw 58% of the triangles after culling, mostly 150 to 400 m
away, all at their first LOD. The data says when each mesh's LODs take over: every
mesh asset names a `MeshLodGroup` (28 in zone 495) with six switch distances and a
screen-area cut, scaled by the asset's `LodScale` (see `docs/FORMATS.md`, "LOD
groups").

- **The switches are read from the data** (owner, over picking by on-screen size).
  Why: the game's own switches, so the same LOD at the same place.
- **The screen-area cut is applied too** (owner): a mesh covering less of the screen
  than its group's `CullScreenArea` is not drawn.
- **A hard switch** (owner, over a dithered cross-fade): one LOD per placement per
  frame. Why: simplest and cheapest; a pop can show when flying.
- **The distance is to the nearest point of the placement's box** (decided by Claude,
  pending review). Why: measured to its middle, a 200 m road piece under the camera
  was already past its 75 m switch and drawn at its far LOD; the game's road under the
  car is surely its first.
- **`CullScreenArea` is a percentage of the screen**, covered by the sphere around the
  box at that distance, with `CullScale` scaling its radius (decided by Claude,
  pending review). Why: read as a fraction, a barrel (`props_m`, 0.005) would vanish
  14 m away and an armco 38 m away; as a percentage, 143 m and 381 m, a small bush
  47 m, a palm 1.7 km.
- **A road's far LODs are drawn with their baked colour** (`vista_texture`, `_DS`;
  decided by Claude, pending review). Why: it is what the game draws there. It is
  lighter than nfsh's near asphalt: the bake's mean albedo is 0.139 (linear), the
  asphalt layers' red decoded as sRGB 0.12 to 0.14 before their tint, which nfsh
  multiplies by `Tarmac_Tint_FG` (0.57), so the near asphalt is likely too dark (the
  tint may not be a plain multiply); left for the asphalt.
- **Every LOD is uploaded** (2,067,181 vertices for zone 495 instead of 1,356,354),
  each draw tagged with its LOD; each frame each placement chooses one LOD or none,
  and each draw takes the placements that chose its LOD. A mesh whose group can't be
  read keeps its first LOD at any distance.
- Measured on the HD 630 (1280x720, zone 495, 60 frames, headless) against 0.23.0,
  alternating the two builds three times: the medians varied by up to a factor of 2
  between rounds (the GPU was shared or changing clocks), so these are each view's
  fastest frame over the three rounds, which matched 0.23.0's recorded medians on
  main: barrels 12.1 to 9.2 ms, speedway 10.5 to 7.1, asphalt 20.2 to 12.3,
  road-edge 7.4 to 7.4, night 10.5 to 6.9. Pixels differing by more than 16 of 255
  from 0.23.0's shots: barrels 1.5%, speedway 11.8% (the far track, now its baked
  colour), asphalt 1.0%, road-edge 0%, night 11.3% (by more than 64: 0.5%).

## 2026-11-06: The road textures' greys are linear

Follows "LODs" above, whose far-roads bullet left the near asphalt too dark.
Supersedes "Asphalt is its clean layer only" on reading the red as sRGB-encoded.

- **The asphalt's and decals' greys go in as they stand**, not decoded as sRGB. Why:
  the textures are unorm (both layer diffuses of all 88 asphalt materials in zone 495;
  ordinary diffuses are mostly sRGB, so the format is a deliberate difference), and the
  programs that draw the layers (64 of `rd_drv_blendwithterrain`'s 66, 32 of
  `rd_drv_basesurface_wetness`'s 34; the others are not read) multiply the sample by the
  tint with no decoding. The earlier match with `detailColor1` (0.05) was a coincidence.
- **A decal's colour is its texture's red times `lineColor`**, not `lineColor` alone.
  Why: the program's own multiply, with `lineColor`'s default 1 in the same register.
- Measured on the speedway view (median luminance of the shot, after the tonemap):
  the near road was 0.06 to 0.08 against the far road's baked 0.32; it is now 0.27 to
  0.29. The rest, 10 to 15%, is not worked out; the seam no longer shows.
- **Not known**: whether the whole picture is too bright. The road and its bake now
  agree under nfsh's light; there is no baseline from the original to compare the
  light itself against.

## 2026-11-06: Measuring the store (decided by Claude, pending review)

The method; the results and what they allow are in [SIZES.md](SIZES.md). The store's
format is unchanged: none of the savings is built yet.

- **Sizes are counted per stored location**, so the tables add up to the packs (27.92 GiB,
  matching the files), and each saving is applied in turn with identical entries keeping
  one copy while any is needed, so no byte is counted twice.
- **A chunk's owner is whatever names it**, found by searching every ebx and res for chunk
  ids at every byte offset. Why every offset: at multiples of four the terrain's tile
  records were missed, 180 MiB of them.
- **"Nothing names it" is not called unused.** It is reported separately, with the evidence
  for and against, and removing those chunks waits for the owner.
- **The drives are a model, not a replay**: straight lines at about 400 km/h, ignoring
  roads, because the game's own streaming distances are in a `GridResource` not read yet.
  The check `cmprssd`'s rules ask for, a recorded path at the disc's throttled speed,
  still has to be built.


## 2026-11-06: Choosing what gets installed, and what goes on the disc (proposed by Claude, pending review)

Nothing here is built. The owner's request: the installer lets each player choose what gets
installed, such as only the English voice-over when the game is in English, so each setting
makes the install smaller. The question it raises: does the disc hold everything to choose
from, or should a second, smaller disc hold the setup most people want? Sizes are from
[SIZES.md](SIZES.md) and include the store's index unless they say otherwise; "about" marks
sums of its separately measured rows.

**Recommendation: one complete disc, and the choice made when copying.** No second disc.

- **The disc is the source; the choice is the copy.** Everyone gets the same screen, which
  lists each option with its size and the running total. Play from the disc and nothing has
  to be chosen: language is a setting in the game, and nothing is copied except the cache.
- **A smaller disc saves nothing.** There is no size between a dual-layer DVD (7.92 GiB) and
  a BD-25 (23.28 GiB), and both setups need the BD-25:

  | Setup | Size | Disc |
  |---|---:|---|
  | Every language | 23.01 GiB; about 22.35 with block fields gathered | BD-25 |
  | One language | 19.40 GiB; about 18.40 with movie tracks rewritten and block fields gathered | BD-25 |
  | One language, the top mip of every texture and all movie video gone | about 8.8 GiB | still over a dual-layer DVD, and no longer the same look |

  A smaller image also fills only the inner, slower part of the disc, so it is no faster.
- **A disc for "most people" isn't something that can exist.** Org rule: each owner burns
  their own disc from their own install, and it is never published. So there is no stock
  disc to tailor to an audience, and each owner's own burn already fits that owner.
- **The burner uses the same screen, defaulting to everything.** Free space on a disc is worth
  nothing, and keeping every language lets anyone in the household change their mind. Trimming
  is there for when the complete set doesn't fit.
- **The screen preselects the language of Windows, then English.** Most people then just press
  on. It serves a Spanish player as well as an English one, which a fixed English setup wouldn't.
- **The choice can change later.** Adding or removing a language copies or deletes one file,
  from the disc or the install, with no new import. A trimmed install isn't a dead end.

### The data's shape

The store is split into **parts**, each its own pack files, listed in a small manifest with
their sizes and checksums. The installer is then only a copy of the chosen files.

- **Base**, plus **one part per language**. An entry goes in a language's part only if that
  language alone uses it; anything shared, even by two languages, stays in base. Storing
  identical bytes once decides this, so it is built together with that.
- **A check instead of trust:** every entry that base plus any one language refers to must be
  found in those parts. A test runs this for each language, because a missing chunk would
  otherwise show up only as a crash or silence mid-game. `nfsh verify` checks the installed
  parts only, and the game offers only the languages installed.
- **On the disc, the language parts go on the inner tracks and the world on the outer ones.**
  A 6x drive reads about 10.3 MiB/s at the centre and 25.7 at the edge (specifications, not
  measured). Voices need little speed and the city's zones need a lot, so a full disc gives
  the world its fastest tracks. The cost is a seek when a voice is needed mid-drive, so the
  org's throttled check has to include voices, and the chosen language may have to live in the
  local cache.
- **Copying:** 19.40 GiB takes between 13 and 32 minutes at those speeds; not measured.

### Which settings are worth offering

| Setting | Saves | Proposal |
|---|---:|---|
| Voice language | about 0.50 to 0.66 GiB per language (voices 316 to 474 MiB, about 147 of cinematics, about 51 of movies; assuming each language has an equal share); 3.95 GiB from all to one | Offer |
| Text language | at most 0.54 GiB (all of "everything else"); not measured | Measure first; keep all if small. Separate from voice, as subtitles need it |
| Top mip of the 4096 textures | at most about 1.03 GiB; an estimate | Only as the owner's A/B with a comparison, since it loses quality. Not built until chosen |
| Top mip of every texture | about 8.16 GiB | No: `SIZES.md` already rules it out |
| Graphics settings | nothing | Chosen at run time from the same data, except the mips above |
| Movies, characters, cars, one time of day | | No: it is the same game, and the rest is needed |

### What has to be found out first

These decide how many bytes a language part can really hold:

- **Is English needed by the other languages?** 217 MiB of English sit directly in `content/`,
  with no language folder, and may be what the game falls back to. Until that's known, this
  217 MiB stays in base and only `en/` (199 MiB) is removable. The installer can't offer "no
  English" for less than that.
- **Which of a cinematic's 8 audio chunks is which language?** Until then its 1.01 GiB saving
  isn't available, so one language is about 20.41 GiB, which still fits the BD-25.
- **Movie tracks** need a WebM rewrite for 0.34 GiB. They can stay whole at first, as the
  saving isn't needed to fit.
- **Do other owners' installs hold all 8 languages?** Only this install has been read. The
  screen lists what it finds rather than a fixed list.

### What it leaves open

The chunks nothing names (2.72 GiB) are not a player's choice, as nobody could judge them, and
parts don't remove the need to settle them. With them, every language comes to 25.73 GiB, 2.45
over a BD-25. Proposal: leave them out of the disc and the install; the import can run again
from the install. So the original install should not be offered for removal until the original's
chunk reads, or nfsh playing the game, show they are unused. If they turn out to be needed, the
choices are a BD-50 or a disc burned without four or five of the languages.

## 2026-11-07: One disc, stripping down to the bare minimum (decided by the owner)

Settles "Choosing what gets installed" above, and replaces its table of settings: the
owner wants far more offered than that table did.

- **One complete disc.** Approved as proposed.
- **A lot of configurability, down to the true bare minimum.** Why: a player with a very
  weak PC should be able to run the game with nothing they don't need. The settings with
  the biggest effect on size and FPS, and the easiest to understand, are the easy ones to
  reach; the rest go in an advanced section.
- **Screenshots, FPS and size for each choice in the advanced section.** Why: so a player
  can see what a cut costs before making it.
- **The chunks nothing names are deleted, not kept aside, if they aren't needed at all.**
  Why: zero bloat. The store never holds them and no way back is built. The steps are below.

The rest of this entry is Claude's design, pending review. **It looks feasible**, with the
limits stated below. Figures marked "about" are sums and shares of [SIZES.md](SIZES.md), not
measurements of a store built this way.

### Why it isn't the industry's habit

Parts of it are common: Steam's per-language downloads, optional high-resolution texture
packs, and the minimal, typical and full installs of older PC games. Choosing this finely is
rare mostly for reasons of cost: every combination is a configuration to test across many
machines, and patches have to work on all of them. Two of those don't apply here: Heat is
finished, so there are no patches, and nothing online checks the files. The test burden
does apply, and is handled below.

### Size and FPS are mostly different settings

| What it changes | Disk | FPS | Memory |
|---|---|---|---|
| Voices, cutscene video, music | large | none | little |
| Texture detail | large | little, unless memory runs out | large |
| Mesh detail | medium | medium | medium |
| Resolution, draw distance, effects | none | large | some |

So a choice belongs in one of two places:

- **Run-time settings, in the game**, for anything that needs no data removed: no reason to
  fix it at install, and it can change at any moment.
- **Install settings** only for what removes data. Changing one later means copying from
  the disc or the install again.

Texture and mesh detail are in both: the game can avoid using the data without it being
removed, and the installer removes it to save disk.

### The settings

**Easy, on the first screen:** the languages, one **Look** choice (original, reduced,
minimum) that sets everything below, and a test of this PC. The default is the original look.

**Advanced, on data:**

| Setting | Saves | Left after each row, about |
|---|---:|---:|
| One language, everything lossless | | 18.4 GiB |
| Top mip of every texture | about 8.16 GiB | 10.2 GiB |
| Second mip | about 2.0 GiB | 8.2 GiB |
| Third mip | about 0.5 GiB | 7.7 GiB |
| Meshes' LOD 0 and 1 | 2.82 GiB | 4.9 GiB |
| Cutscene video | 1.46 GiB | 3.4 GiB |
| All voices (the one language's voices, cinematics and movie audio) | about 0.6 GiB | 2.8 GiB |
| Music | 0.37 GiB | 2.4 GiB |

**The bare minimum is therefore about 3 GiB**, against the install's 34.45: the last figure
plus most of the 0.66 GiB of gathered block fields, which shrink as textures go. It would
look far from the original, with 4096 textures at 512 and 2048 at 256. Whether it works is
not known until each cut is tried: dropping voices needs subtitles, and removing a video
means skipping it.

**Not offered:** anything the game's rules need, such as collision, missions and cars,
because a save or a mission could then refer to something absent.

### Screenshots, FPS and size

- **Size: exact.** The manifest holds each part's size, so nothing needs estimating.
- **Screenshots: rendered by nfsh on the PC that builds the disc,** from fixed views at each
  setting's levels, and kept on the disc only. Game imagery stays out of the public repo. The
  tool exists: `bench/history.sh` already renders fixed views headless. The progress page's
  100 images take 16.6 MB, so 150 would be about 25 MB. One image per level of one setting,
  never one for every combination, which would be impossible.
- **FPS: measured on the player's PC, not estimated.** A table of FPS from one PC would be
  wrong for every other. A **Test this setup** button loads a small fixed scene with the
  exact settings chosen, draws it for a few seconds, and shows the frame time and that very
  picture. It tests the real combination, so no table of combinations is needed. The first
  screen runs it for the three Look choices. Until that test has run, the screen shows no FPS.
- **The limit:** a fixed scene measures the graphics card. Traffic, physics and streaming only
  count once there is a recorded drive through a playable game, so it reads as "about".

### Where it's hard

- **The game doesn't exist yet.** nfsh flies through one zone, with no driving, traffic or
  missions, and has few effects to turn off. So the FPS half can't be built now. **Each new
  rendering feature ships with its off switch and its test**, so the settings grow with it.
- **Testing every combination is impossible, so the data must make most of them safe.** Every
  cut only shortens a chain the engine already walks: fewer mips, fewer LODs, no voice line.
  The engine treats everything cuttable as possibly absent. A recorded path then runs with each
  setting alone at its lowest, all lowest, all highest, and pairs of settings, not all of their
  combinations. It must not crash under any of them.
- **It can't be tested on a truly weak PC.** Only the owner's PC is available, and its HD 630
  is the weakest GPU here. The engine can imitate weaker hardware with memory limits and low
  resolutions; a real one could still behave differently, and that won't be known.
- **Vulkan 1.3 sets the floor,** whatever the settings: `crates/render/src/gpu.rs` refuses
  anything older, which rules out older GPUs and drivers. Removing data can't help them. Going
  lower means rewriting rendering for dynamic rendering and synchronization2 and is left for
  later: the brief's minimum stays the HD 630 and Steam Deck class.
- **The store's format has to allow it now.** See below. It is the only part with a deadline.

### The store's format

The tiers above would need texture mips and mesh LODs in parts, as well as languages. But
keeping a texture's mips in different places on the disc would add seeks, the very thing that
makes a disc stutter. So:

- **Every frame in the index is tagged with its part and tier,** and a texture's chunk is
  stored as a frame per tier: top mip, second, third, the rest. This extends the two frames of
  [SIZES.md](SIZES.md).
- **The disc's order and the install's are independent.** The disc is laid out for reading in
  sequence. The copy reads the frames the player chose and writes one file per part, so adding
  or removing a tier later is a matter of whole files.
- **To measure first:** what splitting chunks into more frames costs. `SIZES.md` expects
  little from two frames, and more frames compress worse. Extend `nfsh sizes-trial` before
  building. The trial also gives the second and third mips' real sizes in place of the
  estimates above.

### Deleting the chunks nothing names

- **Before the first disc is burned,** because a BD-R can't be changed afterwards.
- **What would count as proof:** the search for their ids is done; then a trace of the
  original's file reads, mapped to chunks, while the owner plays. A read proves a chunk is used
  and it stays. No read over one session can't prove the opposite, so this needs a long session
  covering the game's parts, and is then still strong evidence, not certainty. What remains is
  covered by the install, or by Steam downloading it again: a mistake costs a re-import and a
  new disc, not the data.
- **Until then** the next store leaves them out, and the original install stays.
