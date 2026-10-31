# How nfsh works

A guided tour of the project: what it builds, how the code is laid out, how each step is
checked against the original game's files, and how the work itself is run.
[DECISIONS.md](DECISIONS.md) holds every choice with its reason,
[FORMATS.md](FORMATS.md) the file formats as we read them, and
[INVENTORY.md](INVENTORY.md) what the owner's install holds. The same numbers, as
charts, are on the [progress page](#progress-page).

<!-- progress:asof -->
Numbers in the generated parts below are as of 2026-10-31 (11 merged PRs), counted by
`scripts/progress.py`.
<!-- /progress -->
The hand-written parts give their own dates.

## What it is

nfsh is a new engine for **Need for Speed: Heat** (Ghost Games / EA, 2019), written in
Rust with Vulkan 1.3 (`ash`). It reads the player's own Steam install (`Data/` and
`Patch/`) and aims to show the same game with the same look, while taking much less disk
space, running much faster and never crashing.

It is written **clean-room**. Heat's code was never released, so unlike swjkja (whose
reference is the original's published source) there is no code to read: file formats
and behaviour are worked out from the files themselves and from watching the game run.
The repository is public and holds only our own descriptions and code; no game data, no
EA code, nothing decompiled.

## The idea in one picture

Every reader is proven against the whole install, not a sample: a test opens the owner's
copy and reads every file of its kind. What is learned goes into FORMATS.md, and the
inventory turns it into numbers that decide what to build next.

```mermaid
flowchart LR
    data[("The owner's install<br/>Data/ + Patch/<br/>.toc, .sb, .cas")]
    look["Read the bytes<br/>(hex, counts, patterns)"]
    fmt["FORMATS.md<br/>our description"]
    rd["crates/frostbite<br/>bounds-checked readers,<br/>no panics"]
    test{"every retail file<br/>reads, totals<br/>add up?"}
    inv["nfsh inventory<br/>INVENTORY.md"]
    next["next step:<br/>recompress, viewer"]
    data --> look --> fmt --> rd --> test
    data --> test
    test -- no --> look
    test -- yes --> inv --> next
```

Once something renders, the loop gains swjkja's second half: the original game and nfsh
draw the same camera path on the same PC, and screenshots and frame times are compared
(`docs/BASELINE.md`, not started yet).

## Architecture

<!-- progress:code -->
2 crates in one Cargo workspace, 7,636 lines of Rust (sources and tests), 64 tests.
<!-- /progress -->

```mermaid
flowchart BT
    fb["<b>frostbite</b><br/>layout.toc, .toc, .sb,<br/>DbObject, Patch over Data,<br/>cas blocks, nfsh's store,<br/>ebx partitions, level graph,<br/>mesh geometry"]
    oo["oo2core_6_win64.dll<br/>(the install's own,<br/>loaded at run time)"]
    app["<b>nfsh</b><br/>the commands:<br/>nfsh inventory, nfsh verify,<br/>nfsh import, nfsh reindex,<br/>nfsh ebx, nfsh level,<br/>nfsh mesh"]
    oo --> fb
    fb --> app
```

<!-- progress:crates -->
| Crate | Rust lines | Tests | What it does |
|------|------:|------:|------|
| `frostbite` | 6,262 | 54 | readers for the install's tables: `layout.toc`, superbundle `.toc` and `.sb`, DbObject trees, cas references; resolves `Patch/` over `Data/`; decompresses entries (Oodle through the install's own library); writes, checks and reads nfsh's store, with every ebx's partition GUID; reads ebx partitions (types, instances, fields, arrays, references); walks a level's sub-levels and what each zone places (meshes in world space); reads mesh sets and decodes their geometry (LODs, sections, positions, texture coordinates, triangles) |
| `nfsh` | 1,374 | 10 | the `nfsh` command: `inventory` lists what the install holds, `verify` decompresses every entry and checks its size, `import` copies the live data into nfsh's store, `reindex` lists its partition GUIDs, `ebx` prints a partition as text, `level` prints Palm City's bundles or one zone's placements, `mesh` prints a mesh's LODs and writes one as an .obj |
<!-- /progress -->

Every reader checks bounds and returns an error on malformed input, so a damaged or
unexpected file is reported, never a crash. The tests that read the real install and
the store run on the Windows side, where local-ci passes `NFSH_INSTALL` and `NFSH_STORE`;
the store tests read every value of all 725,119 ebx partitions and walk all 232 zones of
Palm City (259,999 placed meshes, every reference resolved), then read the geometry of
every mesh placed there (5,475 meshes, 48.8 million triangles in all their LODs).

## The process

The owner steers; Claude (Claude Code) does the engineering, one task per session.

```mermaid
flowchart TD
    h["Saved handoff<br/>~/.claude/handoffs/cmprssd/nfsh.md"] --> s["New session:<br/>goal shown, owner confirms"]
    s --> q["Two-option questions<br/>(only what changes the result)"]
    q --> read["Read the files and<br/>the docs so far"]
    read --> branch["Branch feat/… or fix/…"]
    branch --> build["Implement + unit tests<br/>(literal expected values)"]
    build --> verify["Check against the<br/>owner's whole install"]
    verify --> docs["FORMATS.md: formats<br/>DECISIONS.md: choices<br/>INVENTORY.md: numbers"]
    docs --> push["local-ci push:<br/>fmt, overview numbers, committed tree builds,<br/>Linux + Windows clippy and tests"]
    push --> ok{"local-ci green<br/>on the head commit?"}
    ok -- no --> build
    ok -- yes --> merge["Self-merge (squash)"]
    merge --> pub["Publish the progress page"]
    pub --> next["Write the next handoff"] --> h
```

Rules that shape it:

- **Two-option questions.** Before building, Claude asks the owner only what changes the
  result, with a recommendation. Choices that come up mid-build are made by Claude,
  marked *decided by Claude, pending review* in DECISIONS.md, and listed in the PR.
  <!-- progress:process -->
  So far: 17 decision entries, 8 chosen by the owner, 8 made by Claude for review.
  <!-- /progress -->
- **$0 development.** No GitHub Actions: `.local-ci` runs on this machine (format, shell
  and Python syntax, this overview's numbers, a build of exactly the committed tree, then
  clippy and tests on Linux/WSL and on Windows/MSVC in parallel) and posts a `local-ci`
  status to the PR. A PR merges only when that status is green on its head commit.
- **No game data in the repository.** A whitelist `.gitignore` admits only the kinds of
  files the project writes, so a `.toc` or `.cas` cannot be committed by accident.
- **The numbers are counted, not typed.** The generated parts of this page come from
  `scripts/progress.py`, and local-ci fails when they are stale.
- **Short sessions, saved handoffs.** Each session ends with a self-contained prompt for
  the next one, so context is rebuilt from the docs, not from a long chat.

## Statistics

### History

<!-- progress:history -->
12 commits, 11 merged PRs from 2026-10-30 to 2026-10-31, on 2 working days: 8 feat, 2
docs, 1 ci. About 10,780 lines added and 286 removed; 40 tracked files.

```mermaid
xychart-beta
    title "PRs merged per working day"
    x-axis ["Oct 30", "Oct 31"]
    y-axis "PRs" 0 --> 12
    bar [1, 10]
```
<!-- /progress -->

### The install, read

<!-- progress:install -->
Measured by `nfsh inventory` on 2026-10-30 ([INVENTORY.md](INVENTORY.md)):

| What | Count | Size |
|------|------:|------:|
| Files in the install | 392 | 34.45 GiB |
| cas archives (stored asset bytes) | | 33.15 GiB |
| ... still read after the patch | | 29.28 GiB (88.3%) |
| ... replaced by the patch, dead | | 3.87 GiB |
| Superbundles, bundles | 22, 20,739 | |
| Ebx, res, chunk references | 725,215, 260,398, 411,889 | |
| ... distinct | 221,745, 108,281, 242,640 | |
<!-- /progress -->

What it means: almost all of the 34 GiB is cas archives, so "smaller" is decided by how
the archived data is stored, and dropping the dead data alone is a free first saving
that nfsh's own format has to beat. Palm City is
`Win32/levels/mainlevel/mainlevel`, confirmed by reading its root partition: 232
streaming zones, each with four lighting variants (midday, overcast, sunset,
tealnight). Details in [INVENTORY.md](INVENTORY.md) and [FORMATS.md](FORMATS.md).

### The store

`nfsh import` writes the 1,317,687 distinct entries the patched tables use into nfsh's
store, each recompressed from Heat's Oodle blocks into one zstd frame (level 19, the
ebx with a dictionary), and checks every entry against the install. Measured on the
owner's install, 2026-10-31:

| Store | Packs | Index | Total | Import |
|-------|------:|------:|------:|-------:|
| cas archives (the install) | | | 33.15 GiB | |
| 0.3.0: live entries copied, Oodle blocks kept | 30, 29.35 GiB | 148 MiB | 29.49 GiB | 27 min |
| **0.4.0: recompressed with zstd** | 29, 27.92 GiB | 148 MiB | **28.07 GiB** | 117 min |

zstd ends up 4.8% smaller than Oodle (the sample predicted about the same size) and
makes the store readable anywhere: `nfsh verify` decodes all 65.4 GiB from it in 5
minutes with no game files. How the level was chosen, with the sample measured, is in
[DECISIONS.md](DECISIONS.md). The 0.3.0 packs were about 70 MiB above the 29.28 GiB of
live bytes because some distinct entries overlap in the archives; each is still stored
whole.

## Where it is going

From [BRIEF.md](BRIEF.md) and [DECISIONS.md](DECISIONS.md) (2026-10-31):

| # | Milestone | State |
|---|------|------|
| 1 | Data inventory: read the install's tables | done (0.1.0) |
| 2 | Read entry contents: decompress the cas blocks | done (0.2.0) |
| 3 | Import step: nfsh's own compact copy of the data | done (0.3.0: dead data dropped; 0.4.0: zstd) |
| 4 | Palm City viewer: fly-through, frame time, day/night | started: ebx reader (0.5.0); level graph, every zone's placed meshes (0.6.0); mesh geometry (0.7.0) |
| 5 | Baseline against the original: same camera path, same PC, screenshots | not started |
| 6 | One drivable car, then races, police, campaign | later |

## Progress page

The [progress page](https://cmprssd.github.io/nfsh/) tells the same story as charts: PRs
per day, Rust lines and tests after every PR, the crates, and what the install holds. It
is served from the `gh-pages` branch, which holds the page alone, so main stays free of
anything generated. `scripts/publish-progress.sh` builds it from
`scripts/progress.py --json` and `scripts/progress_page.html` and publishes it after
each merge (see CLAUDE.md). It shows no game imagery yet; when the viewer renders, the
before-and-after comparisons arrive as in swjkja, still only on `gh-pages`.

## Reproducing the numbers

    cargo test --workspace                                        # Linux
    NFSH_INSTALL='D:\…\Need for Speed Heat' NFSH_STORE='D:\nfsh-store' \
        WSLENV=NFSH_INSTALL:NFSH_STORE \
        scripts/cargo-msvc.sh test --workspace                    # Windows, install and store
    cargo run --release -- inventory "<install>" > out.md         # INVENTORY.md
    python3 scripts/progress.py                                   # this page's numbers
