# Code map

Where things live, one line each. Read this before searching the tree. Keep it under ~150
lines: a map, not documentation (the tour is `OVERVIEW.md`). A PR that adds, moves or
renames a module updates it.

Workspace (`Cargo.toml`, edition 2024): three crates, no `[[bin]]` or `[[bench]]` entries.
Tests are inline `#[cfg(test)]` modules plus `crates/render/tests/`.

- `crates/frostbite`: lib `frostbite`, readers for Heat's formats and nfsh's store; no Vulkan.
- `crates/render`: lib `nfsh-render`, Vulkan 1.3 via `ash`.
- `crates/nfsh`: bin `nfsh`, the commands; depends on both.

## Entry points (`crates/nfsh/src`)

- `main.rs`: command dispatch (inventory, verify, import, install, reindex, ebx, level, mesh,
  texture, sizes, sizes-trial, view, camera, replay).
- `inventory.rs`: `nfsh inventory`, counts and bytes per superbundle and resource type.
- `verify.rs`: `nfsh verify`, decompress every entry of an install or store, check sizes.
- `import.rs`: `nfsh import`, recompress the install's data into the store.
- `install.rs`: `nfsh install`, copy the store into an install of kept languages and tiers.
- `ebx.rs`: `nfsh ebx`, one ebx partition as text.
- `level.rs`: `nfsh level`, Palm City bundles or one zone's placements as text.
- `mesh.rs`: `nfsh mesh`, mesh LODs and sections as text, optional .obj.
- `texture.rs`: `nfsh texture`, a texture's header as text, pixels to .dds.
- `sizes.rs`: `nfsh sizes` and `sizes-trial`, what the store's bytes are, streaming cost.
- `view.rs`: `nfsh view`, windowed fly-through of one zone, or a screenshot.
- `fly.rs`: free-flying camera controller for `view`.
- `camera.rs`: `nfsh camera`, reads the original game's camera from its memory (read-only).
- `path.rs`: camera path files (record, play) and frame-time statistics.
- `replay.rs`: `nfsh replay`, re-reads an `NFSH_READ_TRACE` trace past the file cache, timed.

## Heat's formats (`crates/frostbite/src`)

Every reader is bounds-checked and returns `Error`; none panics. Formats are described in
`docs/FORMATS.md`.

- `lib.rs`: crate root, `Error`, module list.
- `reader.rs`: bounds-checked byte cursor.
- `toc.rs`: superbundle `.toc` and the signed header shared with layout.toc and initfs.
- `dbobject.rs`: Frostbite's tagged binary tree (layout.toc, chunkmanifest).
- `sb.rs`: bundles inside a superbundle `.sb`, location records.
- `cas.rs`: `cas_NN.cas` entries, chains of blocks with 8-byte BE headers.
- `oodle.rs`: Oodle decompression through the install's own oo2core DLL (Windows only).
- `install.rs`: an installed game, `Data/` with `Patch/` over it.
- `ebx.rs`: ebx partitions, types, instances, fields, arrays, references.
- `mesh.rs`: RigidMesh/CompositeMesh assets, MeshSetResource, LOD chunks.
- `texture.rs`: texture res header, block-compressed mips and layers, material references.
- `terrain.rs`: terrain quadtree tiles, layer records and their cells, outlines, surface map, heightfields, height bounds, coarse heights, colour, finer chunks,
  ground mesh.
- `layers.rs`: terrain layer tree, per-layer textures and shader bindings.
- `shaderdb.rs`: level shader database, parameter blocks, permutations.
- `dxbc.rs`: DXBC (SM5) bytecode, read only for terrain texture sample scales.
- `lighting.rs`: time-of-day sun, sky and exposure; sky panorama and gradient.
- `level.rs`: level graph, sub-level bundles, zones, placed meshes in world space.

## nfsh's store (`crates/frostbite/src`)

- `store.rs`: packs (`pack_NN.nfsh`) and `index.nfsh`; write, check, read.
- `frames.rs`: entry encoding, zstd frames, per-ebx dictionary, size and checksum.
- `plan.rs`: what the store keeps; dedup, tier split of texture chunks, part tagging.
- `copy.rs`: copy store frames into an install by part and tier (`base.nfsh`, `en.nfsh`, ...).

## Renderer (`crates/render`)

- `src/lib.rs`: crate root, exports.
- `build.rs`: compiles `shaders/*.wgsl` to SPIR-V with naga at build time.
- `shaders/zone.wgsl`: zone meshes; texture, normal map, sun and sky lighting, tonemap.
- `shaders/sky.wgsl`: panorama over gradient, sun disc.
- `src/gpu.rs`: instance, device, allocator.
- `src/resource.rs`: buffers, images, offscreen target (RAII).
- `src/swapchain.rs`: window swapchain.
- `src/frame.rs`: `FrameLoop`, frames in flight, GPU timing, presenting.
- `src/camera.rs`: Frostbite world space to Vulkan clip, reversed infinite depth.
- `src/cull.rs`: placement boxes, view culling, LOD and screen-area selection.
- `src/scene.rs`: CPU-side zone packing; vertex, index and instance buffers, draws per section.
- `src/zone.rs`: draws a `Scene`; opaque, ground, cutout, decals.
- `src/sky.rs`: sky upload and the one-triangle draw.
- `tests/draw.rs`, `tests/sky.rs`: real-device draws (lavapipe or GPU) with pixel readback.

## Scripts and bench

- `.local-ci`: the checks every PR passes (fmt, syntax, results.tsv, overview numbers,
  clippy and tests on Linux and Windows, `disk.sh prune`).
- `scripts/cargo-msvc.sh`: Windows-host cargo from WSL, for real-GPU builds and tests.
- `scripts/disk.sh`: build-folder disk use against budgets.
- `scripts/progress.py`: writes or `--check`s the generated blocks of `docs/OVERVIEW.md`.
- `scripts/progress_page.py`, `progress_page.html`: build the progress page.
- `scripts/publish-progress.sh`: publish the page to `gh-pages` as one commit.
- `bench/baseline.sh`: Windows build, fly a camera path RUNS times, median frame-time row.
- `bench/history.sh`: history views with each release since 0.10.0.
- `bench/media.sh`: gallery and film media, headless, for the newest release.
- `bench/paths/*.path`: recorded camera paths (speedway = zone 495, garage-drive).
- `bench/history-views.txt`, `gallery.txt`, `films.txt`: view and shot lists for the page.
- `bench/results.tsv`: measurements (who, release, metric, value, source), charted on the page.

## Docs

- `docs/OVERVIEW.md`: the tour; architecture, process, generated numbers.
- `docs/FORMATS.md`: our description of Heat's formats; the readers follow it.
- `docs/DECISIONS.md`: dated decisions with reasons.
- `docs/INVENTORY.md`: what the install holds (`nfsh inventory`).
- `docs/SIZES.md`: what the store's bytes are (`nfsh sizes`).
- `docs/BASELINE.md`: frame times and loads against the original.
- `docs/BRIEF.md`: the spec. `CHANGELOG.md`: release notes.
