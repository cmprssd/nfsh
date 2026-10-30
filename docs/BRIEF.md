> Copied from the project backlog (ntfifwyb/project-backlog) on 2026-10-30; now maintained here.

---
title: nfsh
status: ready
created: 2026-10-30
updated: 2026-10-30
owner: cmprssd
repo: cmprssd/nfsh
tags: [game, engine, performance]
stack: [rust, vulkan, ash]
effort:
priority:
---

# nfsh

## Summary *

nfsh is a heavily optimized version of **Need for Speed: Heat** (Ghost Games / EA, 2019),
in the same spirit as `cmprssd/swjkja`: the same game, but a smaller footprint on disk,
much faster, and more reliable. Built on the fastest tech stack possible.

## Problem *

From the owner's experience playing it years ago, Need for Speed: Heat is heavily
bloated: large on disk, with nearly no optimization. There is no lighter or faster way to
play it today.

## Users and context

- The owner, on their own PC (Windows), playing their own copy of the game.

## Success criteria *

- [ ] When nfsh's Palm City viewer and the original Heat play the same recorded camera
      path on the same PC, nfsh's average and 1%-low frame times are lower.
- [ ] On that path, nfsh's frames match the original's look by automated screenshot
      comparison (as in swjkja).
- [ ] Load time to the first frame of Palm City is lower than the original's, and the
      numbers are recorded in `docs/BASELINE.md`.
- [ ] nfsh's converted copy of the data the viewer needs is smaller on disk than the
      same data in the original install.

## Scope *

### Must (MVP)

- Palm City viewer: reads the map from the owner's Heat install and flies through it,
  showing frame rate and frame time, with a day/night toggle (both in the MVP).
- Benchmarks against the original on the same camera path and PC.
- Progress GitHub page and `docs/OVERVIEW.md`, as in swjkja.

### Later

- Next after the viewer: one drivable car in free roam, handling and physics measured
  against the original's feel; then races, police, campaign.
- Garage and visual customization (parts, wraps), after driving and races work.
- Asset mods (custom cars, wraps, maps) once the engine is playable; no code-mod
  interface for now.
- Multiplayer of the project's own (not EA's servers).
- Import the owner's original Heat save games (cars, money, progress) once the campaign
  exists.
- Audio: none in the viewer; engine and ambient sound arrive with the drivable car.
- Render-scale/upscaler setting for weak GPUs (benchmarks vs the original stay at native
  resolution).
- Much later, when the project is basically finished: research Heat's graphics/shader
  mods (the owner remembers one that made it look much better) and consider visual
  extras like it, as optional settings.

### Not doing

- Shipping any EA code or game data: the owner's install supplies the assets.
- Connecting to EA's online services.

## Constraints

- Development costs $0 (workspace rule); checks run locally with `local-ci`.
- Testing happens only on the owner's PC; integrated-GPU performance is a goal, not
  something tested on real low-end hardware.
- It must not crash: robustness is a coding priority even without a dedicated check.

## Decided

- Name `nfsh` (Need for Speed: Heat), under the `cmprssd` org (`cmprssd/nfsh`). The org
  uses abbreviated repo names, an exception to the full-name rule.
- Goals, in the owner's words: smaller file size, faster game, more reliable; "heavily
  optimized, extremely quick, using the fastest tech stack possible".
- Same nature as `cmprssd/swjkja` (a new, much faster engine for an existing game).
- Clean-room engine reading the owner's installed Heat data (no source to port).
- Speed and size are measured against the original on the same scene and PC.
- Stack: Rust, Vulkan 1.3 via `ash` (as swjkja). Platforms: Windows and Linux.
- Public repo from day one, GPLv3.
- Minimum hardware target: integrated GPUs (Intel HD 630 / Steam Deck class), if possible.
- "Smaller": an import step converts the install into nfsh's own compact format.
- Reverse engineering covers data files and the game executable.
- The owner's Heat is the Steam version.
- Pieces of swjkja are copied and adapted, not shared as a library.
- Input: keyboard and mouse plus gamepad.
- The default look is the original's; no visual extras until the project is basically
  finished.
- A progress page on GitHub Pages and a `docs/OVERVIEW.md` explaining how the project
  works, both just like swjkja's.

## Open questions *

