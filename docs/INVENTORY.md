# Inventory of the install

What the owner's Need for Speed: Heat install (Steam, at
`D:\SteamLibrary\steamapps\common\Need for Speed Heat`) holds, as measured by
`nfsh inventory` on 2026-10-30. The formats are described in [FORMATS.md](FORMATS.md).
Everything below "On disk" is the command's output, unedited; to refresh it:

```
cargo run --release -- inventory "/mnt/d/SteamLibrary/steamapps/common/Need for Speed Heat" > out.md
```

## What it means for nfsh

- **The install is 34.45 GiB; 96% of it is cas archives** (33.15 GiB). Executables,
  installer leftovers and the tables are under 1.4 GiB together, so "smaller" is decided
  by how the archived data is stored.
- **3.87 GiB (11.7%) of the archives is dead**: `Data/` content that the patch replaced
  and nothing reads any more. Dropping it alone takes the archives from 33.15 to
  29.28 GiB without changing any data. That is a free first step for the import step,
  and a floor that nfsh's compact format has to beat.
- **Bulk data lives in chunks**: the three `chunks*` superbundles hold 28 GiB of stored
  chunks (texture mips, mesh buffers and similar), and the ebx and res entries, the
  descriptions of assets, are 2.43 GiB stored (6.80 GiB decompressed). Texture *res*
  entries are only headers (139k entries, 13.6 MiB); the pixels are in chunks.
- **Bundle references are heavily shared**: 725k ebx references name only 222k distinct
  ebx. Stored bytes counted per reference (37.53 GiB) exceed the live bytes
  (29.28 GiB) for that reason, not because data is stored twice.
- **Palm City is `Win32/levels/mainlevel/mainlevel`**, the only world level besides the
  frontend (confirmed by reading its root partition, see FORMATS.md): 2,033 bundles, 135k ebx, 71k res
  and 67k chunk references, 2.53 GiB stored, plus the chunks it pulls from `chunks*`.
  That is what the viewer has to read.
- `NeedForSpeedHeatTrial.exe` (400 MiB) is half of the executables' size and is not
  needed to play.

Not measured yet: how much of the live chunk data the viewer needs, and how well it
recompresses. Both come with the import step.

## On disk

392 files, 34.45 GiB (36994681502 bytes).

| What | Files | Size | Share |
|---|---:|---:|---:|
| cas archives, Data/ | 27 | 25.29 GiB | 73.4% |
| cas archives, Patch/ | 32 | 7.85 GiB | 22.8% |
| executables and libraries | 6 | 723.60 MiB | 2.1% |
| installer and redistributables | 224 | 380.46 MiB | 1.1% |
| tables (.toc, .sb, layout, initfs) | 93 | 229.36 MiB | 0.7% |
| other (licences, launcher files) | 10 | 5.95 MiB | 0.0% |

## Live and replaced data

Of 33.15 GiB in cas archives, current entries (patch applied) use 29.28 GiB (88.3%); the other 3.87 GiB is data the patch replaced, which nothing reads any more.

| Archives | Count | Size | Live | Live share |
|---|---:|---:|---:|---:|
| `Data/Win32/configurations/superbundleinstallpackage` | 19 | 18.23 GiB | 16.01 GiB | 87.8% |
| `Data/Win32/configurations/superbundleinstallpackage2` | 8 | 7.06 GiB | 5.49 GiB | 77.7% |
| `Patch/Win32/configurations/superbundleinstallpackage` | 18 | 4.29 GiB | 4.29 GiB | 99.9% |
| `Patch/Win32/configurations/superbundleinstallpackage2` | 14 | 3.56 GiB | 3.48 GiB | 97.9% |

## Superbundles

22 superbundles hold 20739 bundles listing 725215 ebx, 260398 res and 411889 chunk entries; distinct by name or id: 221745 ebx, 108281 res, 242640 chunks. Stored is the bytes the entries take in the archives, counting every reference (shared entries count once per bundle).

