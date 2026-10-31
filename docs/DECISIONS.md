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
