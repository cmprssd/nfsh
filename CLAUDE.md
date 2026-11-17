# CLAUDE.md

nfsh is a new engine for **Need for Speed: Heat** in Rust (Vulkan 1.3 via `ash`), written
clean-room: Heat's code was never released, so file formats and behaviour are reverse
engineered. Same game, same look, much smaller on disk, much faster, more reliable; the
same nature as `cmprssd/swjkja` (`~/organizations/cmprssd/swjkja`), whose tooling is copied
and adapted here. It ships no game data: it reads the owner's install at
`D:\SteamLibrary\steamapps\common\Need for Speed Heat`
(`/mnt/d/SteamLibrary/steamapps/common/Need for Speed Heat` from WSL).

Where each format, decoder, system and tool lives is `docs/MAP.md`: read it before
searching the tree, and update it in any PR that adds, moves or renames a module.
How it all works is `docs/OVERVIEW.md`. The spec is `docs/BRIEF.md`; decisions are in
`docs/DECISIONS.md` (dated `##` entries, newest last; mid-build ones marked "decided by
Claude, pending review").

## Not doing

- Shipping any EA code or game data: the owner's install supplies the assets.
- Connecting to EA's online services.

## Constraints

- Development costs $0 (workspace rule); checks run locally with `local-ci`.
- Testing happens only on the owner's PC; integrated-GPU performance is a goal, not
  something tested on real low-end hardware.
- It must not crash: robustness is a coding priority even without a dedicated check.

## Measuring

- Timings go on `nightq` (`nightq add --timing`), which runs only while the PC is idle;
  never wait for a result, park the task in the night handoff. Heavy jobs run under
  `machine-guard run`.
- `bench/baseline.sh` builds nfsh for Windows and flies a camera path (headless by default;
  `MODE=window` refuses unless the PC has had no input for `NFSH_IDLE_SECONDS`, 120 by
  default). `RINGS=0` keeps numbers comparable across releases. Warm loads need the PC
  idle and about 4 GB free; cold ones come after reading something big enough to push the
  store out of the file cache. Results go in `docs/BASELINE.md` and `bench/results.tsv`.

## Rules

- **The repo is public.** Never commit game data, decompiled or disassembled EA code, or
  anything that bypasses the executable's protection. Only our own descriptions of
  formats and behaviour go in.
- Self-merge once `local-ci` passes (workspace default).
- **The numbers are counted, not typed.** `docs/OVERVIEW.md`'s `<!-- progress:… -->`
  blocks come from `python3 scripts/progress.py` (git, the tree, `docs/INVENTORY.md`);
  `.local-ci` fails when they are stale, so run it before pushing.
- **Refresh the progress page after each merge** (https://cmprssd.github.io/nfsh/, the
  `gh-pages` branch): after a release, `bench/history.sh` renders the history views with
  it (one Windows build, headless), and `bench/media.sh`, from the release's tag, its
  gallery and fly-through videos (about 25 minutes, headless); then
  `scripts/publish-progress.sh` from the updated main builds the page and force-pushes it
  to `gh-pages` as one commit. A new measurement in BASELINE.md or SIZES.md gets its line
  in `bench/results.tsv`, which the page charts. The page is monochrome (cmprssd's rule):
  only the renders and videos keep their colour.
