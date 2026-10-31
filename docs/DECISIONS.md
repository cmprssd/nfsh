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
