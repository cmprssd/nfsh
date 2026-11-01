# Heat's data formats

Our own description of the files in a Need for Speed: Heat install, worked out by
reading them; the readers in `crates/frostbite` follow it. Numbers are big-endian unless
noted. Nothing here is copied from EA code or tools.

## Install layout

```
Data/                       the game as shipped
  layout.toc                superbundle list and install packages (signed DbObject)
  Win32/<superbundle>.toc   table of contents: bundles and chunks
  Win32/<superbundle>.sb    the bundles themselves (entry lists, not asset data)
  Win32/configurations/superbundleinstallpackage{,2}/cas_NN.cas   stored asset bytes
Patch/                      every later update, same layout
```

`Patch/` superbundles list only what changed. A patched bundle replaces the `Data/`
bundle with the same name, and a patched chunk replaces the chunk with the same id;
everything else still comes from `Data/`. Patched entries may point back into `Data/`
archives.

## Signed files

`layout.toc`, every `.toc` and `initfs_Win32` start with the magic `00 D1 CE 01` and a
0x22C-byte header holding a signature. The body after it is plain (not encrypted).

## DbObject

A tagged binary tree used by `layout.toc` and `chunkmanifest`. Each value is a type byte
(low 5 bits the type; bit 7 set when the value has no name), a NUL-terminated name, then
the payload:

| type | payload |
|------|---------|
| 0x00 | end of the enclosing list or object |
| 0x01, 0x02 | list, object: LEB128 byte size, then values |
| 0x06 | bool (1 byte) |
| 0x07 | string: LEB128 length including the NUL |
| 0x08, 0x09 | int32, int64 (little-endian) |
| 0x0B, 0x0C | float32, float64 (little-endian) |
| 0x0F, 0x10 | GUID (16 bytes), SHA-1 (20 bytes) |
| 0x13 | blob: LEB128 length |

`layout.toc` has `superBundles` (objects with a `name`) and
`installManifest.installChunks` (objects with a `name` such as
`configurations/superbundleinstallpackage`). An install chunk's position in that list is
the package number in a cas reference.

## Cas references and locations

A cas reference is one word: byte 2 is 1 for `Patch/`, byte 1 the install package, byte
0 the archive number (`cas_NN.cas`). A location is a reference, an offset and a stored
size. The stored bytes are compressed blocks, each with an 8-byte header (decompressed
size u32, codec u16 `0x1170` (Oodle in Heat's files), stored size u16), with no gap
between entries.

## Stored blocks

An entry's stored bytes are a chain of blocks that ends exactly at the location's size.
Each block is an 8-byte header and a payload:

| field | |
|-------|--|
| decompressed size | u32, 1 to 0x10000 (the top byte, flags in other games, is always 0) |
| codec | u16 |
| stored size | u16, the payload's length |

| codec | payload |
|-------|---------|
| 0x1170 | Oodle, decompressed with the install's `oo2core_6_win64.dll` |
| 0x0070, 0x0071 | stored as is: as long as the decompressed size; for a full 0x10000-byte block the stored size field wraps to 0 |

No other codec appears in the first 256 MiB of the archives sampled, and `nfsh verify`
finds none in any current entry. An ebx or res entry decompresses to exactly its listed
size. A chunk entry in a bundle stores only the chunk's blocks that cover its logical
range: from the 0x10000 boundary at or before the logical offset to the offset plus
the logical size, so it decompresses to `offset % 0x10000 + size` bytes. For a texture,
that is the last small mips of the chain; the whole chain is a superbundle's chunk
(see "Textures"). All 1,317,687 distinct entries of the owner's install (31.5 GiB stored, 65.4 GiB
decompressed) pass these checks.

## Table of contents (`.toc` body)

Twelve words, offsets from the start of the body:

| word | meaning |
|------|---------|
| 0 | bundle hash table (lookup by name; not needed for a listing) |
| 1, 2 | bundle records, bundle count |
| 3 | chunk hash table |
| 4, 5 | chunk records, chunk count |
| 6, 7 | unknown (equal to word 9 in every file) |
| 8 | name table |
| 9, 10 | chunk locations, their length in words |
| 11 | 0 in `Data/`; small numbers in `Patch/` (meaning unknown) |

A bundle record is 16 bytes: name offset (into the name table), size, offset (u64) of
the bundle in the `.sb`. A chunk record is 20 bytes: the chunk id, then a word whose top
byte is a flag (always 1) and whose low 24 bits index the location words, where the
chunk's location is three words: cas reference, offset, size.

## Bundles (`.sb`)

A bundle starts with eight words: offset of the binary bundle (0x20), offset of the
location records relative to it, offset of the location flags, entry count, three
copies of the binary bundle's end, zero.

The binary bundle is a size word, then (offsets from after the size word):

| field | |
|-------|--|
| magic | `9D798ED6` |
| total, ebx, res, chunk counts | total = ebx + res + chunks |
| string table offset, metadata offset, metadata size | |
| SHA-1 × total | entry order: ebx, res, chunks |
| ebx × ebx count | name offset, decompressed size |
| res × res count | name offset, decompressed size |
| res type × res count | hash of the type name (below) |
| res metadata × res count | 16 bytes each (meaning not worked out yet) |
| res id × res count | u64 each |
| chunk × chunk count | id (16 bytes), logical offset, logical size |
| metadata | a DbObject (`chunkMeta`) |
| strings | NUL-terminated names |

The location records follow, one per entry in entry order, with one flag byte per entry
at the flags offset. Flag 1: a cas reference, an offset and a size; flag 0: an offset
and a size in the same archive as the entry before.

Resource type hashes are `h = h * 33 ^ byte`, from 5381, over the lowercased type name:
`Texture` is `0x6BDE20BA`. Names are confirmed only when the hash matches and the
assets carrying it fit the name.

## Ebx partitions

An ebx entry, decompressed, is one *partition*: typed objects (*instances*) with the
type descriptions they need, so it reads without any outside type database.
**Little-endian**, unlike the tables. Worked out on a few thousand partitions of
`levels/mainlevel`; everything below is checked on every ebx in the install (see the
end of this section). `frostbite::ebx` reads it, and `nfsh ebx STORE NAME` prints one.

Header (0x40 bytes):

| offset | field |
|-------:|-------|
| 0x00 | magic `0x0FB4D1CE` |
| 0x04 | meta size: every section before the strings; also where the strings start |
| 0x08 | the rest: meta size + this = the entry's size |
| 0x0C | import count |
| 0x10 | instance repeater count (u16) |
| 0x12 | exported repeater count (u16): every instance of the first this many repeaters has a GUID |
| 0x14 | how many distinct types the instances have (u16) |
| 0x16, 0x18 | type count, field count (u16) |
| 0x1A | names size (u16) |
| 0x1C | strings size |
| 0x20 | array repeater count |
| 0x24 | instance data size |
| 0x28 | the partition's GUID (16 bytes) |
| 0x38 | boxed value count |
| 0x3C | where the boxed values start, from the start of the instance data (the end, if there are none) |

Then, back to back from 0x40:

| section | each |
|---------|------|
| imports | 32 bytes: partition GUID, instance GUID (what this partition points at elsewhere) |
| names | NUL-terminated type and field names, the header's size in bytes |
| fields | 16 bytes: name hash, flags (u16), type index (u16), offset, second offset |
| types | 16 bytes: name hash, first field index, field count (u8), alignment (u8), flags (u16), size (u16), second size (u16) |
| instance repeaters | 4 bytes: type index (u16), how many instances in a row (u16) |
| *pad to 16* | |
| array repeaters | 12 bytes: offset, count, type index; the first is an empty array |
| *pad to 16* | |
| boxed value refs | 8 bytes each |
| *pad to 16* | ends exactly at the meta size |
| strings | NUL-terminated; the first is the partition's own name (`levels/MainLevel/MainLevel`) |
| *pad to 16* | |
| instance data | the header's size; then the arrays and boxed values to the end |

Names are referenced by hash: `h = h * 33 ^ byte` from 5381 over the name as written
(the res type hash without lowercasing): `$`, a type's link to its base type, is
`0x0002B581`. A type's fields are a run of the field table, its base type first.

A field's kind is bits 5-9 of its flags:

| kind | | kind | |
|-----:|---|-----:|---|
| 0x00 | base type (`$`) | 0x0F, 0x10 | int32, uint32 |
| 0x02 | struct (value type), stored in place | 0x11, 0x12 | int64, uint64 |
| 0x03 | class reference (4 bytes) | 0x13 | float32 |
| 0x04 | array (4 bytes: an array record index) | 0x14 | float64 |
| 0x07 | string (4 bytes: an offset into the strings; 0xFFFFFFFF for none) | 0x15 | GUID (16 bytes) |
| 0x08 | enum (4 bytes) | 0x16 | 20 bytes, not worked out (1,570 fields, all `ExpressionFunctionTypeInfoAsset.DebugData`) |
| 0x09 | 8 bytes, not worked out (1,718 fields, all `VertexShaderFragmentAsset.SourceFile`) | 0x17 | resource reference (8 bytes: names a res by id) |
| 0x0A | bool (1 byte) | 0x19 | type reference (8 bytes) |
| 0x0B, 0x0C | int8, uint8 (0x0B not seen) | 0x1A | boxed value (16 bytes) |
| 0x0D, 0x0E | int16, uint16 (0x0D not seen) | | |

The integers come in signed/unsigned pairs: the widths are measured (the smallest gap
to the next field and the alignment of every offset, over every type in the install),
and the 64-bit pair names its sign (`Int64Value` is 0x11, `UInt64Value` 0x12).

Types use the same flags: 0x0063 for a class, 0x0045 for a struct, 0x0089 for an
array, 0xC10B for an enum, whose fields (flags 0x01E0) are its members.

**Instance data** holds the instances in repeater order, back to back with no
alignment between repeaters. Every instance of an exported repeater is preceded by
its 16-byte GUID. Each instance's stored bytes start at the next multiple of its
type's alignment (after the GUID, if any; the GUID moves with them). A type aligned
to fewer than 8 bytes does not store its first 8 bytes: its field offsets count from
8 bytes before its stored bytes, and it takes size − 8 bytes in the file. A type
aligned to 8 or 16 stores them all.

**Arrays** follow the instances at the next multiple of 16 that leaves 4 bytes after
the last instance (end + 4, aligned to 16): the header's instance data size ends
there. An array record's offset counts from there to its first element; the element
count is stored in the 4 bytes before. Record 0, the empty array, is shared by every
empty array field whatever its type, so a field's array type comes from the field,
not the record. Elements are the member field's kind, or the struct's size each.

A **class reference** is 0 for none, an import index with the top bit set
(0x80000000 | index), or an instance index plus one.

Checked on the whole install, 2026-10-31: every ebx in nfsh's store (725,119
locations, 2.57 GiB decompressed) has the magic, sizes that add up to the entry, field
ranges inside the field table, and sections that end exactly at the meta size; 14,338
have boxed values. The reader's store test then reads every value of every instance
(2,659,748 instances, 175,929,646 values, following structs and arrays) and finds the
rules above hold everywhere: each instance fits, each array count matches, each
internal reference lands on an instance of the field's class, each non-empty array
has the field's type, fields of known width fit their type without overlapping, and
0x14 and 0x3C mean what the header table says.

