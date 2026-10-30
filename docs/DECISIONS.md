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
