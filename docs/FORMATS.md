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

## nfsh's store (our own format)

What `nfsh import` writes: a folder with an index and a few packs.

```
index.nfsh        the superbundles, their bundles and chunks (below)
pack_NN.nfsh      stored entries, back to back, NN from 00
```

A pack holds each distinct entry the patched tables point at once, in archive order,
byte for byte as in the cas archive (Heat's block chains, see "Stored blocks"). A pack
is closed when the next entry would take it past 1 GiB. The `Data/` bytes the patch
replaced are not copied.

The index is big-endian like Heat's tables. A string or byte run is a u32 length and
the bytes; a list is a u32 count and the items; a location is the pack (u8), the
offset in it and the size (u32 each), and must lie inside the pack.

| field | |
|-------|--|
| magic, version | `NFSHSTOR`, u32 1 |
| packs | list of u64 sizes (checked against the files on open) |
| superbundles | list: name, bundles, chunks |
| bundle | name, layer (u8: 0 `Data/`, 1 `Patch/`), metadata DbObject as stored, ebx list, res list, chunk list |
| ebx | name, SHA-1, decompressed size (u32), location |
| res | as ebx, then type hash (u32), metadata (16 bytes), id (u64) |
| bundle chunk | id (16 bytes), SHA-1, logical offset, logical size (u32), location |
| superbundle chunk | id, layer, location |

Nothing may follow the last superbundle. The index is written after the packs, under
a temporary name renamed at the end, so a store without `index.nfsh` is incomplete.
