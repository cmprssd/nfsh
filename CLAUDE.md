# CLAUDE.md

nfsh is a new engine for **Need for Speed: Heat** in Rust (Vulkan 1.3 via `ash`), written
clean-room: Heat's code was never released, so file formats and behaviour are reverse
engineered. Same game, same look, much smaller on disk, much faster, more reliable; the
same nature as `cmprssd/swjkja` (`~/projects/cmprssd/swjkja`), whose tooling is copied
and adapted here. It ships no game data: it reads the owner's install at
`D:\SteamLibrary\steamapps\common\Need for Speed Heat`
(`/mnt/d/SteamLibrary/steamapps/common/Need for Speed Heat` from WSL).

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

## Rules

- **The repo is public.** Never commit game data, decompiled or disassembled EA code, or
  anything that bypasses the executable's protection. Only our own descriptions of
  formats and behaviour go in.
- Self-merge once `local-ci` passes (workspace default).
- **The numbers are counted, not typed.** `docs/OVERVIEW.md`'s `<!-- progress:… -->`
  blocks come from `python3 scripts/progress.py` (git, the tree, `docs/INVENTORY.md`);
  `.local-ci` fails when they are stale, so run it before pushing.
- **Refresh the progress page after each merge** (https://cmprssd.github.io/nfsh/, the
  `gh-pages` branch): `scripts/publish-progress.sh` from the updated main builds it and
  force-pushes it to `gh-pages` as one commit.
