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
  everything that loses quality is not built.
- **Two savings are the owner's call**: the chunks nothing names, and the other languages.
  Either one is enough, along with storing identical bytes once, though the margin is thin:
  0.27 GiB with the chunks dropped, before the disc's own file system.
  The owner has settled on one complete disc with the choice made at install, down to a
  bare minimum: [DECISIONS.md](DECISIONS.md) ("One disc, stripping down").
- **The install is built:** base plus English is 18.89 GiB with every tier, and 4.91 GiB at
  the bare minimum ([Installs](#installs)), from nfsh 0.30.0's store with gathered block
  fields. That is 0.49 and 1.06 GiB over the estimates, which counted the movies' other
  audio tracks, not built yet, and the gathered block fields at 0.66 GiB, where they save
  0.58 ([Against the estimate](#against-the-estimate)).
- **Blu-ray speed looks sufficient for the city's zones, but that isn't proven.** Reading
  2 km ahead takes 8.7 MiB/s at worst, under a 6x drive's slowest 10.3 MiB/s. Seeks, cars
  and missions are not in that number.

## The store as built

`nfsh import` from nfsh 0.26.0, 2026-11-07, 106 minutes (158 minutes for nfsh 0.29.0's on
2026-11-10, the same sizes but for the parts below), every entry checked byte for byte
against the install; `nfsh verify` then read it all back with no failures. nfsh 0.30.0's,
2026-11-11, about 130 minutes, stores textures' blocks with their fields gathered, checked
byte for byte by the import the same way (`nfsh verify` not run on it).

| | 0.26.0 | 0.30.0 |
|---|---:|---:|
| Packs | 22.92 GiB (24) | 22.34 GiB (23) |
| Index | 162.39 MiB | 162.39 MiB |
| **Store** | **23.08 GiB** | **22.50 GiB** |
| A BD-25 holds | 23.28 GiB | 23.28 GiB |

Not stored again, in the install's stored bytes:

| What | Entries | Size |
|---|---:|---:|
| Identical to another entry | 725,842 | 1.81 GiB |
| A chunk's end, read from the whole chunk | 66,474 | 1.05 GiB |
| Chunks nothing names | 15,908 | 2.86 GiB |

By part: the base, one part per language, and from nfsh 0.29.0 `others`, the cinematics'
lines in the languages with no `loc` table in the install (all but English here,
[FORMATS.md](FORMATS.md), "Cinematic dialogue"). The base plus any one language (and
`others`, for a language without a table) holds everything they refer to (`nfsh verify`
checks it): of 945,550 references between partitions, 24 go from the base to a language,
all from the per-language voice-over loaders (`Audio/VO/LoadVoiceOver_XX`), which are only
used with their language; none goes from one language to another, and 5 go to partitions
no table lists, as in the install. The 70 cinematic waves and their res name their 8
chunks each in the language parts: 1,120 names, which the check allows.

| Part | 0.26.0 to 0.28.0 | From 0.29.0 |
|---|---:|---:|
| Base | 19.90 GiB | 18.75 GiB |
| others | | 1,021.88 MiB |
| en | 414.70 MiB | 567.08 MiB |
| de | 474.75 MiB | |
| es | 451.07 MiB | |
| fr | 425.27 MiB | |
| jp | 338.36 MiB | |
| ru | 337.71 MiB | |
| it | 332.25 MiB | |
| pl | 317.51 MiB | |

The other languages' parts are the same in both.

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
no language counts as English; the cinematics' audio stays in the base in 0.26.0 to 0.28.0,
and from 0.29.0 is in `en` and `others`.

## Installs

`nfsh install` from the store above, with English as the one language: every tier from
nfsh 0.29.0's store (2026-11-10), the bare minimum from 0.28.0's. `nfsh verify` read each
back with no failures, counting the entries it leaves out apart, and the history views
render from both (0.28.0's installs). The bare minimum is not measured again: it cuts all
of the voices, wherever they are, so 0.29.0 leaves it as it was.

| File | Every tier | Bare minimum |
|---|---:|---:|
| `base.nfsh` (never cut) | 3.97 GiB | 3.97 GiB |
| `base.mip0.nfsh`, textures' top mip | 6.51 GiB | |
| `base.mip1.nfsh`, second mip | 1.80 GiB | |
| `base.mip2.nfsh`, third mip | 512.97 MiB | |
| `base.lod0.nfsh`, mesh LOD 0 | 1.63 GiB | |
| `base.lod1.nfsh`, mesh LOD 1 | 1.02 GiB | |
| `base.lod2.nfsh` | 577.92 MiB | 577.92 MiB |
| `base.lod3.nfsh` | 206.67 MiB | 206.67 MiB |
| `base.lod4.nfsh` | 6.79 MiB | 6.79 MiB |
| `base.video.nfsh`, cutscene video | 1.56 GiB | |
| `base.voices.nfsh`, voices in the base (the cinematics' audio until 0.28.0, 1.15 GiB) | 4.50 MiB | |
| `base.music.nfsh` | 414.07 MiB | |
| `en.nfsh` | 1.15 MiB | 1.15 MiB |
| `en.voices.nfsh`, the spoken lines and the cinematics' English | 565.92 MiB | |
| **Install**, with index and placement | **18.89 GiB** | **4.91 GiB** |
| Copy time on the owner's PC, `D:` to `D:` | 16.2 min | 9.2 min |

- **Gathered block fields (0.30.0) save 0.58 GiB,** 622,723,049 bytes, all in the textures'
  tiers: top mip -5.77%, second -5.03%, third -4.26%, the rest (in `base.nfsh`) -5.62%. The
  trial's sample had estimated 0.66. The four texture rows above and the totals are
  0.29.0's installs less each tier's bytes in the new store's index (`D:\nfsh-store-gathered`),
  not a new copy: nothing else in base or English changed, byte for byte. The import checked
  every entry back against the install.
- **Every tier, base plus English, in 0.29.0:** 19.47 GiB, 0.99 GiB less than 0.28.0's 20.46: the
  cinematics' other 7 languages (1,021.88 MiB) are in `others`, which English doesn't need.
  In 0.28.0 the speedway's five history views drew identically to the store's, pixel for
  pixel; 0.29.0 moves only audio.
- **The bare minimum** (`--bare-minimum`: the top three mips, the meshes' LOD 0 and 1,
  cutscene video, voices and music cut) was 4.97 GiB in 0.28.0, against 8.49 in 0.27.0 and the 3.8
  "A frame per tier" estimated. Its views are softer and coarser: the speedway's zone
  draws 1.2 million triangles instead of 6.4, and its textures take 11.7 MiB of the GPU
  instead of 694. No texture or mesh is drawn grey or missing.
- **Video, voices and music come to 3.52 GiB**, against the estimate's 2.5 (from 6.3 to
  3.8). The estimate had counted only an eighth of the cinematics' audio as English, so
  the voices cut takes the other 7/8 too; the bare minimum ends 1.17 GiB over the estimate,
  where the cuts before these ended 2.19 over.
- **Both are larger than "A frame per tier" estimated** (18.4 GiB with everything, 3.8 at
  the bare minimum). The estimate took off savings nothing has built yet; see
  [Against the estimate](#against-the-estimate). Mesh chunks that are some mesh's last
  LOD stay: the cut LOD tiers hold 2.65 GiB here, against 2.75 tagged in the store.
- **What stays of the media:** the UI's movies (0.26 GiB), the crowds' localized walla
  (44 MiB) and the cinematics' other sounds (0.27 GiB) are in `base.nfsh`, as are
  waves whose chunks something else also names ([DECISIONS.md](DECISIONS.md), "Cutscene
  video, voices and music as install tiers").
- **Mesh LOD 5's 3.12 MiB** are last LODs, so they are in `base.nfsh`.

### Against the estimate

The 18.4 GiB came from "What can go without changing the look" (on the 0.4.0 store):
19.24 GiB of packs once 7/8 of the cinematics' audio goes, plus a 0.16 GiB index, makes
19.40. Taking off the movies' other audio tracks and the gathered block fields then gives
18.40. The cinematics are built from nfsh 0.29.0 and the gathered fields from 0.30.0; the
movies' tracks are not. The install's own files account for the whole gap:

| With everything, base plus English | GiB |
|---|---:|
| Estimate | 18.40 |
| Gathered block fields: built in 0.30.0, saving 0.58 of the estimate's 0.66 | +0.08 |
| Movies' other audio tracks: need a WebM rewrite, not built; in `base.video.nfsh` | +0.34 |
| 7/8 of the cinematics' audio: built in 0.29.0, the other 7 languages' 1,021.88 MiB left out in `others`, against the estimate's 1.01 GiB (until 0.28.0, `base.voices.nfsh` kept all of it, +1.01) | +0.01 |
| A frame per tier: the trial's +0.65% on 10.56 GiB of textures' chunks | +0.07 |
| Sum | 18.90 |
| **Install, measured** | **18.89** |

The packs alone agree too, as 0.29.0 measured them: without the movies' tracks and the
gathered fields, the estimate had 19.24 GiB of packs, and that install's packs are 19.31:
0.07 more, which is the frame per tier's cost and the cinematics' 0.01.
The index is the store's whole index (0.16 GiB, every language's entries). The estimate
counted the same index, so it isn't part of the gap. A one-language index would be a
little smaller; that is not measured.

| Bare minimum | GiB |
|---|---:|
| Estimate | 3.85 |
| Gathered block fields: the estimate took off 0.66, the bare minimum keeps only the rest tier's 0.06 of the 0.58 | +0.60 |
| Meshes: chunks that are some mesh's last LOD stay; cut 2.65, not 2.82 | +0.17 |
| Video, voices and music: cut 3.52, not 3.78; mostly the UI's movies (0.26), which stay | +0.26 |
| Textures' top three mips: cut 9.32, not 9.30 | -0.02 |
| Sum | 4.86 |
| **Install** (0.28.0's measured 4.97, less the rest tier's 0.06) | **4.91** |

The last 0.05 GiB is the frame per tier's cost in the tiers kept, plus the estimate's
rounding; it is not split between them. The 3.78 is everything the estimate took off for
media: the 1.01 and 0.34 above, then video 1.46, voices 0.6 and music 0.37.

- **Nothing in the gap is something the install should leave out** today. Two rows are
  savings still to build: the gathered block fields, and the movies' tracks.
- **The bare minimum's estimate was too low by design**: it took the gathered fields off
  in full, but gathering saves bytes in proportion to the texture data left, and the bare
  minimum keeps only about 1.1 GiB of mips.

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

Cinematics' audio is another 1,174 MiB, in assets of 8 chunks each, one per language:
English 152 MiB, the other 7 together 1,022 MiB ([FORMATS.md](FORMATS.md), "Cinematic
dialogue").

## What can go without changing the look

Each row is measured after the ones above it, with no byte counted twice. All of them need
a new store.

| Saving | Packs after | Saved | How sure |
|---|---:|---:|---|
| The store now | 27.92 GiB | | |
| Identical content stored once, no copied chunk ends | 25.57 GiB | 2.35 GiB | Certain: compared byte for byte |
| Chunks nothing names | 22.85 GiB | 2.72 GiB | Strong evidence, not proof |
| Voice-over in folders of one language, English | 20.25 GiB | 2.60 GiB | Certain, if one language is all the owner wants |
| 7/8 of the cinematics' audio | 19.24 GiB | 1.01 GiB | Certain: `loc/en` names the English chunk of each; built in 0.29.0, 1,021.88 MiB |

Not in the tool's table, and not counted against the rows above:

- **7/8 of the movies' audio: 0.34 GiB.** Summing the WebM blocks by track, the 26 distinct
  movies hold 1.46 GiB of video and 0.40 GiB of audio, 16 of them with 8 audio tracks. Taking
  tracks out means rewriting the WebM files and their frame tables.
- **Gathering block fields: about 0.66 GiB** (the trial below), applied to today's sizes;
  built in nfsh 0.30.0, it saves 0.58 ([Installs](#installs)).

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
