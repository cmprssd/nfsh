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