## Palm City

Palm City is `Win32/levels/mainlevel/mainlevel`. Its root partition,
`levels/mainlevel/mainlevel` (21.4 MB, in the bundle of the same name), holds one
`LevelData` and the road network (11,907 `RoadEdgeSegment`, 6,418 `RoadEdge`, 4,233
`RoadNode2`, 1,007 `RoadIntersection`), and references its sub-levels by name:
StaticWorld, GameSpatialData, GameLogic, RobotoGameLogic, Prototypes, GameCollectibles
and CinematicsProduction.

How the drawable world hangs together, read by `frostbite::level` and printed by
`nfsh level STORE [ZONE]`; checked on the whole city (see the end of this section).

**Sub-levels are bundles, linked by name.** A `SubWorldReferenceObjectData` or a
`ZoneStreamerSubWorldRod` has a `BundleName` string (`levels/mainlevel/worldart_zs_495`),
which is the bundle `Win32/<name>` (any case), and the bundle's root partition has the
same name. Following every such link in every ebx of each bundle, from
`levels/mainlevel/mainlevel`, reaches 2,034 bundles, six links deep:

- `mainlevel` links to 7 sub-levels; `levels/mainlevel/staticworld` (a `SubWorldData`)
  to WorldArt, TimeOfDay_Databases, InteriorLightmaps, DrivableSurfaces, WorldVista,
  WorldRoadDecals, BakingLights and WorldAudio, and to 3 layers by reference.
- The `worldart` bundle's root partition is empty; its rods are in another of its
  partitions, `overridenassets/<guid>_subworlddata` (a `ZoneStreamerEntityData` and 19
  `ZoneStreamerSubWorldRod`). Zones then link to further zones, regions
  (`worldart_zs_mainlevel_region_{common,city,jungle,rural}`) and their own lighting
  variants, so the 232 zones `worldart_zs_<n>` are reached at depths 3 to 6.
- `ZoneStreamerNotificationEntityData` has a `BundleName` too, but it names no bundle
  (`617`, `mainlevel_region_common`): it is not a link.