| Superbundle | Bundles | Ebx | Res | Chunks | Stored |
|---|---:|---:|---:|---:|---:|
| `Win32/chunks1` | 0 | 0 | 0 | 69246 | 12.41 GiB |
| `Win32/chunks0` | 0 | 0 | 0 | 64498 | 11.71 GiB |
| `Win32/chunks2` | 0 | 0 | 0 | 27114 | 3.88 GiB |
| `Win32/gamemodeeventdata` | 420 | 227620 | 92827 | 77111 | 2.62 GiB |
| `Win32/levels/mainlevel/mainlevel` | 2033 | 135052 | 71458 | 67347 | 2.53 GiB |
| `Win32/vehicles1` | 17377 | 238453 | 38492 | 66180 | 1.87 GiB |
| `Win32/sequences_normal` | 64 | 28838 | 15120 | 17066 | 1003.19 MiB |
| `Win32/loc/en` | 0 | 0 | 0 | 2520 | 821.92 MiB |
| `Win32/characters` | 441 | 23627 | 7698 | 7564 | 278.82 MiB |
| `Win32/garageblueprintbundles` | 3 | 3189 | 3334 | 2830 | 109.39 MiB |
| `Win32/levels/frontend/frontend` | 6 | 15267 | 1909 | 459 | 101.22 MiB |
| `Win32/activitydata` | 154 | 30147 | 5855 | 4393 | 93.35 MiB |
| `Win32/sequences_gameplay` | 87 | 3688 | 1482 | 1589 | 53.94 MiB |
| `Win32/globals` | 12 | 3888 | 8853 | 465 | 34.47 MiB |
| `Win32/ui` | 14 | 91 | 84 | 3 | 33.48 MiB |
| `Win32/vehicles2` | 106 | 5303 | 3394 | 3394 | 16.99 MiB |
| `Win32/voiceover` | 9 | 8233 | 8138 | 0 | 12.83 MiB |
| `Win32/showroomblueprintbundles` | 2 | 103 | 123 | 108 | 9.60 MiB |
| `Win32/voiceover2` | 8 | 1690 | 1626 | 0 | 3.74 MiB |
| `Win32/loadingmusic` | 1 | 22 | 4 | 1 | 0.13 MiB |
| `Win32/vehicles3` | 1 | 4 | 1 | 1 | 0.02 MiB |
| `Win32/vehicles0` | 1 | 0 | 0 | 0 | 0.00 MiB |

Listed in the layout but not installed: Win32/loc/ar, Win32/loc/br, Win32/loc/cn, Win32/loc/de, Win32/loc/es, Win32/loc/fr, Win32/loc/it, Win32/loc/ja, Win32/loc/ko, Win32/loc/pl, Win32/loc/ru, Win32/loc/tw.

## Resource types

Res entries by type. Names are given only where the type hash is confirmed; the others show the hash and one asset that carries it. All ebx and res entries take 2.43 GiB stored and 6.80 GiB decompressed; chunks (bulk data such as texture mips) have no decompressed size in the tables.

