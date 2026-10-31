# nfsh

A new engine for **Need for Speed: Heat**, written in Rust. The same game, looking the
same, but much smaller on disk, much faster and more reliable. It is written from
scratch: Heat's engine and code were never released.

It ships no game data. It reads the assets from your own copy of the game.

Status: early. nfsh can read the install's tables and list everything in it; nothing
renders yet. The goal is in [docs/BRIEF.md](docs/BRIEF.md).

## Use

```
cargo run --release -- inventory "<path to Need for Speed Heat>"
```

prints, as Markdown, what the install holds: files by kind, how much of each archive is
still used after the patch, every superbundle's bundles and entries, and resource types
with their sizes. The owner's install is recorded in
[docs/INVENTORY.md](docs/INVENTORY.md); the formats are in
[docs/FORMATS.md](docs/FORMATS.md).

## License

GPLv3 (see [LICENSE](LICENSE)). Need for Speed is a trademark of Electronic Arts; this
project is not affiliated with or endorsed by Electronic Arts or Ghost Games.