- [x] Approach, given there is no Heat source to port? → Clean-room engine that reads
      the owner's installed Heat data (like OpenMW), reverse engineering Frostbite's
      formats.
- [x] What does the MVP run? → A Palm City viewer: load and fly through the open-world
      map from the install, frame time shown.
- [x] Online? → Single-player first; the project's own multiplayer later.
- [x] How is "faster" proven? → Same scene, same PC: frame time and load time on a
      recorded camera path vs. the original, plus install size on disk (as swjkja's
      BASELINE.md).
- [x] Success thresholds? → Beat the original on the same path and PC, with the same
      look (screenshot comparison); no fixed multiplier.
- [x] Platforms? → Windows and Linux.
- [x] Stack? → Rust + Vulkan 1.3 via `ash`, as swjkja.
- [x] License and visibility? → Public from day one, GPLv3. Plus a progress GitHub page
      and a `docs/OVERVIEW.md`, both like swjkja's.
- [x] Minimum hardware? → Integrated GPUs (Intel HD 630 / Steam Deck class), if
      possible; testing is limited to the owner's PC.
- [x] How is "smaller" achieved? → An import step converts Heat's data into nfsh's own
      compact format; the original install can then be removed. Size is measured against
      the original install.
- [x] How is "more reliable" checked? → No dedicated check, but it must not crash: keep
      that in mind when coding.
- [x] How far may reverse engineering go? → Data files and the game executable
      (disassembly allowed, to match physics and logic).
- [x] Day and night in the MVP viewer? → Both, toggleable, from the start.
- [x] First session? → A data inventory tool (see First session).
- [x] Where is the install? → Steam (the owner's Steam library; exact path to confirm at
      start).
- [x] Reuse swjkja? → Copy and adapt the useful pieces (bench scripts, progress page,
      `.local-ci`, renderer setup); the repos then evolve independently.
- [x] What comes after the viewer? → A drivable car, then races, police, campaign.
- [x] Customization? → Later, after driving.
- [x] Mods? → Later, asset mods only.
- [x] Input? → Keyboard/mouse for the viewer, gamepad from the first drivable car.
- [x] Audio in the MVP? → No.
- [x] Visual extras? → Original look first. Much later, research graphics/shader mods for
      Heat and consider extras then.
- [x] Upscaling? → Yes, as a setting.
- [x] Original save games? → Import them, once the campaign exists.

## Risks and unknowns

- No source code: everything (formats, physics, handling, AI, rendering) has to be
  reverse engineered or reimplemented. Far larger than swjkja, which had a source port.
- Frostbite's data formats (superbundles, casc catalogs, EBX/RES) are undocumented
  officially; community tools exist but may be incomplete for Heat.
- Legal: the repo is public and the executable is disassembled. EA's EULA and the
  anti-tamper on the executable constrain this; decompiled EA code, game data and
  anything that bypasses protection must never be committed. Only the project's own
  descriptions of formats and behaviour go in the repo.
- Scale of the original: a large open world with modern Frostbite rendering, so "same
  look" may be out of reach; matching would need to be measured, as in swjkja.

## First session

- A data inventory tool: a command that opens the owner's Heat install, lists its
  bundles and assets with sizes, and records what's there in the docs, measured against
  the install size. The foundation for the import step and the viewer.

## References

- `cmprssd/swjkja` (`~/projects/cmprssd/swjkja`): the model this project follows
  (README, `docs/OVERVIEW.md`, `docs/DECISIONS.md` "Project direction").

## Log

- 2026-10-30: created.
- 2026-10-30: round 1: clean-room engine, Palm City viewer MVP, single-player first, measured benchmarks.
- 2026-10-30: round 2: Rust + ash, Windows and Linux, beat original with same look, public GPLv3, progress page and OVERVIEW.md.
- 2026-10-30: round 3: integrated-GPU target, compact converted data, no-crash priority, executable RE allowed.
- 2026-10-30: round 4: day and night in MVP, data inventory first, Steam install, copy-and-adapt from swjkja.
- 2026-10-30: round 5: renamed nfwh → nfsh (owner's correction); drivable car next, customization and asset mods later, keyboard + gamepad.
- 2026-10-30: round 6: no audio in MVP, original look first (shader-mod research much later), upscaling setting, import original saves.
- 2026-10-30: idea → ready: all open questions answered over six rounds
