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
that is the last small mips of the chain; where the rest is stored is not confirmed
yet. All 1,317,687 distinct entries of the owner's install (31.5 GiB stored, 65.4 GiB
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

How the drawable world hangs together, as far as read:

- `levels/mainlevel/staticworld` is a `SubWorldData` with 8 sub-world and 3 layer
  references.
- The world is 232 streaming zones, `worldart_zs_<n>`, each its own bundle. A zone's
  root partition is a `SubWorldData` with its `StaticModelGroupEntityData` (the placed
  static meshes), physics and `ZoneStreamerSubWorldRod`s; its `_layer` partition holds
  the placed objects (`ObjectReferenceObjectData`), lights, reflection volumes and
  effects.
- Every zone has four lighting variants, `worldart_zs_<n>_midday`, `_overcast`,
  `_sunset` and `_tealnight`: each a small bundle with the zone's precomputed global
  illumination (`StaticEnlightenEntityData`, a GI grid, and `worldart_radiosity/`
  textures).
- Meshes are `RigidMeshAsset` ebx whose geometry is a res and chunks; textures are
  `TextureAsset` ebx with a `Texture` res (the header) and a chunk (the pixels).
- The terrain is `mainlevel_terrain_2048`, in the root bundle.

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
| magic, version | `NFSHSTOR`, u32 2 (version 1 kept Heat's Oodle blocks) |
| packs | list of u64 sizes (checked against the files on open) |
| dictionary | byte run: the ebx frames' zstd dictionary, empty for none |
| superbundles | list: name, bundles, chunks |
| bundle | name, layer (u8: 0 `Data/`, 1 `Patch/`), metadata DbObject as stored, ebx list, res list, chunk list |
| ebx | name, SHA-1, decompressed size (u32), location |
| res | as ebx, then type hash (u32), metadata (16 bytes), id (u64) |
| bundle chunk | id (16 bytes), SHA-1, logical offset, logical size (u32), location |
| superbundle chunk | id, layer, location |

Nothing may follow the last superbundle. The index is written after the packs, under
a temporary name renamed at the end, so a store without `index.nfsh` is incomplete.