**Everything else is a class reference**, often an import into another partition,
found by its partition GUID (nfsh's store lists them, see below). A partition is often
copied into several bundles: 725,215 ebx entries hold 221,738 distinct partitions, and
of the 41,414 with copies, all but 7 keep one name and all but 236 the same bytes. The
level reader takes the copy in the level's own superbundle when there is one.

**What a zone places.** A zone's root partition holds a `SubWorldData` whose `Objects`
list is followed, recursively:

| object | what it places |
|--------|----------------|
| `StaticModelGroupEntityData` | its `MemberDatas`: each a `MeshAsset` reference and `InstanceTransforms` (exactly `InstanceCount` of them), under the group's `Transform` |
| `StaticModelEntityData` | its `Mesh` at its `Transform` |
| `ObjectReferenceObjectData`, `SpatialPrefabReferenceObjectData`, `LayerReferenceObjectData` | its `Blueprint` at `BlueprintTransform` |
| `ObjectBlueprint` | its `Object` (usually a `StaticModelEntityData` in the same partition) |
| `SubWorldData`, `LayerData`, `SpatialPrefabBlueprint`, `PrefabBlueprint` | each of its `Objects` |

A zone's `LayerReferenceObjectData` places its `_layer` partition's `LayerData`, which
holds the placed objects (`ObjectReferenceObjectData`, prefabs, lights, effects).

A `LinearTransform` is four `Vec3`: `right`, `up`, `forward` (the axes) and `trans`
(the origin), in the parent's space; a point (x, y, z) goes to
x·right + y·up + z·forward + trans. Placements in zones are in world coordinates
(zone 495, the speedway, sits around x 3,700-4,400, y 80-95, z 2,800-3,350); the zone
and group transforms seen so far are the identity. Composing a prefab's child into
its parent was checked by hand on zone 495 (a tube light inside a tripod prefab).

Checked on the whole city, 2026-10-31: all 232 zones walk with every reference
resolved, placing 259,999 meshes; counted once per zone, 21,015 `RigidMeshAsset` and
6,738 `CompositeMeshAsset`. Not followed yet (counted by `nfsh level`): lights,
probe and emitter volumes, effects locators, sound locators, `ClothObjectBlueprint`
(87) and the zones' lighting (`DynamicEnlightenEntityData`).

- Every zone has four lighting variants, `worldart_zs_<n>_midday`, `_overcast`,
  `_sunset` and `_tealnight`: each a small bundle with the zone's precomputed global
  illumination (`StaticEnlightenEntityData`, a GI grid, and `worldart_radiosity/`
  textures).
- Meshes are `RigidMeshAsset` ebx whose geometry is a res (`MeshSetResource` names it
  by id) and chunks; textures are `TextureAsset` ebx with a `Texture` res (the header)
  and a chunk (the pixels). A zone's bundle holds most of the meshes, blueprints and
  textures it uses; the rest are in shared bundles (the regions).
- The ground is in two other places: the roads in the drivable-surface zones and the
  terrain in the root bundle (the next two sections).

## Drivable surfaces

The roads, kerbs, bridges, car parks and their decals are not in the art zones: they
are 66 bundles `levels/mainlevel/drivablesurfaces_zs_<n>` (reached through
`levels/mainlevel/drivablesurfaces`, a `SubWorldData` whose two objects are imports),
each with `_midday`, `_overcast`, `_sunset` and `_tealnight` lighting variants like the
art zones'. A drivable zone's root partition is a `SubWorldData` placing
`RigidMeshAsset` roads (`worldassets_roboto/roads/…/rd_…_mesh`), manholes, bridges and
collision walls, walked exactly as an art zone is: all 66 walk with every reference
resolved, 11,836 placements (2026-11-01).

They lie on a coarser grid than the art zones: numbers 24 to 167 in rows of 14, each
about 1.4 km across (zone 123 places its roads between x 3,301 and 4,507, z 2,634 and
3,982), against the art zones' numbers to 630. Which art zones a drivable zone covers
is not stated anywhere found yet; the viewer takes, from all of them, the placements
whose origin lies within 150 m of the art zone's own (zone 495: 341).

**Road materials.** A material names its shader by reference (`Shader`, a partition
such as `shaders/world/roads/rd_drv_blendwithterrain`) and, beside its textures, sets
`VectorParameters` (each a `Vec4` `Value` and a `ParameterName`: a scalar in `x`, a
colour in `xyz`). The roads near zone 495 use six road shaders (counted 2026-11-01):

| shader | sections | what it draws |
|--------|---------:|---------------|
| `rd_drv_blendwithterrain`, `rd_drv_basesurface_wetness` | 84, 40 | the asphalt: `Tarmac_FG_Diffuse` and `Tarmac_BG_Diffuse` (clean and damaged layers, tinted by `Tarmac_Tint_FG`/`_BG`, tiled by `Tarmac_Tiling_FG`/`_BG`), `detailMask` with `detailColor1`-`5`, normals, shoulder and dirt layers |
| `rd_dcl_transparentdcl_lerp` (`shaders/world/roads/` and `wstroads/`) | 39, 53 | cracks, tar and road lines over the asphalt |
| `rd_dcl_transparentdcl_lerpnsmultiply` | 28 | tyre marks, rumble strips |
| `rd_dcl_transparentdcl_lerpbasecolor` | 5 | arrows and lettering |
| `rd_dcl_base_parallax` | 14 | manholes, with an ordinary colour texture |

The road textures are **unorm, with their channels packed**, not sRGB colour: the
asphalt's `drv_set5_clean_d` (4096², BC7) has a grey in red (mean 86 of 255), another
grey in green, a sparse mask in blue and alpha unused. Read as sRGB-encoded, the red
is about 0.09, times the tint 0.57 is 0.05, the same as the material's `detailColor1`
(0.053). A decal's texture holds its shape in one channel, which `alphaSelector`
picks: (1, 0, 0, 0) for cracks and tyre marks (red), (0, 1, 0, 0) for road lines
(green, the stripes); `Opacity` scales it (tyre marks 0.4), and `lineColor`, where
set, is the colour (cracks 0.214, tyre marks 0.003, arrows 0.716). Road lines set no
`lineColor`: their texture's red is near white (214) and their vertex colour makes
them white or yellow.

**Asphalt** (`rd_drv_blendwithterrain`, `rd_drv_basesurface_wetness`; worked out on
2026-11-01 from the material parameters, the textures and the shaders' programs).
A material names a clean (`…_FG_…`) and a damaged (`…_BG_…`) layer, each a diffuse
and a normal map at its own tiling, a `detailMask` for the road and, in some, a
`Noise`; a material that names no noise gets the shader's own,
`roadsurfaces/detailtexture/drv_road_detail_d`, which its parameter block binds (most
of the asphalt near zone 495 leaves it out). The textures:

| texture | channels |
|---------|----------|
| layer diffuse (`drv_set…_d`, BC1 or BC7) | red: the grey; blue: where `detailColor1` (damaged layer) or `detailColor2` (clean) shows |
| layer normal (`drv_set…_n`, BC7) | red, green: the normal; blue: a height (mean 0.55); alpha not worked out |
| `detailMask`, one per road set (`…/mask/rd_…_mask`, 1024², BC7) | an atlas of the roads' strips in the second texture coordinates (0x22): red, light with dark patches, where the clean layer shows; green, white on the road and grey off it; blue, white with dark lines along the wheel paths; alpha a noise |
| `Noise` (`drv_road_detail_d`, 4096², BC7) | red, green, blue: where `detailColor3`, `4` and `5` show, in the first texture coordinates as they are |
| `Shoulder_NH` (`roadside/drv_shoulder_n`, 2048 × 4096, BC7) | red, green: a normal; blue: where the road shows, an atlas of edge profiles across x, white with three black bands (0.375-0.5, 0.69-0.75, 0.94-1.0) with ragged edges; alpha white |

The vertices carry two colours: the first (0x1E) scales the layers' blue (green for
`detailColor1`, blue for `2`; its red is 0 on the road pieces checked), the second
(0x1F) the noise's three channels. The colour is, in order:

1. Each layer's grey times its tint (`Tarmac_Tint_FG`, `…_BG`), the coordinates
   times its tiling (`Tarmac_Tiling_FG`, `…_BG`).
2. The two blended: the damaged layer's height times the mask's red, less
   `Tarmac_Blend_Offset`, times `Tarmac_Blend_Contrast` × 10 (at least 1), plus
   `VertexColor_Blend_Offset`, clamped to 0 to 1 and smoothed (3t² − 2t³), is how much
   of the clean layer shows.
3. The five detail colours laid over it in turn, each as much as its amount above
   says: an overlay, darkening under a half (2ab) and lightening over it
   (1 − 2(1 − a)(1 − b)), so a colour of 0.5 leaves it as it is.
4. The wheel paths: the same blend of the mask's blue with `RoadTire_Blend_Offset` and
   `…_Contrast`, taken from 1, scales the colour by 1 + w(2 `RoadTire_Tint_Color` − 1).
5. `rd_drv_blendwithterrain` only (`rd_drv_basesurface_wetness` has no such step):
   blended into the terrain's colour where the road gives way to it. The shoulder is
   `Shoulder_NH`'s blue at the third texture coordinates (0x23) times
   `Shoulder_Tiling`, and 0 where the coordinates' x is outside 0 to 1; a permutation
   for vertices without them samples it at (0, 0). The dirt is the same blend as in
   step 2 of the mask's green times 1 − `Dirt_Height_Paint_Boost` times the layers'
   height (the two normal maps' blue, mixed as much as step 2's clean layer shows),
   with `Dirt_Height_Offset` and `…_Contrast`, taken from 1. With the first vertex
   colour's red r, the road shows max(shoulder, r) × (1 − dirt × (1 − r)), and the
   terrain the rest. The road pieces checked near zone 495 have r = 0 and the third
   coordinates' x between 0.03 and 0.97, so the shoulder's atlas sets the edge.
   The terrain's colour is sampled from its virtual texture at the pixel's world
   position (squared, as if it were stored with a gamma of 2); nfsh takes its own
   ground's colour there instead.
6. Clamped to 0.04 to 0.5, the terrain's share too.

Besides these, the shader darkens it when wet (`WaterCycle`) and for occlusion
(`OccShadow_*`, which leaves it as it is at the defaults), and adds the normal maps;
nfsh does none of these yet.

A parameter block's constants (between its third and fourth offsets) are records of
60 bytes: the constant's name (32 bytes, zero-padded), its name hash, a slot word,
its component count (u32) and its default (four f32). The slot word is 0x10000 plus
one more than the constant's register in the program's second constant buffer: the
asphalt's `Tarmac_Tiling_BG` is 0x10001, register 0. A
material that leaves a constant out gets the block's default: for the asphalt,
`Tarmac_Blend_Offset` 0.171, `…_Contrast` 0.880, `RoadTire_Blend_Offset` 0.502,
`…_Contrast` 0.880, `VertexColor_Blend_Offset` 0.214, `Dirt_Height_Offset` 0.216,
`…_Contrast` 0.815, `…_Paint_Boost` 0, the detail colours and `RoadTire_Tint_Color`
0.5, the tints and tilings (`Shoulder_Tiling` among them) 1.

## Terrain

Palm City's terrain is `levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048`
(root bundle): a `TerrainData` ebx naming three res by id and listing its 39
`TerrainLayers` (each with mesh scattering, the grass and bushes). Worked out so far,
2026-11-01:

| res (type hash) | size | what |
|-----------------|-----:|------|
| `TerrainStreamingTreeResource`, `…streamingtree_win32` (`0x22FE8AC8`) | 5,957,625 | the quadtree and the chunk of every tile (below) |
| `VisualResource`, `mainlevel_terrain_2048` (`0x1CA38E06`) | 19,833 | the scattering: mesh names (`sct_grass_…_mesh`) and their settings |
| `TerrainLayerCombinationsResource`, `…layercombinations_win32` (`0xA23E75DB`) | 1,680,272 | each combination of layers and its shaders' names ("Terrain layers", below) |

**The streaming tree** is packed little-endian with no alignment; `frostbite::terrain`
reads it. Worked out on 2026-11-01 and checked on every heightfield (below).

The header is 0x7F bytes; of it only two counts are read: the tiles (u32 at 0x15,
8,253) and the nodes (u32 at 0x6B, 10,077). Then a depth-first **quadtree of nodes**
covers x and z from −8,192 to 8,192 m, down to 128 m leaves (6,968 of them). A node is
its bounding box (six f32: minimum x, y, z, then maximum; y is the height range) and an
f32, then a byte: 1 marks an empty node (the open sea; 50 of them) and ends it there,
29 bytes in all. Otherwise four more bytes follow (the first is 1 for the root and the
four 8,192 m nodes, which then carry 340 bytes: min/max height pairs, u16, for
1 + 4 + 16 + 64 cells; the second is 1 for every other node), and a last byte, 1 when
four children follow, in the order (0, 0), (1, 0), (1, 1), (0, 1) (x first). The tree
ends at `0x54083`.

Then **blocks**, each a kind (u8) and a length (u32) before its bytes, until a kind
of 0xFF with no length. Palm City has four, kinds 1 to 4 (1,335,460, 128,136,
3,836,271 and 1,063 bytes). Kinds 2 to 4 are not worked out (each starts with the
world's square as four f32, −8,192 to 8,192; kind 3 is mostly 3-byte cells).

**Kind 1 is the layer tree**, where each layer lies (worked out on 2026-11-01;
`frostbite::terrain` reads it and a store test checks it): a 0x38-byte header (0x88,
1, 0, 4,096, 6,144, 2, the world's square as four f32 at 0x18, then the node count,
21,429, and the record count, 37,033, at 0x28 and 0x2C), then a depth-first quadtree
of nodes over that square, in the tiles' child order, down to 32 m. A node is a zero
byte, its record count (u16), how many of them have a page (u16) and a zero u16 (7
bytes); with no records it is empty and ends there (2,740 of them). Otherwise its
records follow, then a zero byte and a byte that is 1 when four children follow.

A **record** is 31 bytes: the layer (u16, an index into the `TerrainData`'s
`TerrainLayers`), the node's square (four f32: minimum x, z, then maximum), a byte
that is 1 when no node below lists the layer, a byte that is 1 when the layer has
**no page** at this square, and an 85-bit pyramid (11 bytes) of the node's
1 + 4 + 16 + 64 cells below it that the layer touches (not read yet). A node's
records are in ascending layer order. The root's one record is layer 0, which no
shader lists.

**A tile's pages are its square's records that have one**: on all 6,972 tiles with a
node at their square, the page count (from the chunk's size, below) is exactly that
node's count of records with a page; the other 1,276 tiles have no node of their own
and no pages. The speedway tile at (3,712, 2,944) has pages for layers 2, 8, 9, 24
and 25. **The pages are likely in the records' (ascending) order**: predicting each
colour texel (below) as the pages' weighted mean of their layers' diffuse colours,
on 150 128 m tiles whose layers all have plain colour maps, ascending order gives an
RMS error of 24.8 (in 0 to 255 units), the best order for each tile 21.8, the reverse
32.5; on the 108 two-layer tiles ascending is the better order 80 times. The model is
crude (painting each layer over the last instead fits far worse, 61), so this is
evidence, not proof.

Then, to the end, **the tiles**: a second quadtree over the same square, depth-first
from the root, in the same child order. It is not the node tree: it stops at 256 m in
some places and goes down to 64 m in others (108 tiles). A tile is a chunk's size (u32)
and id (16 bytes, an in-memory GUID: `store::guid_order` gives the id a bundle's chunk
list uses), a count (0 or 1) of further chunks with their size and id (20 bytes each),
a byte (always 0), and a byte that is 1 when the tile's four children follow. The root
and the four 8,192 m tiles have no chunk (size and id zero). The chunks are superbundle
chunks of `mainlevel`. A tile's first chunk is one of two kinds, told by its size
(k runs 0 to 16; 18,496 = 136 × 136, and the pages look like one-byte masks of that
size, likely a layer's weight each, not read yet):

- `42,231 + 18,496·k` bytes (7,618 tiles): a **heightfield**, 137 × 137 u16 first
  (37,538 bytes), then 2,381 bytes (on the speedway tile they read as 17 × 17
  min/max pairs of heights, u16, then a 35 × 35 grid of bytes 0 to 9 and 0xFF, not
  worked out), then the pages and the colour below.
  Height is the u16 ÷ 32 m
  (the sea floor's 5 is the root's minimum, 0.156 m; the maximum 30,292 is 946.6 m);
  0 is a **hole** (the ground is left out: 64 tiles cut holes their parents don't
  have). Rows run along z and columns along x, with a 4-sample border: sample (r, c)
  of a tile at (x0, z0) of size S lies at (x0 + (c − 4)·S/128, z0 + (r − 4)·S/128), so
  a 128 m tile has a sample every metre.
- `2,312 + 18,496·k` bytes (630 tiles): no heights, only the pages and the colour.
  A tile like this is drawn from its nearest ancestor's heights.

Both kinds end in the tile's **colour**: its last 2,312 bytes are BC1 blocks of a
68 × 68 image (sRGB-encoded), rows along z and columns along x like the heights, with
a 2-texel border: texel (r, c) of a tile at (x0, z0) of size S covers from
(x0 + (c − 2)·S/64, z0 + (r − 2)·S/64), so a 128 m tile has a texel every 2 m. It is
the ground's colour already blended from its layers: grass, earth, sand, the old road
beds and the sea floor's tint. Laid side by side, the leaves around the speedway meet
with no seams. A store test reads every tile's colour (2026-11-01).

The further chunk, when there is one, is another `2,312 + 18,496·k` chunk or, on 300
of the 128 m leaves, 168,924 = 4 × 42,231 bytes: likely four 64 m heightfields.
Neither is read yet.

**The pages** (18,496 = 136 × 136 bytes each) are one layer's weight each, 0 to 255,
just before the colour, in rows along z and columns along x with a 4-texel border:
read like the colour, texel (r, c) of a tile at (x0, z0) of size S covers from
(x0 + (c − 4)·S/128, z0 + (r − 4)·S/128) (texel middles are assumed, as for the
colour; the heights' sample points would put them half a texel over, not
checked). On the speedway tile their means are 211, 88, 92, 9 and 118, so they
don't sum to 255 per texel; nfsh takes a layer's weight as its share of the sum
(2026-11-01, `terrain::Pages`). Which layer each is for: the layer tree, above.

**How it was checked**: a child's even samples are its parent's, at the child's
quadrant. Of the 7,602 parent and child pairs that both have heights, 7,477 agree on
every sample, 61 differ by at most 2 units (1/16 m) on a few dozen samples, and 64
differ only where one of them has a hole. The heights of every leaf also span exactly
its node's y range in the node tree. A store test checks the first rule on every pair.

### Terrain layers

Worked out on 2026-11-01; `frostbite::layers` and `frostbite::shaderdb` read what
it takes to find each layer's diffuse maps, and a store test checks them.

**The layer combinations res** is an array of 2,468 entries of 0x58 bytes from 0x20
(the count, u32 at 8). An entry is seven arrays, each a count (u32) and an offset
(u64, from the res's start), then a u32. Arrays 1 to 6 point at 0x28-byte records
naming a shader (an offset (u64) to its name), each with two arrays of u16 layer
indices (a count (u32) and an offset (u64) each), then 8 bytes not worked out.
Array 4 lists the entry's `__2d` shader (282 different ones), array 1 its
`MaskScale2d` ones, 3 `GlobalColorSurfaceShader` and 6 a few `__3d` ones. A `__2d`
shader's first array is its layers: `…mainlevel_terrain_2048__46M_46V__2d__0`
lists layers 1, 2 and 6, the bits of its hex `46` (the name also sets bit 0 in 4
shaders; the array leaves layer 0 out, and nfsh goes by the array). The names (2,368 shaders, all also ebx `ShaderGraph`s with no fields)
come in two passes: `…__<M>M_<S>S__MaskScale2d__0` (2,080) and `…__<M>M_<V>V__2d__0`
(282), M being the set of layers as bits. The speedway tile's layers {2, 8, 9, 24,
25} are `3000304M`, and `…__3000304M_1000004S__MaskScale2d__0` exists.

**A layer's textures are in the level's shader database**, the res
`levels/mainlevel/mainlevel/shaderdb` (type `0x36F3F2C0`, 16,710,480 bytes), which
names every texture by the ebx name hash (`ebx::name_hash`) of its lowercase name
and every shader by the hash of its lowercase name:

- From 0x1C, 3,091 **parameter blocks** (the count, u32 at 0x18), back to back to
  0x3662EC, each a u64 size then six u64 offsets from the block's start. Between the second and third offsets
  are its textures, 16 bytes each: the texture's name hash, a slot (u32), 8 zero
  bytes. Between the third and fourth, its constants, 60 bytes each: the name (32
  bytes), its hash, a slot word, a count, and a default `Vec4`.
- At 0x367854, a count (4,154) and as many 48-byte records, then at 0x398338 the
  **programs**: a count (4,787) and as many 48-byte records from 0x39833C. A
  record is 16 bytes not worked out, then a GUID (the in-memory order, its first
  three fields little-endian) naming a bytecode res, `shaders/bytecode/<guid>`
  (type `0x0E2B02F7`; every one of the 4,787 is in the store), then 16 bytes
  (a small number, 1, 0, 1 in the first). The 4,154 records have the same shape
  and also name bytecode res; which programs they are is not worked out.
- At 0x3D0810, 57,340 **permutations** of 176 bytes (the count, u32 at 0x3D0808);
  the u32 at 17 × 4 bytes in is the permutation's parameter block, the u32 at 28 its
  program, an index into the programs (the largest is 4,786). The 5.5 KB between the
  blocks and the first table (from a count of 65, then what look like packed u16s)
  and the 828 bytes after the programs are not worked out, so nfsh takes the
  offsets as constants and checks the counts and every block and program index.
- At 0xD70550, the same 57,340 again, 32 bytes each, starting with the shader's name
  hash; a table at 0xF37BB0 lists each shader once (sorted by hash) with its first
  and last permutation.

A `__2d` shader's textures are its layers' diffuse, normal and SMD maps in slot
order, a group per layer in ascending layer order, with a texture shared by two
layers bound once (all 282 have one permutation, so one block): `46M_46V__2d` binds
`t_mega_grass_01` (layer 1), `t_field_base_03` (layer 2) and
`t_burnforestground_01`/`_02` (layer 6), and `t_vrg_wheelerdam_rock_02_n`, a detail
normal most of them bind. **A layer's own textures are those every `__2d` shader
with that layer binds, less those another layer accounts for**: a texture is dropped
when, in every shader with the layer, another of its layers binds it in all of that
layer's shaders (layer 6 is in 16 shaders, each with layer 1 or 4, so mega grass is
theirs, not 6's; the slot groups agree). 23 layers have their own textures: layer 1
is `t_mega_grass_01_d`, 2 `t_field_base_03_d`, 4 shares 1's, 8 `drv_set3_clean_d` and
`drv_set4_damage_d`, 9 `drv_set4_damage_d` and `drv_set1_full_damage_d`, 25
`t_gravel_test_01_d`; some layers have two diffuse maps (3: sand footprints and sand
waves; 13: forest slope and gravel). The road sets' `drv_…_d` maps are not plain
colour (`drv_set3_clean_d` averages a purple, 105, 57, 139): their channels hold
something else, not worked out. Layers 16, 17, 21 to 24, 27 and 31 to 38 have no
`__2d` shader (likely masks with nothing to draw); 30 has one but owns only the
shared detail normal.

**Tiling is not a constant**: no `__2d` block has a `Tiling` (other shaders do); a
layer's tiling, tint and how the layers blend are compiled into the shaders'
bytecode. nfsh reads the tiling from it (below, "Shader programs").

### Shader programs

Worked out on 2026-11-01; `frostbite::dxbc` reads a program as far as finding the
scales a terrain shader samples a texture at, and `frostbite::layers` each layer
diffuse's primary scale, which a store test checks.

**A bytecode res** is a 32-byte header (u32s 1 and 1, the container's size, its
offset, 0x20, then zeros) and a Direct3D shader container (`DXBC`): its chunk count
at 28 and their offsets after it, each chunk a tag and a size. The `__2d` shaders'
programs (one each, 282 different ones) are pixel shaders of shader
model 5 with no reflection chunk: `ISGN` (`SV_Position`, `TEXCOORD0` in v1),
`OSGN`, `SHEX`. Their code starts by mapping v1 to the world position with a `mad`
by a constant (`r0.xy`, x and z), and a texture's `t` register is its slot in the
parameter block.

**A layer's coordinates** are the world position times a literal (a `mul`, or a
`mad` adding a literal), or the position turned and scaled: one axis a `mad` adding
the other's product, the other a `dp2` with a literal pair, the scale being the
pair's length. A shader samples most layers several ways: one direct scale for
detail, often a far scale (a tenth of it, for the detail normal), a turned copy to
hide repeats. In `46M_46V__2d`, mega grass is `r0.yx × 0.2`; field_base_03 is
`r0.yx × 0.1` and the same turned (0.0924, 0.0383), length 0.1.

**A texture two layers share has a scale per layer**: mega grass is 0.2 for layer 1
and 0.25 (and turned 0.325) for layer 4, so nfsh goes by the scales a layer's diffuse
is sampled at in *every* shader with the layer: across the 282 shaders each layer
has one such set. **Its primary scale** is the finest direct one in that set, or the
finest turned one when there is none (the owner's choice, docs/DECISIONS.md): 0.2
per metre for mega grass (layer 1), 0.1 for field_base_03 (2), 0.1 and 0.15 for the
roads' `drv_set4_damage_d` in layers 8 and 9. Two have none: layer 28 binds
`t_terrain_desert_fine_gravel_02_d` but only shaders also with layer 5 or 29 sample
it, and layer 3's sand waves take their coordinates from something other than a
literal scale (not worked out).

## Meshes

A `RigidMeshAsset` or `CompositeMeshAsset` ebx names its geometry by res id in
`MeshSetResource`. The res, type `MeshSet` (`0x49B156D4`), describes the mesh: its
LODs, each LOD's sections (one material each) and their vertex layout. A LOD's
vertices and indices are in one chunk. `frostbite::mesh` reads both, and
`nfsh mesh STORE NAME [OBJ]` prints a mesh and writes its first LOD as an .obj.
Worked out on zone 495 (the speedway), then checked on every mesh set in the store
(see the end of this section). **Little-endian.**

The res is a block of records with pointers between them, then a relocation table.
Its 16 bytes of res metadata (in the bundle) give the layout: the block's size (u32),
4 zero bytes, the table's size (u32), then the header size and the section size (u16
each: 0xA0 or 0xB0, and 0xE0). The table lists, as u32 each, the offset of every
pointer in the block. A pointer is a u64 offset from the start of the res.

Header (0xA0 bytes; 0xB0 when the mesh has parts):

| offset | field |
|-------:|-------|
| 0x00, 0x10 | bounds: min and max, each 3 floats and 4 bytes of padding |
| 0x20 | LOD pointers, 7 slots (at most 6 used) |
| 0x58, 0x60 | name, name without its folders |
| 0x68 | the name's hash |
| 0x6C | kind: 0 rigid, 1 skinned (characters), 2 composite |
| 0x88, 0x8C | flags (not worked out) |
| 0x90, 0x92 | LOD count, section count over all LODs (u16) |

A LOD (0xA0 bytes):

| offset | field |
|-------:|-------|
| 0x00 | the LOD's kind (as the header's) |
| 0x08, 0x0C | section count (u32), pointer to its first section (sections are back to back) |
| 0x14 | five lists of section numbers (u32 count, u64 pointer to one byte each), apparently by how the sections are drawn; not needed to read the geometry |
| 0x50 | flags (not worked out) |
| 0x54 | index format: 0x21 for 16-bit indices, 0x2E for 32-bit (13 LODs, all large) |
| 0x58 | index buffer size in bytes: the indices, rounded up to 4 |
| 0x5C | vertex buffer size: where the last section's vertices end, rounded up to 16 |
| 0x60 | the chunk id, as a GUID in memory |
| 0x70 | 0xFFFFFFFF when the geometry is in the chunk; otherwise the chunk id is zero and this is an offset into geometry stored inline (only character meshes, not read yet) |
| 0x74 | name (`Mesh:<name>_lod0`), then two more pointers to parts of it |

A section (0xE0 bytes):

| offset | field |
|-------:|-------|
| 0x08 | name (the material's, often empty) |
| 0x10 | material: an index into the asset's `Materials` |
| 0x18, 0x1C | triangle count, first index (in indices, from the start of the index buffer) |
| 0x20, 0x24 | vertex offset (bytes from the start of the chunk), vertex count |
| 0x28 | vertex stride (u8, the streams' strides added up), primitive type (u8, always 3: triangle lists), then two bytes not worked out |
| 0x2C, 0x30 | a part count and a pointer, used by composite meshes |
| 0x40 | vertex elements: 16 slots of 4 bytes (usage, format, offset in its stream, stream); an empty slot is `00 00 FF 00` |
| 0x80 | each stream's bytes per vertex, 16 × u16 |
| 0xC0, 0xD0 | the section's bounds, as the header's |

**The chunk** holds the LOD's vertex buffer, then its index buffer at the vertex
buffer size; the chunk is their sizes added up, rounded up to 16. A section's
vertices are **one stream after another, not interleaved**: stream *k* starts at the
vertex offset plus the vertex count times the strides of the streams before it.
Indices count from the section's first vertex. The chunk id appears in three byte
orders: as a GUID in memory in the res, with its first three fields byte-swapped
(big-endian) in a bundle's chunk list, and with all 16 bytes reversed in a `.toc`'s.
A LOD's chunk is either in the bundle (most small LODs) or in a superbundle's own
chunks (`Win32/chunks0` and others).

Vertex elements seen (usage/format, bytes):

| usage | formats | what it is |
|------:|---------|------------|
| 0x01 | 0x03 (3 floats, 12), 0x08 (4 halves, 8; the fourth not used) | position, in the mesh's space |
| 0x21 | 0x06 (2 halves, 4), 0x02 (2 floats, 8) | texture coordinates, the first set: the colour texture's, and on roads the tiling layers' |
| 0x22-0x26 | the same | further texture coordinate sets: on roads, 0x22 maps the road's mask (`detailMask`, 0 to 1 across the mask) and 0x23 the shoulder (`Shoulder_NH`) |
| 0x1E | 0x0D (4 bytes) | a vertex colour, RGBA, a byte each: road lines are white (219 to 255) or yellow (255, 146, 19); on cracks and tyre marks only red varies, 0 to 255 (taken as their fade) |
| 0x34 | 0x0D (4 bytes) | the tangent frame: the normal and tangent, packed (below) |
| 0x02-0x08, 0x1F, 0x2A, 0x33, 0xBC, 0xD0 | 0x0C, 0x0D (4 bytes), 0x13 (4), 0x17, 0x19 (8), 0x03, 0x08 | not worked out: a second set of lightmap coordinates is likely among them; 0x1F is a second four-byte colour on roads; 0x2A (3 floats, the third 0) runs 0 to 1 across a road piece |

**The tangent frame** (0x34; worked out on 2026-11-01 from the meshes' faces and the
vertex programs that read it) is four bytes. The fourth is flags: bit 0, the sign of
the normal's largest component; bits 1 and 2, its axis (0: x, 1: y, 2: z, 3: x, the
other two components then in the order y, z rather than z, y); bit 3, the
bitangent's sign; bits 4 and 5, the high bits of the other two components; bits 6
and 7, the tangent angle's quarter turn. The other two components are 9 bits each,
the first byte (or the second) over 255 plus its high bit, times 1/√2, less 1/√2, so
from −1/√2 to 1/√2 (axis x: the first is z, the second y; y: x then z; z: x then y);
the largest is what makes the normal unit length. The tangent is a vector across the
normal, (−n.y, n.x, 0) when bit 1 is set and (n.z, 0, −n.x) otherwise, normalized and
turned about the normal by the angle (the third byte over 255, plus the quarter turns,
times π/2), and then taken the other way: it runs along the first texture coordinates'
u. The bitangent is the normal × the tangent, the other way when bit 3 is clear; it
runs along v decreasing. Checked on the whole city (2026-11-01): 32,314 sections have
it; every frame comes out orthonormal, and 93% of the vertices (28.5 of 30.5 million)
take a normal within 30° of their triangles' average, the rest mostly where a vertex
joins faces at an angle and the art chose its normal. The axis code 3 has not been
seen.

Sections with positions only and no name (usually the last one or two of a LOD) look
like shadow geometry: the same one often repeats in every LOD.

**Composite meshes** (`CompositeMeshAsset`, the breakable `brk` props such as trees and
poles) have the same geometry, with their parts already in place; the part data
(header 0xB0, the section's part count and list, a LOD pointer at 0x98) is not read
yet. Drawn whole, a composite looks as it does before anything breaks it.

Checked on the whole city, 2026-10-31: the store test reads every mesh the 232 zones
place (5,475 meshes; 14,112 LODs; 53,194 sections; 48,797,619 triangles), every
LOD's chunk and every section. Each chunk is exactly its two buffers rounded up to
16; each section's strides add up to its stride byte; every index is inside its
section; and every position is inside its section's bounds: exactly for floats,
within the rounding of a half (at most |x| / 2048; the bounds are those of the
positions before they were stored as halves, 103 sections touch them), except one
section of `prop_brk_abandoned_15_front_01_mesh`, in all three of its LODs, whose 15
vertices lie up to 2 cm outside (the stored bounds look slightly off). A survey of
all 31,034 mesh sets in the store (not only the placed ones) found the index format,
inline geometry and chunk byte orders described above, and no other index format.

## Textures

A `TextureAsset` ebx names its texture by res id in `Resource`. The res, type
`Texture` (`0x6BDE20BA`), is a fixed header naming the chunk that holds the pixels.
`frostbite::texture` reads both, and `nfsh texture STORE NAME [DDS]` prints a texture
and writes it as a .dds. Worked out on zone 495's textures, then checked on every
texture in the store and on every texture the placed meshes use (see the end of this
section). **Little-endian.**

A mesh asset's materials name their textures: each of its `Materials` (a
`MeshMaterial` instance in the same partition; a section's material indexes the list)
has a `Shader` struct whose `TextureParameters` array holds `Value` (a reference to
the texture asset's partition, or null) and `ParameterName` (`Diffuse`, `Normal`,
`SMD`, …).

The header (0x84 bytes; every texture res is exactly that, and its 16 bytes of res
metadata are 0x0C, then the flags below in byte 4, then zeros):

| offset | field |
|-------:|-------|
| 0x00, 0x04 | not worked out (zero for small textures) |
| 0x08 | kind: 0 flat, 1 cube, 2 volume, 3 array |
| 0x0C | format (table below) |
| 0x10 | usually 0; a hash on a few character textures (not worked out) |
| 0x14 | flags (u16: 0x1, 0x9 for UI textures, 0x20, 0x21; not worked out) |
| 0x16, 0x18 | width, height (u16) |
| 0x1A, 0x1C | depth and layer count (u16, equal in every texture): 1, 6 for a cube, the slices of an array or volume |
| 0x1E | mip count (u8, 1 to 15) |
| 0x1F | the first mips that only the whole chunk holds (u8; see below) |
| 0x20 | the chunk id, as a GUID in memory (byte orders as for meshes) |
| 0x30 | each mip's size in bytes for one layer, largest first: 15 × u32, zero past the mip count |
| 0x6C | their sum: one layer's chain |
| 0x70 | not worked out (close values for close group names, so not their hash) |
| 0x74 | the texture group, 16 bytes, zero-padded (`Props`, `Props_NSMD`, `Architecture`, …) |

Each mip is half the size of the one before (at least 1×1); a block-compressed mip
is its 4×4 blocks, at least one. **The chunk** holds every mip, largest first, and
each mip holds all its layers back to back (layer 1 of an array follows layer 0 of
the same mip; read layer-first, an array's layers come out scrambled). So an array's
or a cube's chunk is the layer count times the chain. A volume's mip sizes already
count its depth slices, which halve with each mip as the width and height do, so its
chunk is the chain itself (a 32×32×32 BC3 volume is one mip of 32,768 bytes). The whole chunk is in a superbundle's chunks or,
for a texture loaded all at once, a bundle's chunk entry from offset 0. A bundle
whose texture streams lists the chunk from the first mip after the streamed ones
(byte 0x1F), to the end: concrete 1024×1024 BC1 with four streamed mips is listed
from 0xAA000 (the first four mips' sizes) for 0xAB8 bytes.

Formats, from each mip's size (bytes per block), from decoding a texture of each (an
image that makes sense one way and noise the other), and from the names of the
textures that use them (`_d` diffuse in the sRGB ones):

| code | format | block |
|-----:|--------|------:|
| 0x36, 0x37 | BC1, unorm and sRGB | 8 |
| 0x38, 0x39 | BC1 with 1-bit alpha, unorm and sRGB | 8 |
| 0x3C, 0x3D | BC3, unorm and sRGB | 16 |
| 0x3E | BC4 unorm | 8 |
| 0x3F | BC5 unorm | 16 |
| 0x40 | BC6H unsigned float | 16 |
| 0x42, 0x43 | BC7, unorm and sRGB | 16 |

Seen but not worked out: 0x3A (one test texture, 16-byte blocks: BC2 by the
pattern), and the uncompressed 0x06, 0x12, 0x14, 0x1B, 0x29, 0x33 (look-up tables,
gradients, noise: about 70 textures, none on a placed mesh).

Checked on the whole city, 2026-10-31: the store test reads every texture the
materials of the 232 zones' meshes name (14,806 materials, 37,345 texture
parameters, 8,015 textures, 4.7 GiB of pixels), every header and every whole chunk,
which is always exactly its mips; and for the 7,010 whose chunk a bundle also lists
from a later offset, that the offset is where the streamed mips end, the entry runs
to the chunk's end, and its bytes are the whole chunk's. All 8,015 are flat and in a
format above. Every one of the 32,394 texture res in the store parses by these rules
(32,333 flat, 37 cubes, 8 arrays, 16 volumes), every mip of a format above having
the size its blocks give.

## Lighting

A time of day's look is a **visual environment**: a partition with a
`VisualEnvironmentBlueprint` and one instance per component (`OutdoorLightComponentData`,
`SkyComponentData`, `TonemapComponentData`, `FogComponentData`, `ColorCorrectionComponentData`,
`EnlightenComponentData`, …). Each component lists the fields it sets in
`PropertyOverrides`; the rest keep their defaults. `frostbite::lighting` reads the sun,
the sky's colours and the exposure.

Palm City's are in `levels/mainlevel/lighting/visualenvironments/`, in bundles named
by time of day:

| bundle | visual environments |
|--------|--------|
| `tod_day3` | `lighting_mainlevel_day_02v03_neutral_ve` (midday), the midday tunnels |
| `tod_day6` | `…_day_05_morning_neutral_ve`, the sunset tunnels |
| `tod_day9` | `…_day_04_overcast_neutral_ve`, the overcast tunnels |
| `tod_night3` | `lighting_mainlevel_twilight_02_neutral_ve`, the tealnight tunnels and radiosity |

The fields read, at midday and at night (`twilight_02`):

| component, field | midday | night | meaning |
|------------------|-------:|------:|---------|
| OutdoorLight `SunIntensity` | 170,000 | 6 | the sun's (moon's) illuminance, lux |
| OutdoorLight `SunColor` | (0.99, 0.78, 0.60) | (1, 1, 1) | linear RGB |
| OutdoorLight `SunRotationX`, `SunRotationY` | 153.1, 34.7 | 229.6, 90 | degrees: bearing and elevation, by the values (the bearing's axis is not checked) |
| OutdoorLight `SkyColor`, `GroundColor` | (0.318, 0.341, 0.4), 0.05 grey | the same | not overridden: the defaults |
| Sky `LuminanceScale` | 983 | 30 | the sky's brightness, in units not worked out |
| Tonemap `MinEV`, `MaxEV` | 14.25, 15 | 0.75, 2.25 | the automatic exposure's range, EV at ISO 100 |
| Tonemap `ExposureCompensation` | 1.1 | -1.1 | stops |
| Tonemap `TonemapMethod` | `FilmicNeutral` | `FilmicNeutral` | |

Each zone's `worldart_zs_<n>_midday` (and `_tealnight`, …) bundle holds the zone's
precomputed indirect light for that time (a `LayerReferenceObjectData`, a
`GiGridAssetSelectorData`, Enlighten data); not read yet.

## nfsh's store (our own format)

What `nfsh import` writes: a folder with an index and a few packs.

```
index.nfsh        the superbundles, their bundles and chunks (below)
pack_NN.nfsh      stored entries, back to back, NN from 00
```

A pack holds each distinct entry the patched tables point at once, in archive order,
as one [zstd](https://www.rfc-editor.org/rfc/rfc8878) frame of the entry's decompressed
bytes (what Heat's block chain decompresses to, see "Stored blocks"): level 19, with
the decompressed size and the content checksum in the frame. ebx frames are compressed
with the index's dictionary and name its id in their header; the others use none. A
location covers exactly one frame. A pack is closed when the next frame would take it
past 1 GiB. The `Data/` bytes the patch replaced are not copied.

The index is big-endian like Heat's tables. A string or byte run is a u32 length and
the bytes; a list is a u32 count and the items; a location is the pack (u8), the
offset in it and the size (u32 each), and must lie inside the pack.

| field | |
|-------|--|
| magic, version | `NFSHSTOR`, u32 3 (version 2 had no partition GUIDs and is still read; version 1 kept Heat's Oodle blocks) |
| packs | list of u64 sizes (checked against the files on open) |
| dictionary | byte run: the ebx frames' zstd dictionary, empty for none |
| superbundles | list: name, bundles, chunks |
| bundle | name, layer (u8: 0 `Data/`, 1 `Patch/`), metadata DbObject as stored, ebx list, res list, chunk list |
| ebx | name, SHA-1, decompressed size (u32), location |
| res | as ebx, then type hash (u32), metadata (16 bytes), id (u64) |
| bundle chunk | id (16 bytes), SHA-1, logical offset, logical size (u32), location |
| superbundle chunk | id, layer, location |
| partitions | after the superbundles: list of each ebx's partition GUID (16 bytes, from its header), in the order the superbundles list their ebx; empty for none, else one per ebx |

Nothing may follow the partition list. The import lists the GUIDs at the end, by
reading every ebx back from the packs; `nfsh reindex` does the same for a version 2
store, in about 90 seconds. The index is written after the packs, under
a temporary name renamed at the end, so a store without `index.nfsh` is incomplete.