| Type | Entries | Stored | Decompressed | Example |
|---|---:|---:|---:|---|
| AssetBank | 44 | 442.62 MiB | 900.24 MiB | `animations/antanimations/levels/frontend/frontend_win32_antstate` |
| MeshSet | 51558 | 433.74 MiB | 1.57 GiB | `gameplay/gameplayassets/navigationhelp/startline_check_mesh` |
| `f20a477a` | 852 | 175.23 MiB | 584.14 MiB | `levels/mainlevel/worldart_zs_244_sunset_gigridstreaminggroup_sunset_gigrid2` |
| PhysicsResource | 19227 | 149.75 MiB | 575.88 MiB | `.74044e4b-7559-44e7-b647-8697606f58b6.racevolumetriggerdata` |
| NewWaveResource | 22574 | 33.87 MiB | 63.36 MiB | `audio/ui/game_modes/uisound_event_dragrace_burnoutfailed` |
| `063994c5` | 80 | 33.44 MiB | 63.85 MiB | `webbrowser/fontassets/eadp_helvetica_bold` |
| `0e2b02f7` | 21355 | 32.31 MiB | 152.54 MiB | `shaders/bytecode/8ed8182c-e8ab-21b2-66a1-5b18b1d15415` |
| EnlightenStaticDatabase | 1256 | 29.71 MiB | 237.90 MiB | `levels/mainlevel/worldart_radiosity/zone_static_244_sunset` |
| `85ea8656` | 42 | 15.07 MiB | 22.27 MiB | `characters/hair/hair_010/models/hair_010_f_cloth_alpha_test_clothwrappingasset` |
| Texture | 138874 | 13.63 MiB | 17.48 MiB | `ui/textures/hud/activities/horizontalblurmainwhite` |
| `36f3f2c0` | 12 | 3.46 MiB | 27.00 MiB | `gameplay/activities/utility/activitystreamedprefabbundlecollectionasset_445068875_bbid_sharedbundle/shaderdb` |
| TerrainDecals | 1 | 2.95 MiB | 8.64 MiB | `levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048_levels/mainlevel/mainlevel/decals` |
| EnlightenDatabase | 309 | 2.15 MiB | 20.10 MiB | `systems/enlighten/enlightendefault` |
| TerrainStreamingTree | 1 | 1.91 MiB | 5.68 MiB | `levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048.streamingtree_win32` |
| `85ac783d` | 42 | 1.71 MiB | 2.96 MiB | `characters/hair/hair_010/cloth/hair_010_f_eacloth` |
| `0ede7594` | 6 | 1.37 MiB | 13.38 MiB | `characters/common/meshcompute/famemeshcomputeasset/0937b052-7564-6e3d-6a23-7108ed9d4d43_faceadjacency` |
| `00d41d60` | 1 | 1.07 MiB | 3.00 MiB | `fx/_textures/hashtagfxgroundtexture` |
| IesResource | 66 | 0.78 MiB | 2.06 MiB | `iesprofile/4m_below/ies_inc_150w_warningbulb` |
| OccluderMesh | 1759 | 0.49 MiB | 1.41 MiB | `worldassets_hashtag/architecture/residential/poor/prop_vanhouse_02_mesh_occludermesh` |
| `7dd4cc89` | 1570 | 0.38 MiB | 0.91 MiB | `gameplay/checkpoints/checkpointlogic/calculatecheckpointspatialtransform` |
| `f04f0c81` | 1 | 0.25 MiB | 5.40 MiB | `systems/shaderprogramdb` |
| TerrainLayerCombinations | 1 | 0.15 MiB | 1.60 MiB | `levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048.layercombinations_win32` |
| AtlasTexture | 611 | 0.06 MiB | 0.05 MiB | `fx/_textures/debris/brokenglass_4x2` |
| MeshEmitterResource | 2 | 0.04 MiB | 0.17 MiB | `fx/nfs20/prototype/mea_proto_emitfrommesh_03` |
| ZoneStreamerGrid | 3 | 0.03 MiB | 0.12 MiB | `levels/mainlevel/zonestreamer/drivablesurfaces_zonestreamerasset` |
| AnimTrackData | 138 | 0.02 MiB | 0.03 MiB | `ui/menu/animtrackdata/18371652-ec83-4740-924e-6081b4673c7d_alpha_bg` |
| `81f0e34f` | 6 | 0.01 MiB | 0.01 MiB | `characters/common/meshcompute/famemeshcomputeasset/0937b052-7564-6e3d-6a23-7108ed9d4d43_meshdefinition` |
| VisualTerrain | 1 | 0.00 MiB | 0.02 MiB | `levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048` |
| `2c3e1e37` | 6 | 0.00 MiB | 0.00 MiB | `characters/common/meshcompute/famemeshcomputeasset/0937b052-7564-6e3d-6a23-7108ed9d4d43_indexbuffer` |
