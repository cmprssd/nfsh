# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

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
