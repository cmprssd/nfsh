# What the store's bytes are

Measured on the owner's install with `nfsh sizes` and `nfsh sizes-trial` on 2026-11-06,
on the store made by nfsh 0.4.0, except [the store as built](#the-store-as-built), made by
nfsh 0.26.0 on 2026-11-07. Sizes are stored bytes unless they say otherwise. To
refresh, on the Windows side, which reads `D:` much faster:

```
nfsh sizes D:\nfsh-store       (reads all 65 GiB; about 16 minutes on 4 cores)
nfsh sizes-trial D:\nfsh-store (about 5 minutes)
```

## In short

- **The store fits a 25 GB Blu-ray with no loss of quality, and is built.** A BD-25 holds
  23.28 GiB. nfsh 0.26.0's store is 23.08 GiB: 22.92 GiB of packs and a 0.16 GiB index,
  every language kept, every texture's chunk split into a frame per tier. It was 28.07 GiB.
- **The rest of this page measured the 0.4.0 store** and says what a new store would hold;
  the gathered block fields and everything that loses quality are not built.
- **Two savings are the owner's call**: the chunks nothing names, and the other languages.
  Either one is enough, along with storing identical bytes once, though the margin is thin:
  0.27 GiB with the chunks dropped, before the disc's own file system.
  The owner has settled on one complete disc with the choice made at install, down to a
  bare minimum: [DECISIONS.md](DECISIONS.md) ("One disc, stripping down").
- **The install is built:** base plus English is 20.46 GiB with every tier, and 8.49 GiB at
  the bare minimum that can be cut today ([Installs](#installs)).
- **Blu-ray speed looks sufficient for the city's zones, but that isn't proven.** Reading
  2 km ahead takes 8.7 MiB/s at worst, under a 6x drive's slowest 10.3 MiB/s. Seeks, cars
  and missions are not in that number.

## The store as built

`nfsh import` from nfsh 0.26.0, 2026-11-07, 106 minutes, every entry checked byte for byte
against the install; `nfsh verify` then read it all back with no failures.

| | Size |
|---|---:|
| Packs (24) | 22.92 GiB |
| Index | 162.39 MiB |
| **Store** | **23.08 GiB** |
| A BD-25 holds | 23.28 GiB |

Not stored again, in the install's stored bytes:

| What | Entries | Size |
|---|---:|---:|
| Identical to another entry | 725,842 | 1.81 GiB |
| A chunk's end, read from the whole chunk | 66,474 | 1.05 GiB |
| Chunks nothing names | 15,908 | 2.86 GiB |

By part: the base and one part per language. The base plus any one language holds
everything they refer to (`nfsh verify` checks it): of 945,550 references between
partitions, 24 go from the base to a language, all from the per-language voice-over
loaders (`Audio/VO/LoadVoiceOver_XX`), which are only used with their language; none goes
from one language to another, and 5 go to partitions no table lists, as in the install.

| Part | Size |
|---|---:|
| Base | 19.90 GiB |
| de | 474.75 MiB |
| es | 451.07 MiB |
| fr | 425.27 MiB |
| en | 414.70 MiB |
| jp | 338.36 MiB |
| ru | 337.71 MiB |
| it | 332.25 MiB |
| pl | 317.51 MiB |

By tier, what the player can choose to copy or leave on the disc:

| Tier | Size |
|---|---:|
| Whole entries (ebx, res, audio, video, other chunks) | 8.84 GiB |
| Textures' top mip | 6.91 GiB |
| Second mip | 1.89 GiB |
| Third mip | 535.79 MiB |
| The other mips | 1.14 GiB |
| Mesh LOD 0 | 1.71 GiB |
| Mesh LOD 1 | 1.04 GiB |
| Mesh LOD 2 | 591.16 MiB |
| Mesh LOD 3 | 235.72 MiB |
| Mesh LOD 4 | 62.04 MiB |
| Mesh LOD 5 | 3.12 MiB |

The margin is 0.20 GiB before the disc's own file system. Voice-over in a folder that names
no language counts as English; the cinematics' audio, whose languages are not worked out,
stays in the base.

## Installs

`nfsh install` from nfsh 0.27.0, 2026-11-09, from the store above, with English as the one
language. `nfsh verify` read each back with no failures, counting the entries it leaves
out apart, and the history views render from both.

| File | Every tier | Bare minimum |
|---|---:|---:|
| `base.nfsh` (never cut) | 7.16 GiB | 7.16 GiB |
| `base.mip0.nfsh`, textures' top mip | 6.91 GiB | |
| `base.mip1.nfsh`, second mip | 1.89 GiB | |
| `base.mip2.nfsh`, third mip | 535.79 MiB | |
| `base.lod0.nfsh`, mesh LOD 0 | 1.63 GiB | |
| `base.lod1.nfsh`, mesh LOD 1 | 1.02 GiB | |
| `base.lod2.nfsh` | 577.92 MiB | 577.92 MiB |
| `base.lod3.nfsh` | 206.67 MiB | 206.67 MiB |
| `base.lod4.nfsh` | 6.79 MiB | 6.79 MiB |
| `en.nfsh` | 414.70 MiB | 414.70 MiB |
| **Install**, with index and placement | **20.46 GiB** | **8.49 GiB** |
| Copy time on the owner's PC, `D:` to `D:` | 13.5 min | 9.1 min |

- **Every tier, base plus English:** the speedway's five history views draw identically to
  the store's, pixel for pixel.
- **The bare minimum** (`--bare-minimum`: the top three mips and the meshes' LOD 0 and 1
  cut) is softer and coarser: the speedway's zone draws 1.2 million triangles instead of
  6.4, and its textures take 11.7 MiB of the GPU instead of 694. No texture or mesh is
  drawn grey or missing.
- **Both are larger than "A frame per tier" estimated:** 18.4 GiB for one language with
  everything, and 6.3 GiB after the same cuts (4 GiB with cutscene video, voices and music
  also off, which aren't tiers yet). The 2.06 GiB gap in the whole install carries through;
  where the estimate went wrong is not worked out. Mesh chunks that are some mesh's last
  LOD stay: the cut LOD tiers hold 2.65 GiB here, against 2.75 tagged in the store.
- **Mesh LOD 5's 3.12 MiB** are last LODs, so they are in `base.nfsh`.

## By kind

| Kind | Stored | Share |
|---|---:|---:|
| Textures (10,810 MiB of chunks, 1,121 MiB of bundles' copies of small mips) | 11.65 GiB | 41.7% |
| Audio (localized waves 4,305 MiB, other waves 1,653 MiB) | 5.82 GiB | 20.8% |
| Meshes (chunks 3,861 MiB, mesh set res 177 MiB) | 3.94 GiB | 14.1% |
| Chunks nothing names | 2.71 GiB | 9.7% |
| Video (`MovieTexture2Asset`) | 1.82 GiB | 6.5% |
| `OctaneAsset` chunks (what they hold is not worked out) | 0.45 GiB | 1.6% |
| Animation (`AssetBank` res) | 0.42 GiB | 1.5% |
| ebx | 0.32 GiB | 1.2% |
| Terrain tiles | 0.23 GiB | 0.8% |
| Everything else | 0.54 GiB | 1.9% |
| **All packs** | **27.92 GiB** | |

### Textures

| Format | Decompressed | Stored | | Largest side | Entries | Stored |
|---|---:|---:|---|---:|---:|---:|
| BC3 sRGB | 11,139 MiB | 2,234 MiB | | up to 256 | 40,079 | 275 MiB |
| BC5 | 3,140 MiB | 2,233 MiB | | 512 | 37,280 | 1,672 MiB |
| BC7 unorm | 2,780 MiB | 2,027 MiB | | 1024 | 27,590 | 2,494 MiB |
| BC3 unorm | 3,827 MiB | 1,524 MiB | | 2048 | 17,711 | 6,112 MiB |
| BC1 sRGB | 7,626 MiB | 1,502 MiB | | 4096 | 1,109 | 1,379 MiB |
| BC7 sRGB | 1,883 MiB | 1,050 MiB | | | | |
| BC1 unorm | 3,610 MiB | 940 MiB | | | | |
| BC6H | 690 MiB | 343 MiB | | | | |
| others | 263 MiB | 79 MiB | | | | |

BC5 (normal maps) and BC7 unorm barely compress, at about 70% of their size, BC7 sRGB
reaches 56%, and BC1 and BC3 sRGB reach 20%.

The largest users are the world's assets (`worldassets_roboto` 3,636 MiB,
`worldassets_hashtag` 838 MiB, `levels/mainlevel` 796 MiB), then characters (about 4.0 GiB
over top, hair, bottom, shoe, head and accessory).

### Meshes, by LOD

| LOD | Chunks | Decompressed | Stored |
|---:|---:|---:|---:|
| 0 | 33,270 | 5,281 MiB | 1,779 MiB |
| 1 | 29,329 | 3,103 MiB | 1,106 MiB |
| 2 | 28,847 | 1,697 MiB | 635 MiB |
| 3 | 21,597 | 693 MiB | 268 MiB |
| 4 and 5 | 17,170 | 183 MiB | 71 MiB |

### Voices

Localized waves, by the language folder in their names:

| Language | Stored | | Language | Stored |
|---|---:|---|---|---:|
| de | 474 MiB | | jp | 337 MiB |
| es | 450 MiB | | ru | 337 MiB |
| fr | 424 MiB | | it | 331 MiB |
| en, in `en/` | 199 MiB | | pl | 316 MiB |
| en, assumed, directly in `content/` | 217 MiB | | | |

Cinematics' audio is another 1,174 MiB, in assets of 8 chunks each.

## What can go without changing the look

Each row is measured after the ones above it, with no byte counted twice. All of them need
a new store.

| Saving | Packs after | Saved | How sure |
|---|---:|---:|---|
| The store now | 27.92 GiB | | |
| Identical content stored once, no copied chunk ends | 25.57 GiB | 2.35 GiB | Certain: compared byte for byte |
| Chunks nothing names | 22.85 GiB | 2.72 GiB | Strong evidence, not proof |
| Voice-over in folders of one language, English | 20.25 GiB | 2.60 GiB | Certain, if one language is all the owner wants |
| 7/8 of the cinematics' audio | 19.24 GiB | 1.01 GiB | Likely: which chunk is which language is not worked out |

Not in the tool's table, and not counted against the rows above:

- **7/8 of the movies' audio: 0.34 GiB.** Summing the WebM blocks by track, the 26 distinct
  movies hold 1.46 GiB of video and 0.40 GiB of audio, 16 of them with 8 audio tracks. Taking
  tracks out means rewriting the WebM files and their frame tables.
- **Gathering block fields: about 0.66 GiB** (the trial below), applied to today's sizes.

### Identical bytes

773,580 entries are copies of an earlier one, 2.07 GiB stored: chunks from their start
980 MiB, chunk ends 761 MiB, ebx 181 MiB (503,145 of the 725,119), mesh sets 117 MiB. All
were compared byte for byte, and none differed. Separately, 61,886 of 66,474 chunk ends that
bundles hold alone are exactly the end of a whole chunk also stored (1,071 MiB).

The index can point every copy at one location. Dropping chunk ends costs something: a
texture's small mips are what is read first, and they would sit at the end of a large zstd
frame. So a texture's chunk should be stored as two frames, the mips only the whole chunk
holds and the rest, which should cost little.

### Chunks nothing names

No ebx or res names these 15,965 entries (2,780 MiB):

| What | Entries | Stored |
|---|---:|---:|
| Block-sized data, many in exact mip chain sizes (5,592,432 bytes is 2048² at 1 byte a pixel, rounded up to 16) | 10,374 | 1,263 MiB |
| EA audio | 790 | 739 MiB |
| WebM video | 5 | 564 MiB |
| Other | 4,796 | 214 MiB |

Why they look unused:

- **Every one is in `Data/`; none is in `Patch/`.** A patched asset gets a new chunk and the
  old table still lists the old one, so leftovers from patching would look like this.
- **Nothing refers to them in any form we searched.** Every entry except texture pixels was
  searched for all 16,427 ids, in four byte orders and as hex text, and none was found. The
  same scan found all 4 known ids given to it as a control.

Why that isn't proof: the game might derive an id, read a table by position, or name them
in a form not searched. Running the original and watching which chunks it reads would
settle it. A store without them could also be tested by nfsh once it plays the game. The
install stays on the PC, so a store missing something can always be rebuilt from it.

### The sample of lossless recompression

Every 37th chunk or so at the store's zstd level, with each block's fields gathered: end
points together, indices together.

| Kind | Chunks sampled | Stored now | Gathered | Change |
|---|---:|---:|---:|---:|
| BC3 unorm | 88 | 40.7 MiB | 35.6 MiB | -12.6% |
| BC3 sRGB | 141 | 73.9 MiB | 66.6 MiB | -9.8% |
| BC1 sRGB | 224 | 31.8 MiB | 29.8 MiB | -6.2% |
| BC5 | 38 | 52.6 MiB | 49.5 MiB | -5.8% |
| BC1 unorm | 126 | 13.6 MiB | 13.0 MiB | -4.1% |
| BC4 | 23 | 0.4 MiB | 0.4 MiB | -1.8% |
| BC7, BC6H (no fields: their layout changes by block; unchanged) | 266 | 103.5 MiB | 103.5 MiB | 0% |
| Meshes, streams split by byte, indices as differences | 3,278 | 107.6 MiB | 116.6 MiB | +8.4% |

Applied to today's sizes by format, that is about 0.66 GiB. It would add one step when
reading, undoing the gathering. Meshes get worse in this simple form, so a mesh-specific
codec would need its own trial.

### A frame per tier

The same sample of textures, each chunk split into a frame per tier (top mip, second, third,
the rest), at the store's level. A texture always keeps its smallest mip, so one with few
mips has its smallest in the last tier and the tiers before it empty.

| Largest side | Chunks sampled | One frame | Top mip | Second | Third | Rest | Split | Change |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| up to 256 | 203 | 2.3 MiB | 1.5 MiB | 0.4 MiB | 0.1 MiB | 0.3 MiB | 2.4 MiB | +1.82% |
| up to 512 | 315 | 34.1 MiB | 20.2 MiB | 5.8 MiB | 1.7 MiB | 6.7 MiB | 34.3 MiB | +0.73% |
| up to 1024 | 196 | 59.4 MiB | 38.1 MiB | 10.4 MiB | 2.9 MiB | 8.4 MiB | 59.9 MiB | +0.79% |
| up to 2048 | 166 | 174.6 MiB | 122.7 MiB | 33.9 MiB | 9.4 MiB | 9.8 MiB | 175.8 MiB | +0.68% |
| up to 4096 | 26 | 46.1 MiB | 24.6 MiB | 6.6 MiB | 1.8 MiB | 13.1 MiB | 46.2 MiB | +0.23% |
| All | 906 | 316.5 MiB | 207.1 MiB | 57.2 MiB | 15.9 MiB | 38.3 MiB | 318.6 MiB | +0.65% |

- **Splitting costs little:** +0.65%, about 70 MiB on the 10,810 MiB of textures' chunks.
- **Tiers' shares of the stored bytes:** top 65.0%, second 18.0%, third 5.0%, rest 12.0%.
  Applied to those chunks (10.56 GiB), about 6.9, 1.9, 0.5 and 1.3 GiB. This is a sample of
  906 chunks, so these are estimates too, but better ones than a share of raw bytes.
- **Cutting the top mip saves less than the table below's estimate,** because a texture of
  one mip can't lose it: non-power-of-two sizes such as 2560 have only one.

## What would lose quality

Not needed to fit a BD-25, so nothing was compared. Each would be the owner's A/B choice
with a measured size and a comparison.

| Saving | At most | |
|---|---:|---|
| The largest mip of the 1,109 textures of 4096 | about 1.03 GiB | An estimate, by the mip's share of raw bytes |
| Re-encoding video | part of 1.46 GiB | Generation loss |
| Re-encoding audio | part of 5.8 GiB | Already compressed, so generation loss |
| The largest mip of every texture | about 6.9 GiB | Visibly softer close up; the sample above, keeping every texture's smallest mip |

## Streaming from a disc

### Zones

| Zones | Count | Median | Largest | All of them, shared bytes once |
|---|---:|---:|---:|---:|
| Art, with one time of day's light | 232 | 11.8 MiB | 153.2 MiB | 1,612 MiB without the light |
| Drivable surfaces, with light | 66 | 10.7 MiB | 59.1 MiB | 854 MiB without the light |

The median art zone's nearest neighbour is 493 m away, a drivable zone's 859 m. 41 zones
place no mesh and hold 0.6 MiB together, so they have no position and aren't driven through.

### Always resident

Everything the level links outside the zones and their light, as an upper bound, because
some of it (`dealerships_spatial`, `garage`) surely loads only when needed. Which of it is
truly always in memory is not worked out.

| | Stored | Decompressed |
|---|---:|---:|
| All mips | 5,271 MiB | 11,261 MiB |
| Without the mips only a whole chunk holds | 2,335 MiB | not counted |
| `globals`, in addition | 57 MiB | 147 MiB |

The largest parts, stored with all mips and without those mips: `region_common` 1,261 and
155 MiB, `mainlevel` 729 and 443, `drivable_common` 709 and 12, `worldvista_worldart` 376
and 52, `music` 374 and 374, `region_city` 330 and 79.

So the shared textures have to stream by distance: 11.3 GiB decompressed can't stay in
memory on an 8 GiB PC. Reading the 2.3 GiB at the start takes about 4 minutes at the slowest
speed of a 6x drive, which argues for keeping it in a local cache.

### MiB per km and read rates

Straight lines across the city at about 400 km/h, 111 m/s: 21 lines, along both axes and
the diagonals. A zone is read when it comes within the radius, unless a zone in range or the
resident set already holds its bytes. "Without streamed" counts a texture's stored bytes
without the mips only the whole chunk holds, in proportion to their raw size.

| Radius | Mips | Mean MiB/km | Worst line | Worst 1 km | At top speed: mean MiB/s | Worst 1 km MiB/s |
|---:|---|---:|---:|---:|---:|---:|
| 500 m | all | 13 | 38 | 181 | 1.5 | 20.1 |
| 500 m | without streamed | 6 | 16 | 76 | 0.6 | 8.5 |
| 1000 m | all | 33 | 87 | 250 | 3.6 | 27.7 |
| 1000 m | without streamed | 14 | 35 | 110 | 1.5 | 12.2 |
| 2000 m | all | 64 | 120 | 355 | 7.2 | 39.4 |
| 2000 m | without streamed | 28 | 54 | 165 | 3.1 | 18.3 |

What matters more is the steady rate that never falls behind, because the game can start
reading before a zone is due:

| Everything in by | Read ahead by | All mips | Without streamed |
|---:|---:|---:|---:|
| 500 m | 500 m | 17.6 MiB/s | 8.3 MiB/s |
| 500 m | 1 km | 11.6 MiB/s | 5.5 MiB/s |
| 500 m | 2 km | 8.7 MiB/s | 3.9 MiB/s |
| 1000 m | 1 km | 15.0 MiB/s | 7.0 MiB/s |
| 1000 m | 2 km | 11.6 MiB/s | 5.3 MiB/s |
| 1500 m | 2 km | 15.0 MiB/s | 6.5 MiB/s |

For comparison, from the discs' and drives' usual specifications, not measured: Blu-ray 1x
is 4.3 MiB/s, and a 6x drive reads between about 10.3 MiB/s at the disc's centre and 25.7 at
its edge, with seeks of about 0.2 s. 2 to 4 zones a km at top speed is up to about 0.4 seeks
a second, so up to about 9% of the time.

**What this says:** a 6x drive can keep up with the city's zones with a read-ahead of about
2 km, and the slowest cases leave little margin unless the highest mips of far zones wait
until they are near. A 1x drive can't, except in the gentlest rows.

**What it leaves out:**

- The game's own radii, which are in a `GridResource` not read yet.
- Real roads and speeds: a straight line at top speed through every building is harsher than
  any drive, but a road can also turn back towards data already dropped.
- Cars, traffic, characters and missions, which load on top.
- The streamed mips of the shared textures. The drives treat the resident set as already
  loaded, and its 2.9 GiB of mips would have to come from a cache or add to these rates.
- Seeks and the disc's layout.

So the org's check still has to be built: a recorded path played at the disc's throttled
speed and seek times, with no frame over budget. Until it passes, playing from the disc isn't
offered.
