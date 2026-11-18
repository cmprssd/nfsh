//! Palm City's terrain: the streaming tree's quadtree of tiles, its blocks (layer tree,
//! outlines, surface map), a tile's heightfield,
//! height bounds, pages and colour, its finer chunk, and a mesh of the ground under an
//! area (see docs/FORMATS.md, "Terrain").
//! Little-endian, packed with no alignment.

use std::collections::HashMap;

use crate::reader::Reader;
use crate::store::{Store, guid_order};
use crate::texture::bc1_palette;
use crate::{Error, Location, Result, bail};

/// The res holding Palm City's streaming tree (in the terrain's root bundle).
pub const STREAMING_TREE: &str =
    "levels/mainlevel/mainlevel_terrain_2048/mainlevel_terrain_2048.streamingtree_win32";

/// The header before the node tree; the counts in it that are checked.
const HEADER: usize = 0x7F;
const TILE_COUNT_AT: usize = 0x15;
const NODE_COUNT_AT: usize = 0x6B;
/// A node's bytes: its box and an f32, then the byte that marks it empty.
const NODE: usize = 28;
/// The min/max height pyramid the root and the 8,192 m nodes carry.
const PYRAMID: usize = 340;
/// The byte that ends the blocks between the node tree and the tiles.
const LAST_BLOCK: u8 = 0xFF;
/// The block holding the layer tree, and its header's size.
const LAYER_BLOCK: u8 = 1;
const LAYER_HEADER: usize = 0x38;
/// The blocks holding outlines, quadtrees with no data of their own: kind, header
/// size, where its square and node count are, and the flag bytes before a node's
/// children byte.
const OUTLINE_BLOCKS: [(u8, OutlineLayout); 2] = [
    (
        2,
        OutlineLayout {
            header: 0x34,
            square_at: 0x14,
            count_at: 0x2C,
            flags: 3,
        },
    ),
    (
        4,
        OutlineLayout {
            header: 0x28,
            square_at: 0x08,
            count_at: 0x18,
            flags: 2,
        },
    ),
];
/// The block holding the surface map, its header's size, and where its counts are.
const SURFACE_BLOCK: u8 = 3;
const SURFACE_HEADER: usize = 0x24;
const SURFACE_NODES_AT: usize = 0x18;
const SURFACE_MAPS_AT: usize = 0x1C;
/// A surface value is 4 bits, an index into a table of at most this many entries.
pub const SURFACE_KINDS: usize = 16;
/// Deeper than any tree the readers accept (Palm City's goes 9 levels down, to 64 m),
/// so a malformed file can't recurse without end.
const MAX_DEPTH: usize = 16;

/// A heightfield's samples along each side, its border on each side, and the cells
/// between the border's inner samples.
pub const SAMPLES: usize = 137;
pub const BORDER: usize = 4;
pub const CELLS: usize = 128;
/// A heightfield chunk's size is `HEIGHTFIELD + PAGES·k` bytes; any other tile's,
/// `COLOUR_BYTES + PAGES·k`.
const HEIGHTFIELD: u32 = 42_231;
const PAGES: u32 = 18_496;
/// What follows the heights in a heightfield chunk: [`BOUNDS`] min/max pairs of u16,
/// the coarse heights ([`COARSE_SIDES`]), then 1,225 bytes not worked out (see
/// docs/FORMATS.md).
const AFTER_HEIGHTS: usize = 2_381;
/// The height bounds: the tile, then its 4, 16 and 64 blocks, each row by row.
pub const BOUNDS: usize = 1 + 4 + 16 + 64;
/// The sides of the coarse heights' grids, coarsest first: a point every `CELLS /
/// (side - 1)` samples from the border's inner edge, the last level every 8.
pub const COARSE_SIDES: [usize; 5] = [2, 3, 5, 9, 17];
/// A heightfield in a finer chunk: the heights and what follows them, no colour.
const FINER_HEIGHTS: usize = 2 * SAMPLES * SAMPLES + AFTER_HEIGHTS;
/// The order a finer chunk stores its quadrants in, as indices into [`QUADRANTS`]:
/// the reverse of theirs.
const FINER_ORDER: [usize; 4] = [3, 2, 1, 0];
/// A tile's colour: the last bytes of its chunk, BC1 blocks of `COLOUR_SIDE` ×
/// `COLOUR_SIDE` texels with a `COLOUR_BORDER` beyond the tile on each side.
pub const COLOUR_SIDE: usize = 68;
pub const COLOUR_BORDER: usize = 2;
const COLOUR_BYTES: usize = COLOUR_SIDE * COLOUR_SIDE / 2;
/// A page: one layer's weight, 0 to 255, `PAGE_SIDE` × `PAGE_SIDE` texels with a
/// `BORDER` beyond the tile on each side; the pages lie just before the colour.
pub const PAGE_SIDE: usize = 136;
/// How many layers a ground vertex carries weights for.
pub const DETAIL_SLOTS: usize = 8;
/// Metres per height unit.
const UNIT: f32 = 1.0 / 32.0;

/// Where a tile's four children lie, in the order they follow it: x then z, in halves.
pub const QUADRANTS: [[u32; 2]; 4] = [[0, 0], [1, 0], [1, 1], [0, 1]];

/// One square of the tiles' quadtree.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Its least x and z.
    pub min: [f32; 2],
    pub size: f32,
    /// Its first chunk's id in the order a bundle's chunk list stores it, and size;
    /// `None` for the root and the first 8,192 m quadrant, which have none.
    pub chunk: Option<([u8; 16], u32)>,
    /// Its finer chunk's id (in the same order) and size, when it has one: its four
    /// quadrants at twice its detail ([`Finer`]).
    pub finer: Option<([u8; 16], u32)>,
    /// Its children's indices, in [`QUADRANTS`] order.
    pub children: Option<[u32; 4]>,
}

impl Tile {
    /// Whether its first chunk is a heightfield.
    pub fn has_heights(&self) -> bool {
        self.chunk.is_some_and(|(_, size)| {
            size >= HEIGHTFIELD && (size - HEIGHTFIELD).is_multiple_of(PAGES)
        })
    }

    /// Whether its chunk has one of the two sizes known, so ends in its colour.
    pub fn has_colour(&self) -> bool {
        self.has_heights()
            || self.chunk.is_some_and(|(_, size)| {
                size >= COLOUR_BYTES as u32 && (size - COLOUR_BYTES as u32).is_multiple_of(PAGES)
            })
    }

    /// How many pages its chunk holds: 0 without a chunk of a known size.
    pub fn page_count(&self) -> usize {
        let Some((_, size)) = self.chunk.filter(|_| self.has_colour()) else {
            return 0;
        };
        let base = if self.has_heights() {
            HEIGHTFIELD
        } else {
            COLOUR_BYTES as u32
        };
        ((size - base) / PAGES) as usize
    }

    fn overlaps(&self, lo: [f32; 2], hi: [f32; 2]) -> bool {
        (0..2).all(|k| self.min[k] < hi[k] && lo[k] < self.min[k] + self.size)
    }
}

/// A layer's record at one square of the layer tree.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerRecord {
    /// The layer index, as in the shader names and the scattering (not a place in the
    /// `TerrainData`'s `TerrainLayers`; see docs/FORMATS.md, "Scattering").
    pub layer: u16,
    /// Whether a tile at this square has a page (a weight mask) for the layer.
    pub paged: bool,
    /// Whether no square below lists the layer.
    pub leaf: bool,
    /// The cells of the square the layer touches, as a pyramid of bits; read them
    /// with [`LayerRecord::touches`].
    pub cells: [u8; 11],
}

/// The levels of a record's cell pyramid: 1, 2, 4 and 8 cells a side.
pub const CELL_LEVELS: usize = 4;

impl LayerRecord {
    /// Whether the layer touches cell (`row`, `column`) of `level` (0 to 3, with
    /// 2^level cells a side, rows along z, as the pages): bit (4^level − 1)/3 + row ×
    /// 2^level + column of `cells`, least significant first. A set bit's cell, grown by
    /// the pages' 4-texel border, has weight in the layer's page; false outside the
    /// pyramid.
    pub fn touches(&self, level: usize, row: usize, column: usize) -> bool {
        let n = 1 << level.min(CELL_LEVELS);
        if level >= CELL_LEVELS || row >= n || column >= n {
            return false;
        }
        let bit = (n * n - 1) / 3 + row * n + column;
        self.cells[bit / 8] >> (bit % 8) & 1 == 1
    }
}

/// Where an outline block keeps its parts (see [`OUTLINE_BLOCKS`]).
struct OutlineLayout {
    header: usize,
    square_at: usize,
    count_at: usize,
    flags: usize,
}

/// A block's quadtree with no data of its own (kinds 2 and 4): over the world's square,
/// in the tiles' child order. What it stands for is not worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// The block's kind.
    pub kind: u8,
    /// The block's first u32: 68 for kind 2 (a colour's side), 137 for kind 4 (a
    /// heightfield's).
    pub side: u32,
    pub nodes: usize,
    /// Its leaves' least x and z and side, depth first.
    pub leaves: Vec<([f32; 2], f32)>,
}

/// The surface map (block kind 3): 4-bit values in a grid over the land, each an index
/// into [`Surfaces::table`] of the ground's physics materials (docs/FORMATS.md,
/// "Terrain").
#[derive(Debug, Clone, PartialEq)]
pub struct Surfaces {
    /// Values along each side of a map: 256.
    pub side: usize,
    /// The quadtree's node count.
    pub nodes: usize,
    /// The squares that have a map, by [`square`].
    pub maps: HashMap<[u32; 3], SurfaceMap>,
    /// Each value's physics material, packed as the level's material grid packs one
    /// (`MaterialDecl`): 0x80 in the low byte, the material in the next ([`material`]),
    /// four zero bits, and a modifier in the top 12 bits ([`property`]).
    pub table: Vec<u32>,
    /// The material where the map has none, packed the same way: the material grid's
    /// `DefaultMaterial` (0x80, material 0).
    pub default: u32,
}

/// The material of a packed `MaterialDecl`: its index into the material grid's
/// `MaterialIndexMap`, which gives its row of properties.
pub fn material(packed: u32) -> u8 {
    (packed >> 8) as u8
}

/// The modifier of a packed `MaterialDecl` (its top 12 bits; 0 for none): another
/// index into `MaterialIndexMap`, for a row with no sounds of its own. On the ground
/// it is always a row holding race physics alone (its `SurfaceType`).
pub fn property(packed: u32) -> u16 {
    (packed >> 20) as u16
}

/// One square's surface values, kept as stored: each row run-length coded.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceMap {
    pub min: [f32; 2],
    pub size: f32,
    rows: Vec<u8>,
    lengths: Vec<u16>,
}

impl SurfaceMap {
    /// Its values, `side` × `side` (the [`Surfaces::side`]), rows along z and columns
    /// along x, value (r, c) covering from (min x + c·size/side, min z + r·size/side).
    /// A row is run-length coded bytes, two values each, the low 4 bits first: two equal
    /// bytes and a count n stand for n + 1 of that byte, any other byte for itself.
    pub fn values(&self, side: usize) -> Result<Vec<u8>> {
        if self.lengths.len() != side {
            bail!("{} rows, {side} expected", self.lengths.len());
        }
        let mut out = Vec::with_capacity(side * side);
        let mut row = Vec::with_capacity(side / 2);
        let mut at = 0;
        for (r, &len) in self.lengths.iter().enumerate() {
            let src = self
                .rows
                .get(at..at + usize::from(len))
                .ok_or_else(|| Error(format!("row {r} runs past the map")))?;
            at += usize::from(len);
            row.clear();
            let mut i = 0;
            while i < src.len() {
                if src.get(i + 1) == Some(&src[i]) {
                    let n = *src
                        .get(i + 2)
                        .ok_or_else(|| Error(format!("row {r}: a run with no count")))?;
                    row.resize(row.len() + usize::from(n) + 1, src[i]);
                    i += 3;
                } else {
                    row.push(src[i]);
                    i += 1;
                }
            }
            if row.len() * 2 != side {
                bail!("row {r} holds {} values, {side} expected", row.len() * 2);
            }
            out.extend(row.iter().flat_map(|b| [b & 0xF, b >> 4]));
        }
        Ok(out)
    }
}

/// The streaming tree's tiles; the first is the root.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub tiles: Vec<Tile>,
    /// The node tree's node count.
    pub nodes: usize,
    /// The layer tree: each square's layers, in ascending order, by [`square`].
    pub layers: HashMap<[u32; 3], Vec<LayerRecord>>,
    /// The blocks with outlines, kinds 2 and 4, in block order.
    pub outlines: Vec<Outline>,
    /// The surface map, when the tree has one.
    pub surfaces: Option<Surfaces>,
}

/// A square's key in [`Tree::layers`]: its least x and z and its side, as bits.
pub fn square(min: [f32; 2], size: f32) -> [u32; 3] {
    [min[0].to_bits(), min[1].to_bits(), size.to_bits()]
}

impl Tree {
    /// Reads a streaming-tree res.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let header = Reader::at(data, 0)?.bytes(HEADER)?;
        let count = |at: usize| -> usize {
            u32::from_le_bytes(header[at..at + 4].try_into().expect("4 bytes")) as usize
        };
        let (tile_count, node_count) = (count(TILE_COUNT_AT), count(NODE_COUNT_AT));
        let mut r = Reader::at(data, HEADER)?;
        let mut nodes = 0;
        let (min, size) = root_box(&mut r)?;
        r = Reader::at(data, HEADER)?;
        skip_node(&mut r, 0, &mut nodes)?;
        if nodes != node_count {
            bail!("the node tree has {nodes} nodes, its header says {node_count}");
        }
        let mut layers = HashMap::new();
        let mut outlines = Vec::new();
        let mut surfaces = None;
        loop {
            let kind = r.u8()?;
            if kind == LAST_BLOCK {
                break;
            }
            let len = r.u32_le()? as usize;
            let block = r.bytes(len)?;
            let in_block = |e: Error| Error(format!("block {kind}: {e}"));
            if kind == LAYER_BLOCK {
                layers = read_layers(block).map_err(|e| Error(format!("the layer tree: {e}")))?;
            } else if kind == SURFACE_BLOCK {
                surfaces = Some(read_surfaces(block).map_err(in_block)?);
            } else if let Some((_, layout)) = OUTLINE_BLOCKS.iter().find(|(k, _)| *k == kind) {
                outlines.push(read_outline(kind, block, layout).map_err(in_block)?);
            }
        }
        let mut tiles = Vec::with_capacity(tile_count.min(data.len() / 23));
        read_tile(&mut r, min, size, 0, &mut tiles)?;
        if r.pos() != data.len() {
            bail!(
                "the tiles end at {:#x}, {} bytes before the end",
                r.pos(),
                data.len() - r.pos()
            );
        }
        if tiles.len() != tile_count {
            bail!("{} tiles, the header says {tile_count}", tiles.len());
        }
        Ok(Self {
            tiles,
            nodes,
            layers,
            outlines,
            surfaces,
        })
    }

    /// The layers of `tile`'s pages, in page order: the paged layers at its square,
    /// ascending (docs/FORMATS.md, "Terrain"). Empty when no node of
    /// the layer tree is at its square: such a tile has no pages.
    pub fn pages(&self, tile: &Tile) -> Vec<u16> {
        self.layers
            .get(&square(tile.min, tile.size))
            .map(|records| {
                records
                    .iter()
                    .filter(|r| r.paged)
                    .map(|r| r.layer)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The layers of the pages in `tile`'s finer chunk, for each of its quadrants in
    /// [`QUADRANTS`] order: the paged layers at the quadrant's square, ascending.
    pub fn finer_pages(&self, tile: &Tile) -> [Vec<u16>; 4] {
        let half = tile.size / 2.0;
        QUADRANTS.map(|[qx, qz]| {
            let min = [
                tile.min[0] + qx as f32 * half,
                tile.min[1] + qz as f32 * half,
            ];
            self.pages(&Tile {
                min,
                size: half,
                chunk: None,
                finer: None,
                children: None,
            })
        })
    }

    /// Reads Palm City's tree from `store`.
    pub fn read(store: &Store) -> Result<Self> {
        let asset = store
            .res(STREAMING_TREE)
            .ok_or_else(|| Error(format!("{STREAMING_TREE}: not in the store")))?;
        let data = store.entries()?.read(asset.location)?;
        Self::parse(&data).map_err(|e| Error(format!("{STREAMING_TREE}: {e}")))
    }
}

fn f32_le(r: &mut Reader) -> Result<f32> {
    Ok(f32::from_bits(r.u32_le()?))
}

/// The root node's least x and z and its side, which must be square.
fn root_box(r: &mut Reader) -> Result<([f32; 2], f32)> {
    let mut b = [0.0; 6];
    for v in &mut b {
        *v = f32_le(r)?;
    }
    let size = b[3] - b[0];
    if !(size > 0.0 && size.is_finite() && b[5] - b[2] == size) {
        bail!("the root's box {b:?} is not a square");
    }
    Ok(([b[0], b[2]], size))
}

/// Skips the node at `r` and its children, counting them.
fn skip_node(r: &mut Reader, depth: usize, count: &mut usize) -> Result<()> {
    if depth > MAX_DEPTH {
        bail!("the node tree is deeper than {MAX_DEPTH}");
    }
    *count += 1;
    r.bytes(NODE)?;
    match r.u8()? {
        1 => return Ok(()),
        0 => {}
        e => bail!("node {count}: empty flag {e}"),
    }
    let flags = r.bytes(4)?;
    if flags[0] == 1 {
        r.bytes(PYRAMID)?;
    }
    match r.u8()? {
        0 => Ok(()),
        1 => (0..4).try_for_each(|_| skip_node(r, depth + 1, count)),
        c => bail!("node {count}: children flag {c}"),
    }
}

/// Reads the layer tree, the block of kind [`LAYER_BLOCK`]: a header, then a quadtree
/// of nodes over the header's square, each listing the layers under it.
fn read_layers(block: &[u8]) -> Result<HashMap<[u32; 3], Vec<LayerRecord>>> {
    let header = Reader::at(block, 0)?.bytes(LAYER_HEADER)?;
    let u32_at = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().expect("4 bytes"));
    let f32_at = |at: usize| f32::from_bits(u32_at(at));
    let (min, max) = ([f32_at(0x18), f32_at(0x1C)], [f32_at(0x20), f32_at(0x24)]);
    let size = max[0] - min[0];
    if !(size > 0.0 && size.is_finite() && max[1] - min[1] == size) {
        bail!("the square {min:?} to {max:?} is not a square");
    }
    let (node_count, record_count) = (u32_at(0x28) as usize, u32_at(0x2C) as usize);
    let mut r = Reader::at(block, LAYER_HEADER)?;
    let mut layers = HashMap::new();
    let mut nodes = 0;
    read_layer_node(&mut r, min, size, 0, &mut nodes, &mut layers)?;
    let records: usize = layers.values().map(Vec::len).sum();
    if (nodes, records) != (node_count, record_count) {
        bail!(
            "{nodes} nodes and {records} records, the header says {node_count} and {record_count}"
        );
    }
    if r.pos() != block.len() {
        bail!("{} bytes after the last node", block.len() - r.pos());
    }
    Ok(layers)
}

/// Reads the layer node at `r` covering `min` to `min + size`, and its children.
fn read_layer_node(
    r: &mut Reader,
    min: [f32; 2],
    size: f32,
    depth: usize,
    nodes: &mut usize,
    layers: &mut HashMap<[u32; 3], Vec<LayerRecord>>,
) -> Result<()> {
    if depth > MAX_DEPTH {
        bail!("deeper than {MAX_DEPTH}");
    }
    *nodes += 1;
    let _zero = r.u8()?;
    let count = r.u16_le()?;
    let paged = r.u16_le()?;
    let _zero = r.u16_le()?;
    // A node with no records is empty: no records, no children, nothing more.
    if count == 0 {
        return Ok(());
    }
    let flag = |b: u8, what: &str| match b {
        0 | 1 => Ok(b == 1),
        _ => Err(Error(format!("node {nodes}: {what} flag {b}"))),
    };
    let mut records: Vec<LayerRecord> = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let layer = r.u16_le()?;
        let mut at = [0.0; 4];
        for v in &mut at {
            *v = f32_le(r)?;
        }
        if at != [min[0], min[1], min[0] + size, min[1] + size] {
            bail!("node {nodes}: a record's square {at:?}, the node's is {min:?} + {size}");
        }
        if records.last().is_some_and(|last| last.layer >= layer) {
            bail!("node {nodes}: layers not ascending");
        }
        let leaf = flag(r.u8()?, "leaf")?;
        let unpaged = flag(r.u8()?, "page")?;
        records.push(LayerRecord {
            layer,
            paged: !unpaged,
            leaf,
            cells: r.array()?,
        });
    }
    if records.iter().filter(|r| r.paged).count() != usize::from(paged) {
        bail!("node {nodes}: the paged records don't match its count {paged}");
    }
    layers.insert(square(min, size), records);
    let _zero = r.u8()?;
    match r.u8()? {
        0 => Ok(()),
        1 => QUADRANTS.iter().try_for_each(|[qx, qz]| {
            let at = [
                min[0] + *qx as f32 * size / 2.0,
                min[1] + *qz as f32 * size / 2.0,
            ];
            read_layer_node(r, at, size / 2.0, depth + 1, nodes, layers)
        }),
        c => bail!("node {nodes}: children flag {c}"),
    }
}

/// The square at `at` in a block's header (four f32: least x and z, then greatest).
fn block_square(block: &[u8], at: usize) -> Result<([f32; 2], f32)> {
    let mut r = Reader::at(block, at)?;
    let mut b = [0.0; 4];
    for v in &mut b {
        *v = f32_le(&mut r)?;
    }
    let size = b[2] - b[0];
    if !(size > 0.0 && size.is_finite() && b[3] - b[1] == size) {
        bail!("the square {b:?} is not a square");
    }
    Ok(([b[0], b[1]], size))
}

/// A u32 at `at` in `block`.
fn block_u32(block: &[u8], at: usize) -> Result<u32> {
    Reader::at(block, at)?.u32_le()
}

/// A byte that must be 0 or 1.
fn flag(r: &mut Reader, what: &str) -> Result<bool> {
    match r.u8()? {
        b @ (0 | 1) => Ok(b == 1),
        b => bail!("{what} flag {b} at {:#x}", r.pos() - 1),
    }
}

/// The child squares of `min` + `size`, in [`QUADRANTS`] order.
fn quadrants(min: [f32; 2], size: f32) -> [[f32; 2]; 4] {
    QUADRANTS.map(|[qx, qz]| {
        [
            min[0] + qx as f32 * size / 2.0,
            min[1] + qz as f32 * size / 2.0,
        ]
    })
}

/// Reads an outline block: a header, then a quadtree whose nodes are flag bytes
/// and a byte that is 1 when four children follow.
fn read_outline(kind: u8, block: &[u8], layout: &OutlineLayout) -> Result<Outline> {
    let side = block_u32(block, 0)?;
    let (min, size) = block_square(block, layout.square_at)?;
    let count = block_u32(block, layout.count_at)? as usize;
    let mut r = Reader::at(block, layout.header)?;
    let mut outline = Outline {
        kind,
        side,
        nodes: 0,
        leaves: Vec::new(),
    };
    fn node(
        r: &mut Reader,
        flags: usize,
        min: [f32; 2],
        size: f32,
        depth: usize,
        out: &mut Outline,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            bail!("deeper than {MAX_DEPTH}");
        }
        out.nodes += 1;
        for _ in 0..flags {
            flag(r, "node")?;
        }
        if flag(r, "children")? {
            quadrants(min, size)
                .into_iter()
                .try_for_each(|at| node(r, flags, at, size / 2.0, depth + 1, out))
        } else {
            out.leaves.push((min, size));
            Ok(())
        }
    }
    node(&mut r, layout.flags, min, size, 0, &mut outline)?;
    if outline.nodes != count {
        bail!("{} nodes, the header says {count}", outline.nodes);
    }
    if r.pos() != block.len() {
        bail!("{} bytes after the last node", block.len() - r.pos());
    }
    Ok(outline)
}

/// Reads the surface map's block: a header, a quadtree whose nodes may hold a map,
/// then the values' table.
fn read_surfaces(block: &[u8]) -> Result<Surfaces> {
    let side = block_u32(block, 0)? as usize;
    if side == 0 || !side.is_multiple_of(2) || side > 4096 {
        bail!("a map side of {side}");
    }
    let (min, size) = block_square(block, 0x08)?;
    let node_count = block_u32(block, SURFACE_NODES_AT)? as usize;
    let map_count = block_u32(block, SURFACE_MAPS_AT)? as usize;
    let mut r = Reader::at(block, SURFACE_HEADER)?;
    let mut surfaces = Surfaces {
        side,
        nodes: 0,
        maps: HashMap::new(),
        table: Vec::new(),
        default: 0,
    };
    fn node(
        r: &mut Reader,
        min: [f32; 2],
        size: f32,
        depth: usize,
        out: &mut Surfaces,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            bail!("deeper than {MAX_DEPTH}");
        }
        out.nodes += 1;
        let has_map = flag(r, "map")?;
        let _unknown = flag(r, "second")?;
        if has_map {
            let len = r.u32_le()? as usize;
            let rows = r.bytes(len)?.to_vec();
            let lengths = (0..out.side)
                .map(|_| r.u16_le())
                .collect::<Result<Vec<_>>>()?;
            let total: usize = lengths.iter().map(|&l| usize::from(l)).sum();
            if total != len {
                bail!(
                    "node {}: its rows' lengths sum to {total}, not {len}",
                    out.nodes
                );
            }
            let map = SurfaceMap {
                min,
                size,
                rows,
                lengths,
            };
            out.maps.insert(square(min, size), map);
        }
        if flag(r, "children")? {
            quadrants(min, size)
                .into_iter()
                .try_for_each(|at| node(r, at, size / 2.0, depth + 1, out))
        } else {
            Ok(())
        }
    }
    node(&mut r, min, size, 0, &mut surfaces)?;
    if (surfaces.nodes, surfaces.maps.len()) != (node_count, map_count) {
        bail!(
            "{} nodes and {} maps, the header says {node_count} and {map_count}",
            surfaces.nodes,
            surfaces.maps.len()
        );
    }
    let kinds = r.u32_le()? as usize;
    if kinds > SURFACE_KINDS {
        bail!("a table of {kinds} values");
    }
    surfaces.table = (0..kinds).map(|_| r.u32_le()).collect::<Result<_>>()?;
    surfaces.default = r.u32_le()?;
    if r.pos() != block.len() {
        bail!("{} bytes after the table", block.len() - r.pos());
    }
    Ok(surfaces)
}

/// Reads the tile at `r` covering `min` to `min + size`, and its children.
fn read_tile(
    r: &mut Reader,
    min: [f32; 2],
    size: f32,
    depth: usize,
    tiles: &mut Vec<Tile>,
) -> Result<u32> {
    if depth > MAX_DEPTH {
        bail!("the tile tree is deeper than {MAX_DEPTH}");
    }
    let index = tiles.len() as u32;
    let chunk_size = r.u32_le()?;
    let id = r.array::<16>()?;
    // Further chunks, size and id each: Palm City's tiles have none or one, their
    // finer chunk; any after the first are skipped.
    let further = r.u8()?;
    let mut finer = None;
    for _ in 0..further {
        let size = r.u32_le()?;
        let id = r.array::<16>()?;
        finer = finer.or(Some((guid_order(id), size)));
    }
    let _unknown = r.u8()?;
    let has_children = r.u8()?;
    let chunk = (chunk_size != 0 || id != [0; 16]).then_some((guid_order(id), chunk_size));
    tiles.push(Tile {
        min,
        size,
        chunk,
        finer,
        children: None,
    });
    match has_children {
        0 => {}
        1 => {
            let mut children = [0; 4];
            for (c, [qx, qz]) in children.iter_mut().zip(QUADRANTS) {
                let at = [
                    min[0] + qx as f32 * size / 2.0,
                    min[1] + qz as f32 * size / 2.0,
                ];
                *c = read_tile(r, at, size / 2.0, depth + 1, tiles)?;
            }
            tiles[index as usize].children = Some(children);
        }
        c => bail!("tile {index}: children flag {c}"),
    }
    Ok(index)
}

/// A tile's heights, `SAMPLES` × `SAMPLES` in rows along z, columns along x, with a
/// `BORDER` of samples beyond the tile on each side.
#[derive(Debug, Clone, PartialEq)]
pub struct Heightfield {
    samples: Vec<u16>,
}

impl Heightfield {
    /// Reads the heights at the start of a heightfield chunk.
    pub fn parse(chunk: &[u8]) -> Result<Self> {
        let bytes = Reader::at(chunk, 0)?.bytes(2 * SAMPLES * SAMPLES)?;
        let (pairs, _) = bytes.as_chunks::<2>();
        let samples = pairs.iter().map(|&b| u16::from_le_bytes(b)).collect();
        Ok(Self { samples })
    }

    /// The height in metres at row `row` and column `column` (counting the border);
    /// `None` outside, and for 0, which marks a hole (the lowest ground is 5).
    pub fn at(&self, row: usize, column: usize) -> Option<f32> {
        if row >= SAMPLES || column >= SAMPLES {
            return None;
        }
        let h = self.samples[row * SAMPLES + column];
        (h != 0).then_some(f32::from(h) * UNIT)
    }

    /// The ground's normal (unit length, y up) at row `row` and column `column`, from
    /// the heights `step` samples either side, `cell` metres a sample: into the border
    /// at the tile's edges, so it agrees with its neighbour's. One-sided where a side
    /// is a hole or beyond the border, level along an axis with neither; `None` at a
    /// hole.
    pub fn normal(&self, row: usize, column: usize, step: usize, cell: f32) -> Option<[f32; 3]> {
        let here = self.at(row, column)?;
        let d = step as f32 * cell;
        let slope = |before: Option<f32>, after: Option<f32>| match (before, after) {
            (Some(a), Some(b)) => (b - a) / (2.0 * d),
            (Some(a), None) => (here - a) / d,
            (None, Some(b)) => (b - here) / d,
            (None, None) => 0.0,
        };
        let back = |i: usize| i.checked_sub(step);
        let dx = slope(
            back(column).and_then(|c| self.at(row, c)),
            self.at(row, column + step),
        );
        let dz = slope(
            back(row).and_then(|r| self.at(r, column)),
            self.at(row + step, column),
        );
        let length = (dx * dx + 1.0 + dz * dz).sqrt();
        Some([-dx / length, 1.0 / length, -dz / length])
    }

    /// The `CELLS + 1` raw samples (0 a hole) along one edge of the tile, border left
    /// out: along z for [`Edge::MinX`] and [`Edge::MaxX`], along x for the others.
    pub fn edge(&self, edge: Edge) -> Vec<u16> {
        let (first, last) = (BORDER, BORDER + CELLS);
        (first..=last)
            .map(|k| {
                let (row, column) = match edge {
                    Edge::MinX => (k, first),
                    Edge::MaxX => (k, last),
                    Edge::MinZ => (first, k),
                    Edge::MaxZ => (last, k),
                };
                self.samples[row * SAMPLES + column]
            })
            .collect()
    }
}

/// A side of a tile's square.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    MinX,
    MaxX,
    MinZ,
    MaxZ,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::MinX, Edge::MaxX, Edge::MinZ, Edge::MaxZ];

    /// The side of the neighbour that shares this one.
    pub fn opposite(self) -> Edge {
        match self {
            Edge::MinX => Edge::MaxX,
            Edge::MaxX => Edge::MinX,
            Edge::MinZ => Edge::MaxZ,
            Edge::MaxZ => Edge::MinZ,
        }
    }
}

/// How far, in the heights' units, a finer tile's edge `fine` lies from its coarser
/// neighbour's edge over the same stretch, `coarse`, drawn as straight lines between
/// its samples: the greatest distance at any of `fine`'s samples (0 when every one
/// lies on the coarse edge). Heat's tiles are made so this is next to nothing
/// (docs/FORMATS.md, "Terrain"), so a crack where detail changes is at most that
/// wide. `None` where either has a hole, or the lengths are not `n·k + 1` and
/// `n + 1` samples.
pub fn edge_gap(fine: &[u16], coarse: &[u16]) -> Option<f32> {
    let spans = coarse.len().checked_sub(1).filter(|&n| n > 0)?;
    let k = fine.len().checked_sub(1).filter(|f| f % spans == 0)? / spans;
    if k == 0 || fine.contains(&0) || coarse.contains(&0) {
        return None;
    }
    let gap = fine
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            let (span, j) = (i / k, i % k);
            let a = f32::from(coarse[span]);
            let b = coarse.get(span + 1).map_or(a, |&b| f32::from(b));
            let line = a + (b - a) * j as f32 / k as f32;
            (f32::from(f) - line).abs()
        })
        .fold(0.0, f32::max);
    Some(gap)
}

/// A tile's colour, `COLOUR_SIDE` × `COLOUR_SIDE` texels (sRGB-encoded) in rows
/// along z, columns along x, with a `COLOUR_BORDER` of texels beyond the tile on each
/// side: half the heightfield's detail.
#[derive(Debug, Clone, PartialEq)]
pub struct Colour {
    texels: Vec<[u8; 3]>,
}

impl Colour {
    /// Reads the colour at the end of a tile's chunk.
    pub fn parse(chunk: &[u8]) -> Result<Self> {
        let Some(at) = chunk.len().checked_sub(COLOUR_BYTES) else {
            bail!("{} bytes, too short for a colour", chunk.len());
        };
        let blocks = &chunk[at..];
        let side = COLOUR_SIDE / 4;
        let mut texels = vec![[0; 3]; COLOUR_SIDE * COLOUR_SIDE];
        for (b, block) in blocks.as_chunks::<8>().0.iter().enumerate() {
            let (by, bx) = (b / side, b % side);
            let palette = bc1_palette(block);
            let bits = u32::from_le_bytes(block[4..].try_into().expect("4 bytes"));
            for i in 0..16 {
                let (y, x) = (by * 4 + i / 4, bx * 4 + i % 4);
                texels[y * COLOUR_SIDE + x] = palette[(bits >> (2 * i)) as usize & 3];
            }
        }
        Ok(Self { texels })
    }

    /// The colour at `at` (x and z) of a tile at `min` of side `size`, blended
    /// between the four nearest texels' middles.
    pub fn sample(&self, min: [f32; 2], size: f32, at: [f32; 2]) -> [u8; 3] {
        let per_texel = size / (COLOUR_SIDE - 2 * COLOUR_BORDER) as f32;
        let last = (COLOUR_SIDE - 1) as f32;
        let [u, v] = [0, 1]
            .map(|k| ((at[k] - min[k]) / per_texel + COLOUR_BORDER as f32 - 0.5).clamp(0.0, last));
        let (c0, r0) = (u as usize, v as usize);
        let (c1, r1) = ((c0 + 1).min(COLOUR_SIDE - 1), (r0 + 1).min(COLOUR_SIDE - 1));
        let (fu, fv) = (u - c0 as f32, v - r0 as f32);
        let t = |r: usize, c: usize| self.texels[r * COLOUR_SIDE + c];
        let (a, b, c, d) = (t(r0, c0), t(r0, c1), t(r1, c0), t(r1, c1));
        [0, 1, 2].map(|k| {
            let top = f32::from(a[k]) * (1.0 - fu) + f32::from(b[k]) * fu;
            let bottom = f32::from(c[k]) * (1.0 - fu) + f32::from(d[k]) * fu;
            (top * (1.0 - fv) + bottom * fv).round() as u8
        })
    }
}

/// A tile's pages: one weight mask per layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Pages {
    /// The layer of each page, in page order.
    pub layers: Vec<u16>,
    masks: Vec<u8>,
}

impl Pages {
    /// Reads the pages of a tile's chunk, for `layers` (one per page, as
    /// [`Tree::pages`] gives them); an error when the chunk holds another number.
    pub fn parse(chunk: &[u8], layers: Vec<u16>) -> Result<Self> {
        let bytes = PAGE_SIDE * PAGE_SIDE * layers.len();
        let tail = chunk.len().checked_sub(COLOUR_BYTES);
        let start = tail.and_then(|t| t.checked_sub(bytes));
        let (Some(tail), Some(start)) = (tail, start) else {
            bail!(
                "{} bytes, too short for {} pages",
                chunk.len(),
                layers.len()
            );
        };
        let heights = chunk.len() == HEIGHTFIELD as usize + bytes;
        if !(heights || start == 0) {
            bail!("{} bytes do not hold {} pages", chunk.len(), layers.len());
        }
        Ok(Self {
            layers,
            masks: chunk[start..tail].to_vec(),
        })
    }

    /// Page `page`'s weights, [`PAGE_SIDE`] rows of as many (rows along z, a 4-texel
    /// border). Panics when there is no such page.
    pub fn mask(&self, page: usize) -> &[u8] {
        &self.masks[page * PAGE_SIDE * PAGE_SIDE..][..PAGE_SIDE * PAGE_SIDE]
    }

    /// Each of `slots`' share of the weight at `at` (x and z) of a tile at `min` of
    /// side `size`, out of all its pages' (255 is all of it): each page blended
    /// between the four nearest texels' middles. Zero for a layer it has no page for,
    /// and for all when no page has weight there.
    pub fn weights(&self, min: [f32; 2], size: f32, at: [f32; 2], slots: &[u16]) -> [u8; 8] {
        let per_texel = size / (PAGE_SIDE - 2 * BORDER) as f32;
        let last = (PAGE_SIDE - 1) as f32;
        let [u, v] =
            [0, 1].map(|k| ((at[k] - min[k]) / per_texel + BORDER as f32 - 0.5).clamp(0.0, last));
        let (c0, r0) = (u as usize, v as usize);
        let (c1, r1) = ((c0 + 1).min(PAGE_SIDE - 1), (r0 + 1).min(PAGE_SIDE - 1));
        let (fu, fv) = (u - c0 as f32, v - r0 as f32);
        let weight = |page: usize| {
            let m = self.mask(page);
            let t = |r: usize, c: usize| f32::from(m[r * PAGE_SIDE + c]);
            let top = t(r0, c0) * (1.0 - fu) + t(r0, c1) * fu;
            let bottom = t(r1, c0) * (1.0 - fu) + t(r1, c1) * fu;
            top * (1.0 - fv) + bottom * fv
        };
        // Summed in page order, as the shares are taken of it.
        let total: f32 = (0..self.layers.len()).map(weight).sum();
        let mut out = [0; DETAIL_SLOTS];
        if total > 0.0 {
            for (o, slot) in out.iter_mut().zip(slots) {
                if let Some(p) = self.layers.iter().position(|l| l == slot) {
                    *o = (weight(p) / total * 255.0).round() as u8;
                }
            }
        }
        out
    }
}

/// The height bounds after a heightfield chunk's heights, [`BOUNDS`] least and
/// greatest heights (in the heights' units, holes left out): the tile's, then its 4,
/// 16 and 64 blocks', each level row by row (rows along z). They bound every height
/// at the tile or below it, its finer tiles' too.
pub fn height_bounds(chunk: &[u8]) -> Result<Vec<[u16; 2]>> {
    let mut r = Reader::at(chunk, 2 * SAMPLES * SAMPLES)?;
    (0..BOUNDS)
        .map(|_| Ok([r.u16_le()?, r.u16_le()?]))
        .collect()
}

/// The coarse heights after a heightfield chunk's bounds: one grid for each of
/// [`COARSE_SIDES`], coarsest first, each row by row (rows along z), in the heights'
/// units. Each is close to the heights at its points, closer level by level, and a
/// coarser level's value is at or below the next one's at the points they share;
/// what reads them in Heat is not known (see docs/FORMATS.md).
pub fn coarse_heights(chunk: &[u8]) -> Result<[Vec<u16>; 5]> {
    let mut r = Reader::at(chunk, 2 * SAMPLES * SAMPLES + 4 * BOUNDS)?;
    let mut level = |side: usize| {
        (0..side * side)
            .map(|_| r.u16_le())
            .collect::<Result<Vec<_>>>()
    };
    Ok([
        level(COARSE_SIDES[0])?,
        level(COARSE_SIDES[1])?,
        level(COARSE_SIDES[2])?,
        level(COARSE_SIDES[3])?,
        level(COARSE_SIDES[4])?,
    ])
}

/// The ebx holding Palm City's ground height grid (its `GroundHeightEntityData`).
pub const GROUND_HEIGHTS: &str = "levels/mainlevel/default";

/// Palm City's ground height grid (`GroundHeightData`): the least height of the
/// terrain in each square of a grid over the world, rows along z and columns along x,
/// as u16 over a span (65535 is the span's top). It stands apart from the streaming
/// tree, in the level's ebx, and agrees with the tiles' height bounds (see
/// docs/FORMATS.md); what reads it in Heat is not known.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundHeights {
    /// Squares to a side.
    pub side: usize,
    /// The world's side in metres (`WorldSize`), centred on the origin.
    pub world: f32,
    /// The heights 0 and 65535 stand for, in metres (`HeightSpan`).
    pub span: [f32; 2],
    /// `side` × `side` values, row by row.
    pub values: Vec<u16>,
}

impl GroundHeights {
    /// Checks the values make a square grid.
    pub fn new(world: f32, span: [f32; 2], values: Vec<u16>) -> Result<Self> {
        let side = values.len().isqrt();
        if side == 0 || side * side != values.len() {
            bail!("ground heights: {} values are not a square", values.len());
        }
        if !(world > 0.0 && span[1] >= span[0]) {
            bail!("ground heights: world {world} or span {span:?} out of range");
        }
        Ok(Self {
            side,
            world,
            span,
            values,
        })
    }

    /// One square's side in metres.
    pub fn cell(&self) -> f32 {
        self.world / self.side as f32
    }

    /// The least height in metres of the square at `row` (along z) and `column`
    /// (along x), which covers from (−world/2 + column·cell, −world/2 + row·cell).
    pub fn at(&self, row: usize, column: usize) -> Option<f32> {
        if row >= self.side || column >= self.side {
            return None;
        }
        let v = f32::from(self.values[row * self.side + column]) / f32::from(u16::MAX);
        Some(self.span[0] + v * (self.span[1] - self.span[0]))
    }

    /// Reads the grid from the `GroundHeightEntityData` of a partition.
    pub fn from_partition(p: &crate::ebx::Partition) -> Result<Self> {
        use crate::ebx::Value;
        use crate::level::{elements, field};
        let entity = p
            .instances
            .iter()
            .map(|i| p.object(i))
            .find(|o| p.type_name(o.type_index) == "GroundHeightEntityData")
            .ok_or_else(|| Error(format!("{}: no GroundHeightEntityData", p.name)))?;
        let Some(Value::Struct(data)) = field(p, entity, "Data")? else {
            bail!("{}: GroundHeightEntityData has no Data", p.name);
        };
        let float = |o, name: &str| match field(p, o, name)? {
            Some(Value::Float32(v)) => Ok(v),
            Some(Value::UInt(v)) => Ok(v as f32),
            Some(Value::Int(v)) => Ok(v as f32),
            other => bail!("{}: {name} is {other:?}", p.name),
        };
        let Some(Value::Struct(span)) = field(p, data, "HeightSpan")? else {
            bail!("{}: GroundHeightData has no HeightSpan", p.name);
        };
        let values = elements(p, data, "Data")?
            .iter()
            .map(|v| match v {
                Value::UInt(v) => u16::try_from(*v).map_err(|_| Error(format!("{v}"))),
                other => Err(Error(format!("{}: a height is {other:?}", p.name))),
            })
            .collect::<Result<Vec<_>>>()?;
        Self::new(
            float(data, "WorldSize")?,
            [float(span, "x")?, float(span, "y")?],
            values,
        )
    }

    /// Reads Palm City's grid from `store`.
    pub fn read(store: &Store) -> Result<Self> {
        let asset = store
            .ebx(GROUND_HEIGHTS)
            .ok_or_else(|| Error(format!("{GROUND_HEIGHTS}: not in the store")))?;
        let data = store.entries()?.read(asset.location)?;
        let p = crate::ebx::Partition::parse(&data)?;
        Self::from_partition(&p).map_err(|e| Error(format!("{GROUND_HEIGHTS}: {e}")))
    }
}

/// What a finer chunk of `size` bytes holds: whether it has four heightfields, how
/// many pages, and whether it has four colours; `None` for a size no such chunk has.
/// The four combinations leave different remainders of a page's size, so the size
/// alone tells them apart.
pub fn finer_layout(size: u32) -> Option<(bool, usize, bool)> {
    let size = size as usize;
    let page = PAGES as usize;
    [(false, false), (false, true), (true, false), (true, true)]
        .into_iter()
        .find_map(|(heights, colours)| {
            let fixed =
                usize::from(heights) * 4 * FINER_HEIGHTS + usize::from(colours) * 4 * COLOUR_BYTES;
            let rest = size.checked_sub(fixed)?;
            rest.is_multiple_of(page)
                .then_some((heights, rest / page, colours))
        })
}

/// A tile's finer chunk: its four quadrants, in [`QUADRANTS`] order, at twice its
/// detail: a heightfield each (a tile at the quadrant's square has the same layout),
/// pages for the paged layers at each quadrant's square, and a colour each. The chunk
/// stores them in [`FINER_ORDER`]: the heightfields, then each quadrant's pages,
/// then the colours.
#[derive(Debug, Clone, PartialEq)]
pub struct Finer {
    pub heights: Option<[Heightfield; 4]>,
    pub pages: [Pages; 4],
    pub colours: Option<[Colour; 4]>,
}

impl Finer {
    /// Reads a finer chunk, for `layers` (each quadrant's, as [`Tree::finer_pages`]
    /// gives them); an error when its size does not hold them.
    pub fn parse(chunk: &[u8], layers: [Vec<u16>; 4]) -> Result<Self> {
        let Some((has_heights, count, has_colours)) =
            u32::try_from(chunk.len()).ok().and_then(finer_layout)
        else {
            bail!("{} bytes: not a finer chunk's size", chunk.len());
        };
        let listed: usize = layers.iter().map(Vec::len).sum();
        if count != listed {
            bail!("{count} pages, the layer tree lists {listed}");
        }
        let page_bytes = PAGE_SIDE * PAGE_SIDE;
        let mut at = 0;
        let heights = if has_heights {
            let mut stored = Vec::with_capacity(4);
            for _ in FINER_ORDER {
                stored.push(Heightfield::parse(&chunk[at..at + FINER_HEIGHTS])?);
                at += FINER_HEIGHTS;
            }
            Some(in_quadrant_order(stored))
        } else {
            None
        };
        let mut pages: [Option<Pages>; 4] = Default::default();
        for q in FINER_ORDER {
            let len = layers[q].len() * page_bytes;
            pages[q] = Some(Pages {
                layers: layers[q].clone(),
                masks: chunk[at..at + len].to_vec(),
            });
            at += len;
        }
        let colours = if has_colours {
            let mut stored = Vec::with_capacity(4);
            for _ in FINER_ORDER {
                stored.push(Colour::parse(&chunk[at..at + COLOUR_BYTES])?);
                at += COLOUR_BYTES;
            }
            Some(in_quadrant_order(stored))
        } else {
            None
        };
        Ok(Self {
            heights,
            pages: pages.map(|p| p.expect("every quadrant's pages")),
            colours,
        })
    }
}

/// Four quadrants' values, as a finer chunk stores them ([`FINER_ORDER`]), in
/// [`QUADRANTS`] order.
fn in_quadrant_order<T>(stored: Vec<T>) -> [T; 4] {
    let mut out: [Option<T>; 4] = [None, None, None, None];
    for (value, q) in stored.into_iter().zip(FINER_ORDER) {
        out[q] = Some(value);
    }
    out.map(|v| v.expect("four quadrants"))
}

/// The ground under an area, in world space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ground {
    pub positions: Vec<[f32; 3]>,
    /// One per position, from its heights ([`Heightfield::normal`]).
    pub normals: Vec<[f32; 3]>,
    /// One per position, from the tile's colour (sRGB-encoded; alpha 255).
    pub colours: Vec<[u8; 4]>,
    /// The layers whose weights the vertices carry, at most [`DETAIL_SLOTS`].
    pub detail: Vec<u16>,
    /// One per position: each of `detail`'s share of the weight of the tile's pages
    /// (255 is all of it), from the tile's pages or its nearest ancestor's.
    pub weights: Vec<[u8; DETAIL_SLOTS]>,
    pub indices: Vec<u32>,
    /// The same colour and weights on a grid, for what is drawn over the ground.
    pub map: GroundMap,
    /// How many leaf tiles it covers.
    pub tiles: usize,
    /// Tiles whose heightfield could not be read, with why; their area is drawn from
    /// a coarser tile's heights when there is one.
    pub failed: Vec<(usize, String)>,
}

/// The ground's colour and weights over the area it was asked for, a texel every
/// `texel` metres, the first's middle half a texel from `min` (x and z): what a road
/// blends into at its edges (see `docs/FORMATS.md`, "Asphalt"). A texel over no
/// drawn tile (none with heights to draw from) is grey with no weights.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroundMap {
    pub min: [f32; 2],
    pub texel: f32,
    pub width: usize,
    pub height: usize,
    /// Row by row (z, then x): the colour (sRGB-encoded; alpha 255)…
    pub colours: Vec<[u8; 4]>,
    /// …each of [`Ground::detail`]'s share of the weight, as a vertex's…
    pub weights: Vec<[u8; DETAIL_SLOTS]>,
    /// …and its normal as a vertex's, each axis from -1 to 1 as 0 to 255, alpha 255;
    /// all 0 where it has none (no heights, or a hole).
    pub normals: Vec<[u8; 4]>,
}

/// The most texels on a side of a [`GroundMap`]: a larger area takes larger texels.
pub const MAP_SIDE: usize = 4096;

impl GroundMap {
    /// A map of `lo`..`hi` with a texel every `spacing` metres, or as many as
    /// [`MAP_SIDE`] allows; every texel grey with no weights.
    fn new(lo: [f32; 2], hi: [f32; 2], spacing: f32) -> Self {
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(0.0);
        let texel = spacing.max(extent / MAP_SIDE as f32).max(f32::MIN_POSITIVE);
        let side = |k: usize| (((hi[k] - lo[k]) / texel).ceil().max(0.0) as usize).min(MAP_SIDE);
        let (width, height) = (side(0), side(1));
        let [r, g, b] = NO_COLOUR;
        Self {
            min: lo,
            texel,
            width,
            height,
            colours: vec![[r, g, b, 255]; width * height],
            weights: vec![[0; DETAIL_SLOTS]; width * height],
            normals: vec![[0; 4]; width * height],
        }
    }

    /// The texels whose middles lie in the square at `min` of side `size`: their
    /// columns, then their rows.
    fn texels_in(&self, min: [f32; 2], size: f32) -> [std::ops::Range<usize>; 2] {
        let count = [self.width, self.height];
        [0, 1].map(|k| {
            let first = |at: f32| {
                (((at - self.min[k]) / self.texel - 0.5).ceil().max(0.0) as usize).min(count[k])
            };
            first(min[k])..first(min[k] + size)
        })
    }
}

/// The tile whose heights or colour the walk draws from: its square and its data.
type Source<T> = ([f32; 2], f32, std::rc::Rc<T>);

/// The grey a vertex takes when no tile on its way down has a colour.
const NO_COLOUR: [u8; 3] = [128; 3];

/// The ground over `lo`..`hi` (x and z): every leaf tile overlapping it, drawn from
/// its own heightfield or its nearest ancestor's with one, at a vertex every
/// `spacing` metres or the heightfield's own spacing if coarser, and coloured by its
/// own colour or its nearest ancestor's. Each vertex carries the weights of the
/// layers of `detail` that have the most pages over the area (at most
/// [`DETAIL_SLOTS`]). Holes are left out.
pub fn ground(
    tree: &Tree,
    lo: [f32; 2],
    hi: [f32; 2],
    spacing: f32,
    detail: &[u16],
    mut load: impl FnMut(&[u8; 16]) -> Result<Vec<u8>>,
) -> Ground {
    let mut ground = Ground {
        detail: detail_slots(tree, lo, hi, detail),
        map: GroundMap::new(lo, hi, spacing),
        ..Ground::default()
    };
    if !tree.tiles.is_empty() {
        walk(
            tree,
            0,
            Sources::default(),
            (lo, hi),
            spacing,
            &mut load,
            &mut ground,
        );
    }
    ground
}

/// Of `candidates`, those with the most page area over `lo`..`hi`: each leaf tile's
/// square (clipped to it) counted for the layers it is weighted by (its own pages',
/// or its nearest ancestor's), at most [`DETAIL_SLOTS`], most first.
pub fn detail_slots(tree: &Tree, lo: [f32; 2], hi: [f32; 2], candidates: &[u16]) -> Vec<u16> {
    let mut area: HashMap<u16, f32> = HashMap::new();
    if !tree.tiles.is_empty() {
        leaf_pages(tree, 0, Vec::new(), (lo, hi), 0, &mut area);
    }
    let mut ranked: Vec<(u16, f32)> = area
        .into_iter()
        .filter(|(l, a)| candidates.contains(l) && *a > 0.0)
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(DETAIL_SLOTS)
        .map(|(l, _)| l)
        .collect()
}

/// Adds each leaf under tile `index` overlapping `lo`..`hi` to `area`, for the
/// layers of its pages or, without any, `pages` (its nearest ancestor's).
fn leaf_pages(
    tree: &Tree,
    index: usize,
    mut pages: Vec<u16>,
    (lo, hi): ([f32; 2], [f32; 2]),
    depth: usize,
    area: &mut HashMap<u16, f32>,
) {
    let Some(t) = tree.tiles.get(index).filter(|t| t.overlaps(lo, hi)) else {
        return;
    };
    let own = tree.pages(t);
    if !own.is_empty() && t.page_count() == own.len() {
        pages = own;
    }
    match t.children.filter(|_| depth < MAX_DEPTH) {
        Some(children) => {
            for c in children {
                leaf_pages(tree, c as usize, pages.clone(), (lo, hi), depth + 1, area);
            }
        }
        None => {
            let side = |k: usize| (hi[k].min(t.min[k] + t.size) - lo[k].max(t.min[k])).max(0.0);
            for layer in pages {
                *area.entry(layer).or_default() += side(0) * side(1);
            }
        }
    }
}

/// The chunks [`ground`] reads for `lo`..`hi`, in the order it reads them: those of
/// every tile with colour overlapping it whose ancestors overlap it too.
pub fn ground_chunks(tree: &Tree, lo: [f32; 2], hi: [f32; 2]) -> Vec<[u8; 16]> {
    let mut out = Vec::new();
    let mut stack = vec![0usize];
    while let Some(index) = stack.pop() {
        let Some(tile) = tree.tiles.get(index).filter(|t| t.overlaps(lo, hi)) else {
            continue;
        };
        if let Some((id, _)) = tile.chunk.filter(|_| tile.has_colour()) {
            out.push(id);
        }
        if let Some(children) = tile.children {
            stack.extend(children.iter().rev().map(|&c| c as usize));
        }
    }
    out
}

/// The tiles a walk draws from: heights, colour and pages, each its own or an
/// ancestor's.
#[derive(Clone, Default)]
struct Sources {
    heights: Option<Source<Heightfield>>,
    colour: Option<Source<Colour>>,
    pages: Option<Source<Pages>>,
}

fn walk(
    tree: &Tree,
    index: usize,
    mut sources: Sources,
    (lo, hi): ([f32; 2], [f32; 2]),
    spacing: f32,
    load: &mut impl FnMut(&[u8; 16]) -> Result<Vec<u8>>,
    ground: &mut Ground,
) {
    let Some(tile) = tree.tiles.get(index) else {
        return;
    };
    if !tile.overlaps(lo, hi) {
        return;
    }
    if let Some((id, _)) = tile.chunk.filter(|_| tile.has_colour()) {
        let read = load(&id).and_then(|c| {
            let h = tile
                .has_heights()
                .then(|| Heightfield::parse(&c))
                .transpose()?;
            // Pages are detail: a tile without them (or whose chunk doesn't hold as
            // many as the layer tree lists) is weighted by its ancestor's.
            let layers = tree.pages(tile);
            let p = (!layers.is_empty())
                .then(|| Pages::parse(&c, layers).ok())
                .flatten();
            Ok((h, Colour::parse(&c)?, p))
        });
        let (min, size) = (tile.min, tile.size);
        match read {
            Ok((h, c, p)) => {
                if let Some(h) = h {
                    sources.heights = Some((min, size, h.into()));
                }
                sources.colour = Some((min, size, c.into()));
                if let Some(p) = p {
                    sources.pages = Some((min, size, p.into()));
                }
            }
            Err(e) => ground.failed.push((index, e.0)),
        }
    }
    match tile.children {
        Some(children) => {
            for c in children {
                walk(
                    tree,
                    c as usize,
                    sources.clone(),
                    (lo, hi),
                    spacing,
                    load,
                    ground,
                );
            }
        }
        None => {
            if let Some(h) = &sources.heights {
                ground.tiles += 1;
                emit(tile, h, &sources, spacing, ground);
            }
        }
    }
}

/// Adds `tile`'s square, drawn from the heights of `source` (itself or an ancestor),
/// coloured and weighted by `sources`' colour and pages.
fn emit(
    tile: &Tile,
    (min, size, heights): &Source<Heightfield>,
    sources: &Sources,
    spacing: f32,
    ground: &mut Ground,
) {
    let cell = size / CELLS as f32;
    // Samples per vertex: a power of two, so it divides the tile's cells.
    let mut step = 1;
    while (step * 2) as f32 * cell <= spacing && step * 2 <= CELLS {
        step *= 2;
    }
    let first = [0, 1].map(|k| BORDER + ((tile.min[k] - min[k]) / cell).round() as usize);
    let cells = ((tile.size / cell).round() as usize / step).max(1);
    let base = ground.positions.len() as u32;
    let side = cells + 1;
    // Each grid point's vertex, row by row; none where the heights have a hole.
    let mut index_of: Vec<Option<u32>> = vec![None; side * side];
    let mut next = base;
    for i in 0..side {
        for j in 0..side {
            let (row, column) = (first[1] + i * step, first[0] + j * step);
            if let (Some(y), Some(n)) = (
                heights.at(row, column),
                heights.normal(row, column, step, cell),
            ) {
                index_of[i * side + j] = Some(next);
                next += 1;
                let (x, z) = (
                    min[0] + (column - BORDER) as f32 * cell,
                    min[1] + (row - BORDER) as f32 * cell,
                );
                ground.positions.push([x, y, z]);
                ground.normals.push(n);
                let [r, g, b] = sources
                    .colour
                    .as_ref()
                    .map_or(NO_COLOUR, |(at, side, c)| c.sample(*at, *side, [x, z]));
                ground.colours.push([r, g, b, 255]);
                let w = sources
                    .pages
                    .as_ref()
                    .map_or([0; DETAIL_SLOTS], |(at, side, p)| {
                        p.weights(*at, *side, [x, z], &ground.detail)
                    });
                ground.weights.push(w);
            }
        }
    }
    let [columns, rows] = ground.map.texels_in(tile.min, tile.size);
    let map = &mut ground.map;
    for row in rows {
        for column in columns.clone() {
            let at = [column, row].map(|i| i as f32 + 0.5);
            let [x, z] = [0, 1].map(|k| map.min[k] + at[k] * map.texel);
            let i = row * map.width + column;
            if let Some((at, side, c)) = &sources.colour {
                let [r, g, b] = c.sample(*at, *side, [x, z]);
                map.colours[i] = [r, g, b, 255];
            }
            if let Some((at, side, p)) = &sources.pages {
                map.weights[i] = p.weights(*at, *side, [x, z], &ground.detail);
            }
            // The nearest sample's, `step` either side as the vertices' are.
            let [column, row] =
                [0, 1].map(|k| BORDER + (([x, z][k] - min[k]) / cell).round().max(0.0) as usize);
            if let Some(n) = heights.normal(row, column, step, cell) {
                let [a, b, c] = n.map(|a| ((a * 0.5 + 0.5) * 255.0).round() as u8);
                map.normals[i] = [a, b, c, 255];
            }
        }
    }
    for i in 0..cells {
        for j in 0..cells {
            let at = |(i, j): (usize, usize)| index_of[i * side + j];
            let (Some(a), Some(b), Some(c), Some(d)) = (
                at((i, j)),
                at((i, j + 1)),
                at((i + 1, j + 1)),
                at((i + 1, j)),
            ) else {
                continue;
            };
            ground.indices.extend_from_slice(&[a, c, b, a, d, c]);
        }
    }
}

/// Reads tile chunks from a store.
pub struct Chunks<'s> {
    entries: crate::store::Entries<'s>,
    chunks: HashMap<[u8; 16], Location>,
}

impl<'s> Chunks<'s> {
    pub fn new(store: &'s Store) -> Result<Self> {
        Ok(Self {
            entries: store.entries()?,
            chunks: store.whole_chunks(),
        })
    }

    /// Reads the chunks `ids` ahead, in the packs' order ([`Entries::prefetch`]);
    /// those the store lacks are left for [`Chunks::read`] to report.
    ///
    /// [`Entries::prefetch`]: crate::store::Entries::prefetch
    pub fn prefetch(&mut self, ids: &[[u8; 16]]) {
        let locations: Vec<Location> = ids
            .iter()
            .filter_map(|id| self.chunks.get(id))
            .copied()
            .collect();
        self.entries.prefetch(locations);
    }

    /// The chunk `id` (in bundle order).
    pub fn read(&mut self, id: &[u8; 16]) -> Result<Vec<u8>> {
        let location = *self
            .chunks
            .get(id)
            .ok_or_else(|| Error(format!("chunk {} is not in the store", hex(id))))?;
        self.entries.read(location)
    }
}

fn hex(id: &[u8; 16]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn node(out: &mut Vec<u8>, min: [f32; 2], size: f32, empty: bool, pyramid: bool, kids: bool) {
        for v in [min[0], 0.0, min[1], min[0] + size, 10.0, min[1] + size, 1.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(u8::from(empty));
        if empty {
            return;
        }
        out.extend_from_slice(&[u8::from(pyramid), u8::from(!pyramid), 0, 0]);
        if pyramid {
            out.extend_from_slice(&[0; PYRAMID]);
        }
        out.push(u8::from(kids));
    }

    fn tile(out: &mut Vec<u8>, size: u32, id: u8, extra: bool, kids: bool) {
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&[id; 16]);
        out.push(u8::from(extra));
        if extra {
            out.extend_from_slice(&[0x11; 20]);
        }
        out.extend_from_slice(&[0, u8::from(kids)]);
    }

    /// A layer node at `min` + `size`: its records' layers, whether each is paged,
    /// and whether four children follow; no layers makes an empty node.
    fn layer_node(out: &mut Vec<u8>, min: [f32; 2], size: f32, layers: &[(u16, bool)], kids: bool) {
        let paged = layers.iter().filter(|(_, p)| *p).count() as u16;
        out.push(0);
        out.extend_from_slice(&(layers.len() as u16).to_le_bytes());
        out.extend_from_slice(&paged.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        if layers.is_empty() {
            return;
        }
        for &(layer, paged) in layers {
            out.extend_from_slice(&layer.to_le_bytes());
            for v in [min[0], min[1], min[0] + size, min[1] + size] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&[u8::from(!kids), u8::from(!paged)]);
            out.extend_from_slice(&[0x1F; 11]);
        }
        out.extend_from_slice(&[0, u8::from(kids)]);
    }

    /// The sample's layer tree: the root with layers 0 (unpaged) and 2; its first
    /// child with 0, 2 and 5, the third with 2, the others empty.
    fn layer_block() -> Vec<u8> {
        let mut d = vec![0; LAYER_HEADER];
        for (at, v) in [
            (0x18, -128.0f32),
            (0x1C, -128.0),
            (0x20, 128.0),
            (0x24, 128.0),
        ] {
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        d[0x28..0x2C].copy_from_slice(&5u32.to_le_bytes());
        d[0x2C..0x30].copy_from_slice(&6u32.to_le_bytes());
        layer_node(
            &mut d,
            [-128.0, -128.0],
            256.0,
            &[(0, false), (2, true)],
            true,
        );
        layer_node(
            &mut d,
            [-128.0, -128.0],
            128.0,
            &[(0, false), (2, true), (5, true)],
            false,
        );
        layer_node(&mut d, [0.0, -128.0], 128.0, &[], false);
        layer_node(&mut d, [0.0, 0.0], 128.0, &[(2, true)], false);
        layer_node(&mut d, [-128.0, 0.0], 128.0, &[], false);
        d
    }

    /// A 256 m tree from −128: the root, with four children, the third empty in the
    /// node tree; the root has no chunk, the first child a heightfield, the second
    /// none (2,312 bytes), and the third and fourth heightfields, the fourth with an
    /// extra chunk. The layer tree is [`layer_block`].
    fn sample() -> Vec<u8> {
        sample_with_root(0)
    }

    /// [`sample`], its root's chunk `root` bytes (a heightfield's, or none).
    fn sample_with_root(root: u32) -> Vec<u8> {
        let mut d = vec![0; HEADER];
        d[TILE_COUNT_AT..TILE_COUNT_AT + 4].copy_from_slice(&5u32.to_le_bytes());
        d[NODE_COUNT_AT..NODE_COUNT_AT + 4].copy_from_slice(&5u32.to_le_bytes());
        node(&mut d, [-128.0, -128.0], 256.0, false, true, true);
        node(&mut d, [-128.0, -128.0], 128.0, false, false, false);
        node(&mut d, [0.0, -128.0], 128.0, false, false, false);
        node(&mut d, [0.0, 0.0], 128.0, true, false, false);
        node(&mut d, [-128.0, 0.0], 128.0, false, false, false);
        let layers = layer_block();
        d.push(LAYER_BLOCK);
        d.extend_from_slice(&(layers.len() as u32).to_le_bytes());
        d.extend_from_slice(&layers);
        d.extend_from_slice(&[9, 3, 0, 0, 0, 9, 9, 9, LAST_BLOCK]);
        tile(&mut d, root, 0, false, true);
        tile(&mut d, HEIGHTFIELD, 1, false, false);
        tile(&mut d, 2312, 2, false, false);
        tile(&mut d, HEIGHTFIELD + PAGES, 3, false, false);
        tile(&mut d, HEIGHTFIELD, 4, true, false);
        d
    }

    #[test]
    fn reads_the_tiles_as_a_quadtree() {
        let tree = Tree::parse(&sample()).unwrap();
        assert_eq!(tree.nodes, 5);
        let t = &tree.tiles;
        assert_eq!(t.len(), 5);
        assert_eq!(
            (t[0].min, t[0].size, t[0].chunk),
            ([-128.0, -128.0], 256.0, None)
        );
        assert_eq!(t[0].children, Some([1, 2, 3, 4]));
        let squares: Vec<_> = t[1..].iter().map(|t| (t.min, t.size)).collect();
        assert_eq!(
            squares,
            [
                ([-128.0, -128.0], 128.0),
                ([0.0, -128.0], 128.0),
                ([0.0, 0.0], 128.0),
                ([-128.0, 0.0], 128.0)
            ]
        );
        assert_eq!(t[1].chunk, Some(([1; 16], HEIGHTFIELD)));
        let heights: Vec<_> = t.iter().map(Tile::has_heights).collect();
        assert_eq!(heights, [false, true, false, true, true]);
        let pages: Vec<_> = t.iter().map(Tile::page_count).collect();
        assert_eq!(pages, [0, 0, 0, 1, 0]);
    }

    /// An outline block of `kind` over 256 m from −128: the root with four children,
    /// the second split again.
    fn outline_block(kind: u8) -> Vec<u8> {
        let (_, layout) = OUTLINE_BLOCKS.iter().find(|(k, _)| *k == kind).unwrap();
        let mut d = vec![0; layout.header];
        d[0..4].copy_from_slice(&68u32.to_le_bytes());
        for (i, v) in [-128.0f32, -128.0, 128.0, 128.0].into_iter().enumerate() {
            let at = layout.square_at + 4 * i;
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        d[layout.count_at..layout.count_at + 4].copy_from_slice(&9u32.to_le_bytes());
        let node = |d: &mut Vec<u8>, kids: bool| {
            d.extend(std::iter::repeat_n(1, layout.flags));
            d.push(u8::from(kids));
        };
        node(&mut d, true);
        node(&mut d, false);
        node(&mut d, true);
        (0..4).for_each(|_| node(&mut d, false));
        node(&mut d, false);
        node(&mut d, false);
        d
    }

    #[test]
    fn reads_an_outline_blocks_leaves() {
        for (kind, layout) in &OUTLINE_BLOCKS {
            let o = read_outline(*kind, &outline_block(*kind), layout).unwrap();
            assert_eq!((o.kind, o.side, o.nodes), (*kind, 68, 9));
            assert_eq!(o.leaves.len(), 7);
            assert_eq!(o.leaves[0], ([-128.0, -128.0], 128.0));
            // The second child's quadrants, then the third and fourth children.
            assert_eq!(o.leaves[1], ([0.0, -128.0], 64.0));
            assert_eq!(o.leaves[3], ([64.0, -64.0], 64.0));
            assert_eq!(o.leaves[6], ([-128.0, 0.0], 128.0));
            let mut short = outline_block(*kind);
            short.pop();
            assert!(read_outline(*kind, &short, layout).is_err());
            let mut bad = outline_block(*kind);
            let at = layout.count_at;
            bad[at..at + 4].copy_from_slice(&8u32.to_le_bytes());
            assert!(read_outline(*kind, &bad, layout).is_err());
        }
    }

    /// A surface block over 8 m from 0 with maps of 4 × 4 values: the root, its first
    /// child with a map (rows coded `rows`), the others empty; a table of two values.
    fn surface_block(rows: &[&[u8]]) -> Vec<u8> {
        let mut d = vec![0; SURFACE_HEADER];
        d[0..4].copy_from_slice(&4u32.to_le_bytes());
        for (i, v) in [0.0f32, 0.0, 8.0, 8.0].into_iter().enumerate() {
            d[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        d[SURFACE_NODES_AT..SURFACE_NODES_AT + 4].copy_from_slice(&5u32.to_le_bytes());
        d[SURFACE_MAPS_AT..SURFACE_MAPS_AT + 4].copy_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&[0, 0, 1]);
        d.extend_from_slice(&[1, 1]);
        let len: usize = rows.iter().map(|r| r.len()).sum();
        d.extend_from_slice(&(len as u32).to_le_bytes());
        rows.iter().for_each(|r| d.extend_from_slice(r));
        rows.iter()
            .for_each(|r| d.extend_from_slice(&(r.len() as u16).to_le_bytes()));
        d.push(0);
        (0..3).for_each(|_| d.extend_from_slice(&[0, 1, 0]));
        for v in [2u32, 0x0510_8780, 0x00A0_0280, 0x80] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d
    }

    #[test]
    fn reads_the_surface_map_run_length_coded_low_values_first() {
        // A run (two equal bytes, then one more), literals, and a run of exactly two.
        let rows: [&[u8]; 4] = [
            &[0x11, 0x11, 1],
            &[0x21, 0x43],
            &[0xFF, 0xFF, 1],
            &[0x10, 0x10, 1],
        ];
        let s = read_surfaces(&surface_block(&rows)).unwrap();
        assert_eq!((s.side, s.nodes, s.maps.len()), (4, 5, 1));
        assert_eq!(s.table, [0x0510_8780, 0x00A0_0280]);
        assert_eq!((s.default, material(s.table[0])), (0x80, 0x87));
        assert_eq!(
            s.table.iter().map(|&e| property(e)).collect::<Vec<_>>(),
            [81, 10]
        );
        let map = &s.maps[&square([0.0, 0.0], 4.0)];
        assert_eq!((map.min, map.size), ([0.0, 0.0], 4.0));
        assert_eq!(
            map.values(4).unwrap(),
            [1, 1, 1, 1, 1, 2, 3, 4, 15, 15, 15, 15, 0, 1, 0, 1]
        );
        assert!(map.values(8).is_err());
        // A row decoding to too many values, and a run with no count.
        let long: [&[u8]; 4] = [&[0x11, 0x11, 2], &[0, 0, 1], &[0, 0, 1], &[0, 0, 1]];
        let s = read_surfaces(&surface_block(&long)).unwrap();
        assert!(s.maps.values().next().unwrap().values(4).is_err());
        let cut: [&[u8]; 4] = [&[0, 0, 1], &[0, 0, 1], &[0, 0, 1], &[0x11, 0x11]];
        let s = read_surfaces(&surface_block(&cut)).unwrap();
        assert!(s.maps.values().next().unwrap().values(4).is_err());
        // A block cut short, and a row length off by one.
        let mut short = surface_block(&rows);
        short.pop();
        assert!(read_surfaces(&short).is_err());
        let mut bad = surface_block(&rows);
        let at = SURFACE_HEADER + 3 + 2 + 4 + 11;
        bad[at] += 1;
        assert!(read_surfaces(&bad).is_err());
    }

    #[test]
    fn a_tile_keeps_its_finer_chunk_and_its_quadrants_layers() {
        let tree = Tree::parse(&sample()).unwrap();
        let finer: Vec<_> = tree.tiles.iter().map(|t| t.finer).collect();
        let id = guid_order([0x11; 16]);
        assert_eq!(finer, [None, None, None, None, Some((id, 0x1111_1111))]);
        // The root's quadrants are the four 128 m squares of the layer tree.
        let layers = tree.finer_pages(&tree.tiles[0]);
        assert_eq!(layers, [vec![2, 5], vec![], vec![2], vec![]]);
    }

    #[test]
    fn a_finer_chunks_size_tells_what_it_holds() {
        // Sizes Palm City's finer chunks have: colours alone, pages and colours,
        // heightfields and colours, heightfields alone, and pages alone.
        let layouts: Vec<_> = [9_248, 83_232, 168_924, 159_676, 187_420, 36_992, 1_000]
            .into_iter()
            .map(finer_layout)
            .collect();
        assert_eq!(
            layouts,
            [
                Some((false, 0, true)),
                Some((false, 4, true)),
                Some((true, 0, true)),
                Some((true, 0, false)),
                Some((true, 1, true)),
                Some((false, 2, false)),
                None
            ]
        );
    }

    /// A finer chunk with heightfields, `layers`' pages and colours: each stored
    /// heightfield's samples, page's bytes and colour's blocks are its place in the
    /// chunk's order (1, 2, …).
    fn finer_chunk(layers: &[Vec<u16>; 4]) -> Vec<u8> {
        let mut d = Vec::new();
        for k in 1..=4u16 {
            for _ in 0..SAMPLES * SAMPLES {
                d.extend_from_slice(&k.to_le_bytes());
            }
            d.extend_from_slice(&[0; AFTER_HEIGHTS]);
        }
        let mut n = 0;
        for q in FINER_ORDER {
            for _ in &layers[q] {
                n += 1;
                d.extend(std::iter::repeat_n(n, PAGE_SIDE * PAGE_SIDE));
            }
        }
        for k in 1..=4u16 {
            d = coloured(d, block(k, k, 0), block(k, k, 0));
        }
        d
    }

    #[test]
    fn a_finer_chunk_stores_its_quadrants_in_reverse() {
        let layers = [vec![2, 5], vec![], vec![2], vec![]];
        let finer = Finer::parse(&finer_chunk(&layers), layers.clone()).unwrap();
        // Stored (0, 1), (1, 1), (1, 0), (0, 0): the last is the first quadrant's.
        let heights = finer.heights.unwrap();
        let first: Vec<_> = heights.iter().map(|h| h.at(0, 0).unwrap() * 32.0).collect();
        assert_eq!(first, [4.0, 3.0, 2.0, 1.0]);
        // Quadrant (1, 1)'s one page comes first, then (0, 0)'s two.
        let pages: Vec<_> = finer
            .pages
            .iter()
            .map(|p| {
                (
                    p.layers.clone(),
                    p.masks
                        .chunks(PAGE_SIDE * PAGE_SIDE)
                        .map(|m| m[0])
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            pages,
            [
                (vec![2, 5], vec![2, 3]),
                (vec![], vec![]),
                (vec![2], vec![1]),
                (vec![], vec![])
            ]
        );
        let colours = finer.colours.unwrap();
        for (q, k) in [(0, 4u16), (1, 3), (2, 2), (3, 1)] {
            let own = coloured(Vec::new(), block(k, k, 0), block(k, k, 0));
            assert_eq!(colours[q], Colour::parse(&own).unwrap(), "quadrant {q}");
        }
    }

    #[test]
    fn a_finer_chunk_that_does_not_hold_its_layers_is_an_error() {
        let layers = [vec![2, 5], vec![], vec![2], vec![]];
        let d = finer_chunk(&layers);
        assert!(Finer::parse(&d, [vec![2], vec![], vec![2], vec![]]).is_err());
        assert!(Finer::parse(&d[..d.len() - 1], layers.clone()).is_err());
        assert!(Finer::parse(&[], [vec![], vec![], vec![], vec![]]).is_ok());
    }

    #[test]
    fn the_height_bounds_follow_the_heights() {
        let mut d = rows();
        for k in 0..BOUNDS as u16 {
            d.extend_from_slice(&k.to_le_bytes());
            d.extend_from_slice(&(k + 100).to_le_bytes());
        }
        let bounds = height_bounds(&d).unwrap();
        assert_eq!(bounds.len(), BOUNDS);
        assert_eq!((bounds[0], bounds[84]), ([0, 100], [84, 184]));
        assert!(height_bounds(&d[..d.len() - 1]).is_err());
    }

    #[test]
    fn the_ground_heights_are_a_square_grid_over_the_span() {
        let g = GroundHeights::new(512.0, [10.0, 20.0], vec![0, u16::MAX, 32_768, 1]).unwrap();
        assert_eq!((g.side, g.cell()), (2, 256.0));
        assert_eq!(g.at(0, 0), Some(10.0));
        assert_eq!(g.at(0, 1), Some(20.0));
        assert!((g.at(1, 0).unwrap() - 15.0).abs() < 1e-3);
        assert_eq!(g.at(2, 0), None);
        assert!(GroundHeights::new(512.0, [0.0, 1.0], vec![0; 3]).is_err());
        assert!(GroundHeights::new(512.0, [0.0, 1.0], vec![]).is_err());
        assert!(GroundHeights::new(0.0, [0.0, 1.0], vec![0]).is_err());
    }

    #[test]
    fn the_coarse_heights_follow_the_bounds() {
        let mut d = rows();
        d.extend(std::iter::repeat_n(0xAB, 4 * BOUNDS));
        let count: usize = COARSE_SIDES.iter().map(|s| s * s).sum();
        assert_eq!(count, 408);
        for k in 0..count as u16 {
            d.extend_from_slice(&k.to_le_bytes());
        }
        let levels = coarse_heights(&d).unwrap();
        assert_eq!(levels[0], [0, 1, 2, 3]);
        assert_eq!((levels[1][0], levels[1][8]), (4, 12));
        assert_eq!(
            (levels[4].len(), levels[4][0], levels[4][288]),
            (289, 119, 407)
        );
        assert!(coarse_heights(&d[..d.len() - 1]).is_err());
    }

    #[test]
    fn a_damaged_tree_is_an_error() {
        let d = sample();
        for cut in [10, HEADER + 20, d.len() - 1] {
            assert!(Tree::parse(&d[..cut]).is_err(), "cut at {cut}");
        }
        let mut longer = d.clone();
        longer.push(0);
        assert!(Tree::parse(&longer).is_err());
        let mut miscounted = d.clone();
        miscounted[TILE_COUNT_AT] = 6;
        assert!(Tree::parse(&miscounted).is_err());
        let mut bad_flag = d;
        let last = bad_flag.len() - 1;
        bad_flag[last] = 2;
        assert!(Tree::parse(&bad_flag).is_err());
    }

    #[test]
    fn a_tiles_pages_are_the_paged_layers_at_its_square() {
        let tree = Tree::parse(&sample()).unwrap();
        let pages: Vec<_> = tree.tiles.iter().map(|t| tree.pages(t)).collect();
        assert_eq!(pages, [vec![2], vec![2, 5], vec![], vec![2], vec![]]);
        let root = &tree.layers[&square([-128.0, -128.0], 256.0)];
        assert_eq!(
            root.iter()
                .map(|r| (r.layer, r.paged, r.leaf))
                .collect::<Vec<_>>(),
            [(0, false, false), (2, true, false)]
        );
        assert_eq!(root[0].cells, [0x1F; 11]);
    }

    #[test]
    fn a_records_cells_are_a_pyramid_row_by_row() {
        // Palm City's commonest pattern after "all of it": the square, its last
        // quadrant, and the last cell of each level below (bits 0, 4, 20 and 84).
        let mut cells = [0; 11];
        for bit in [0, 4, 20, 84] {
            cells[bit / 8] |= 1 << (bit % 8);
        }
        assert_eq!(cells, [0x11, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0x10]);
        let record = LayerRecord {
            layer: 3,
            paged: true,
            leaf: true,
            cells,
        };
        let set: Vec<_> = (0..CELL_LEVELS)
            .flat_map(|l| {
                let n = 1 << l;
                (0..n * n).map(move |k| (l, k / n, k % n))
            })
            .filter(|&(l, r, c)| record.touches(l, r, c))
            .collect();
        assert_eq!(set, [(0, 0, 0), (1, 1, 1), (2, 3, 3), (3, 7, 7)]);
        // A row-major level: bit 5 + 1·4 + 2 is row 1, column 2 of the 4 × 4 level.
        let mut one = [0; 11];
        one[11 / 8] |= 1 << (11 % 8);
        let record = LayerRecord {
            cells: one,
            ..record
        };
        assert!(record.touches(2, 1, 2) && !record.touches(2, 2, 1));
        // Outside the pyramid: nothing.
        let all = LayerRecord {
            cells: [0xFF; 11],
            ..record
        };
        assert!(all.touches(3, 7, 7));
        assert!(!all.touches(4, 0, 0) && !all.touches(1, 2, 0) && !all.touches(0, 0, 1));
    }

    #[test]
    fn a_damaged_layer_tree_is_an_error() {
        let good = layer_block();
        assert!(read_layers(&good).is_ok());
        let first = LAYER_HEADER + 7;
        let mut descending = good.clone();
        descending[first..first + 2].copy_from_slice(&3u16.to_le_bytes());
        let mut moved = good.clone();
        moved[first + 2..first + 6].copy_from_slice(&64.0f32.to_le_bytes());
        let mut miscounted = good.clone();
        miscounted[0x2C] = 7;
        let mut paged = good.clone();
        paged[LAYER_HEADER + 3] = 2;
        let mut longer = good.clone();
        longer.push(0);
        for (what, d) in [
            ("descending", descending),
            ("moved", moved),
            ("miscounted", miscounted),
            ("paged count", paged),
            ("longer", longer),
            ("cut", good[..good.len() - 1].to_vec()),
        ] {
            assert!(read_layers(&d).is_err(), "{what}");
        }
    }

    /// A heightfield whose sample (r, c) is `r + 1` units, except a hole at (10, 10).
    fn rows() -> Vec<u8> {
        let mut d = Vec::new();
        for r in 0..SAMPLES {
            for c in 0..SAMPLES {
                let h = if (r, c) == (10, 10) { 0 } else { r as u16 + 1 };
                d.extend_from_slice(&h.to_le_bytes());
            }
        }
        d
    }

    /// A BC1 block of the two 5:6:5 colours `a` and `b`, each texel picking `pick`.
    fn block(a: u16, b: u16, pick: u32) -> [u8; 8] {
        let mut d = [0; 8];
        d[..2].copy_from_slice(&a.to_le_bytes());
        d[2..4].copy_from_slice(&b.to_le_bytes());
        d[4..].copy_from_slice(&(pick * 0x5555_5555).to_le_bytes());
        d
    }

    /// `before` bytes, then a colour whose blocks are all `b`, except block 0 (the
    /// top-left 4 × 4 texels) is `first`.
    fn coloured(before: Vec<u8>, first: [u8; 8], b: [u8; 8]) -> Vec<u8> {
        let mut d = before;
        d.extend_from_slice(&first);
        for _ in 1..COLOUR_BYTES / 8 {
            d.extend_from_slice(&b);
        }
        d
    }

    #[test]
    fn a_colour_is_the_chunks_last_bc1_blocks() {
        // Red and blue: the first, the second, and the two thirds between.
        let (red, blue) = (0xF800, 0x001F);
        let picks: Vec<_> = (0..4)
            .map(|p| {
                Colour::parse(&coloured(
                    vec![9; 5],
                    block(red, blue, p),
                    block(red, blue, 0),
                ))
                .unwrap()
                .texels[0]
            })
            .collect();
        assert_eq!(
            picks,
            [[255, 0, 0], [0, 0, 255], [170, 0, 85], [85, 0, 170]]
        );
        // The first not greater: the middle, and black.
        let c = Colour::parse(&coloured(vec![], block(blue, red, 2), block(blue, red, 3))).unwrap();
        assert_eq!((c.texels[0], c.texels[4]), ([127, 0, 127], [0, 0, 0]));
        assert!(Colour::parse(&[0; COLOUR_BYTES - 1]).is_err());
    }

    #[test]
    fn a_colour_is_sampled_between_its_texels_middles() {
        // Texels 0-3 of rows 0-3 white, the rest black; a 64 m tile, a texel a
        // metre, its first texel's middle 1.5 m before the tile.
        let (white, black) = (block(0xFFFF, 0xFFFF, 0), block(0, 0, 0));
        let c = Colour::parse(&coloured(vec![], white, black)).unwrap();
        let at = |x: f32, z: f32| c.sample([0.0, 0.0], 64.0, [x, z]);
        // Texel 3's middle (1.5 m into the tile) is white, texel 4's black.
        assert_eq!(at(1.5, 1.5), [255; 3]);
        assert_eq!(at(2.5, 1.5), [0; 3]);
        assert_eq!(at(2.0, 1.5), [128; 3]);
        assert_eq!(at(2.0, 2.0), [64; 3]);
        // Beyond the border it takes the edge texel.
        assert_eq!(at(-100.0, -100.0), [255; 3]);
        assert_eq!(at(100.0, 100.0), [0; 3]);
    }

    /// A colour-only chunk with a page per value of `pages`: the first texel of each
    /// set to the value's first, the rest to its second.
    fn paged(pages: &[(u8, u8)]) -> Vec<u8> {
        let mut d = Vec::new();
        for &(first, rest) in pages {
            let mut page = vec![rest; PAGE_SIDE * PAGE_SIDE];
            page[0] = first;
            d.extend(page);
        }
        coloured(d, block(0, 0, 0), block(0, 0, 0))
    }

    #[test]
    fn a_layers_weight_is_its_share_of_the_pages() {
        let p = Pages::parse(&paged(&[(0, 200), (0, 100), (0, 0)]), vec![2, 5, 9]).unwrap();
        let at = |x: f32, slots: &[u16]| p.weights([0.0, 0.0], 128.0, [x, 64.0], slots);
        // Layer 2 has two thirds, 5 a third, 9 none; slots past the layers are zero.
        assert_eq!(at(64.0, &[5, 2, 9]), [85, 170, 0, 0, 0, 0, 0, 0]);
        assert_eq!(at(64.0, &[7]), [0; 8], "a layer with no page");
        // The first texel (3.5 texels before the tile's corner) has no weight at all.
        let corner = p.weights([0.0, 0.0], 128.0, [-3.5, -3.5], &[2, 5]);
        assert_eq!(corner, [0; 8]);
        // Pages before a heightfield's colour, as many as the tree lists.
        let mut h = rows();
        h.resize(HEIGHTFIELD as usize - COLOUR_BYTES, 0);
        h.extend(&paged(&[(0, 50)])[..]);
        assert_eq!(h.len(), HEIGHTFIELD as usize + PAGE_SIDE * PAGE_SIDE);
        let one = Pages::parse(&h, vec![4]).unwrap();
        assert_eq!(one.weights([0.0; 2], 64.0, [9.0; 2], &[4])[0], 255);
        assert!(
            Pages::parse(&h, vec![4, 6]).is_err(),
            "two pages listed, one held"
        );
        assert!(Pages::parse(&paged(&[(0, 9)]), vec![]).is_err());
        assert!(Pages::parse(&[0; 10], vec![1]).is_err());
    }

    #[test]
    fn the_detail_slots_are_the_layers_with_the_most_pages() {
        let tree = Tree::parse(&sample()).unwrap();
        // The layer tree lists 2 for the root and two leaves, 5 for the first leaf,
        // but only the third leaf's chunk holds its page (2): the root has no chunk,
        // the first leaf's none.
        let all = [-128.0, -128.0];
        assert_eq!(detail_slots(&tree, all, [128.0; 2], &[5, 2, 7]), [2]);
        assert!(detail_slots(&tree, all, [128.0; 2], &[5]).is_empty());
        assert!(detail_slots(&tree, [0.0, -128.0], [128.0, -1.0], &[2]).is_empty());
    }

    #[test]
    fn heights_are_thirty_seconds_of_a_metre_and_zero_is_a_hole() {
        let h = Heightfield::parse(&rows()).unwrap();
        assert_eq!(h.at(0, 0), Some(1.0 / 32.0));
        assert_eq!(h.at(63, 5), Some(2.0));
        assert_eq!(h.at(10, 10), None);
        assert_eq!(h.at(SAMPLES, 0), None);
        assert!(Heightfield::parse(&rows()[..100]).is_err());
    }

    #[test]
    fn a_normal_leans_against_the_slope_of_the_heights_around_it() {
        // Each row is 1/32 m above the one before, a sample every metre.
        let h = Heightfield::parse(&rows()).unwrap();
        let rising = 1.0 / 32.0;
        let length = (1.0f32 + rising * rising).sqrt();
        let up_the_rows = [0.0, 1.0 / length, -rising / length];
        let near = |n: Option<[f32; 3]>, want: [f32; 3]| {
            let n = n.unwrap();
            assert!((0..3).all(|k| (n[k] - want[k]).abs() < 1e-6), "{n:?}");
        };
        // Two samples either side, from two metres apart.
        near(h.normal(50, 50, 2, 1.0), up_the_rows);
        // One-sided at the first row and the last, and beside the hole at (10, 10).
        near(h.normal(0, 50, 2, 1.0), up_the_rows);
        near(h.normal(SAMPLES - 1, 50, 2, 1.0), up_the_rows);
        near(h.normal(10, 8, 2, 1.0), up_the_rows);
        // Level along an axis with no sample either side.
        near(h.normal(60, 60, 100, 1.0), [0.0, 1.0, 0.0]);
        assert_eq!(h.normal(10, 10, 2, 1.0), None);
    }

    #[test]
    fn an_edge_is_its_samples_from_border_to_border() {
        // Each sample is its row plus 1: the x edges rise along z, the z edges are
        // one row's value throughout.
        let h = Heightfield::parse(&rows()).unwrap();
        let rising: Vec<u16> = (BORDER as u16 + 1..=(BORDER + CELLS) as u16 + 1).collect();
        assert_eq!(h.edge(Edge::MinX), rising);
        assert_eq!(h.edge(Edge::MaxX), rising);
        assert_eq!(h.edge(Edge::MinZ), vec![BORDER as u16 + 1; CELLS + 1]);
        assert_eq!(
            h.edge(Edge::MaxZ),
            vec![(BORDER + CELLS) as u16 + 1; CELLS + 1]
        );
        assert!(Edge::ALL.iter().all(|e| e.opposite().opposite() == *e));
        assert_eq!(Edge::MinZ.opposite(), Edge::MaxZ);
    }

    #[test]
    fn an_edges_gap_is_its_distance_from_the_coarser_edges_lines() {
        // Two coarse spans, four fine samples to each: on the lines, then one off.
        let coarse = [100, 140, 120];
        let mut fine = vec![100, 110, 120, 130, 140, 135, 130, 125, 120];
        assert_eq!(edge_gap(&fine, &coarse), Some(0.0));
        fine[5] = 132;
        assert_eq!(edge_gap(&fine, &coarse), Some(3.0));
        // Twice as fine, the in-between samples halfway.
        assert_eq!(edge_gap(&[100, 121, 140], &[100, 140]), Some(1.0));
        // The same detail: sample against sample.
        assert_eq!(edge_gap(&[7, 9], &[7, 8]), Some(1.0));
        // Holes, and lengths that don't fit, have no gap.
        assert_eq!(edge_gap(&[100, 0, 140], &[100, 140]), None);
        assert_eq!(edge_gap(&[100, 120, 140], &[0, 140]), None);
        assert_eq!(edge_gap(&[1, 2, 3, 4], &[1, 4, 7]), None);
        assert_eq!(edge_gap(&[1, 2], &[1]), None);
        assert_eq!(edge_gap(&[], &[1, 2]), None);
    }

    #[test]
    fn the_ground_takes_each_leaf_from_its_own_heights_or_an_ancestors() {
        let tree = Tree::parse(&sample()).unwrap();
        let mut asked = Vec::new();
        let load = |id: &[u8; 16]| {
            asked.push(id[0]);
            match id[0] {
                3 => Err(Error("unreadable".into())),
                _ => Ok(rows()),
            }
        };
        // Only the first two quadrants (z < 0), a vertex every 16 m (8 cells); the
        // second is read for its colour alone.
        let g = ground(&tree, [-128.0, -128.0], [128.0, -1.0], 16.0, &[], load);
        assert_eq!(asked, [1, 2]);
        // The second quadrant has no heights and the root none to fall back on.
        assert_eq!(g.tiles, 1);
        assert_eq!(g.positions.len(), 81);
        assert_eq!(g.indices.len(), 8 * 8 * 6);
        assert_eq!(g.positions[0], [-128.0, 5.0 / 32.0, -128.0]);
        assert_eq!(g.positions[80], [0.0, 133.0 / 32.0, 0.0]);
        // Every vertex leans back against the rows' rise, 1/32 m a metre.
        assert_eq!(g.normals.len(), g.positions.len());
        assert!(
            g.normals
                .iter()
                .all(|n| n[0] == 0.0 && (n[2] * 32.0 + n[1]).abs() < 1e-6)
        );
        // The fourth quadrant, at full spacing, misses the hole's four cells.
        let g = ground(&tree, [-128.0, 0.0], [0.0, 128.0], 0.5, &[], |_| Ok(rows()));
        assert_eq!(g.positions.len(), 129 * 129 - 1);
        assert_eq!(g.colours.len(), g.positions.len());
        assert_eq!(g.indices.len(), (128 * 128 - 4) * 6);
        let g = ground(&tree, [0.0, 0.0], [128.0, 128.0], 1.0, &[], |_| {
            Err(Error("no".into()))
        });
        assert_eq!((g.tiles, g.failed.len()), (0, 1));
    }

    #[test]
    fn the_chunks_read_ahead_are_those_the_ground_reads_in_its_order() {
        for tree in [sample(), sample_with_root(HEIGHTFIELD)] {
            let tree = Tree::parse(&tree).unwrap();
            for (lo, hi) in [
                ([-128.0, -128.0], [128.0, -1.0]),
                ([-128.0, 0.0], [0.0, 128.0]),
                ([-500.0; 2], [500.0; 2]),
                ([300.0; 2], [400.0; 2]),
            ] {
                let mut asked = Vec::new();
                ground(&tree, lo, hi, 16.0, &[], |id| {
                    asked.push(*id);
                    Ok(rows())
                });
                assert_eq!(ground_chunks(&tree, lo, hi), asked, "{lo:?}..{hi:?}");
            }
        }
    }

    #[test]
    fn each_leaf_is_coloured_by_its_own_colour_or_an_ancestors() {
        // The root and the first tile have grey heights; the second has no heights
        // and is green, so it takes its heights from the root and its own colour.
        let tree = Tree::parse(&sample_with_root(HEIGHTFIELD)).unwrap();
        let grey = coloured(rows(), block(0x8410, 0x8410, 0), block(0x8410, 0x8410, 0));
        let green = coloured(vec![], block(0x07E0, 0x07E0, 0), block(0x07E0, 0x07E0, 0));
        let g = ground(&tree, [-128.0, -128.0], [128.0, -1.0], 16.0, &[], |id| {
            Ok(if id[0] == 2 {
                green.clone()
            } else {
                grey.clone()
            })
        });
        assert_eq!((g.tiles, g.colours.len()), (2, 2 * 81));
        assert!(g.colours[..81].iter().all(|&c| c == [131, 129, 131, 255]));
        assert!(g.colours[81..].iter().all(|&c| c == [0, 255, 0, 255]));
        // The map likewise, a texel every 16 m: 16 across, 8 down (127 m, rounded
        // up); the first 8 columns grey, the rest green.
        let m = &g.map;
        assert_eq!(
            (m.min, m.texel, m.width, m.height),
            ([-128.0; 2], 16.0, 16, 8)
        );
        for (i, &c) in m.colours.iter().enumerate() {
            let want = if i % 16 < 8 {
                [131, 129, 131, 255]
            } else {
                [0, 255, 0, 255]
            };
            assert_eq!(c, want, "texel {i}");
        }
        // Every texel leans back against the rows' rise as the vertices do: 1/32 m a
        // metre in the first's own heights, 1/64 in the root's (twice its size).
        for (i, &n) in m.normals.iter().enumerate() {
            let z = if i % 16 < 8 { 124 } else { 126 };
            assert_eq!(n, [128, 255, z, 255], "texel {i}");
        }
        // The second's chunk unreadable: it is drawn in the root's colour.
        let g = ground(&tree, [0.0, -128.0], [128.0, -1.0], 16.0, &[], |id| {
            if id[0] == 2 {
                Err(Error("no".into()))
            } else {
                Ok(grey.clone())
            }
        });
        assert_eq!((g.tiles, g.failed.len()), (1, 1));
        assert!(g.colours.iter().all(|&c| c == [131, 129, 131, 255]));
        assert!(g.map.colours.iter().all(|&c| c == [131, 129, 131, 255]));
    }

    #[test]
    fn a_large_maps_texels_grow_to_fit_its_side() {
        let m = GroundMap::new([0.0; 2], [16_384.0, 100.0], 2.0);
        assert_eq!((m.texel, m.width, m.height), (4.0, MAP_SIDE, 25));
        assert_eq!(m.colours.len(), MAP_SIDE * 25);
        // The texels whose middles are in a square: those at 2, 6, … 10 m.
        assert_eq!(m.texels_in([1.0, -50.0], 10.0), [0..3, 0..0]);
        assert_eq!(m.texels_in([16_380.0, 98.0], 128.0), [4095..4096, 24..25]);
        let empty = GroundMap::new([5.0; 2], [5.0; 2], 2.0);
        assert_eq!((empty.width, empty.height, empty.colours.len()), (0, 0, 0));
    }

    /// Reads Palm City's streaming tree and every heightfield in the store
    /// `NFSH_STORE` points at (local-ci sets it on the Windows side), and checks the
    /// rule placing them: each one's even samples are its parent's.
    #[test]
    fn every_heightfield_sits_where_the_tree_puts_it() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let tree = Tree::read(&store).unwrap();
        assert_eq!((tree.nodes, tree.tiles.len()), (10_077, 8_253));
        // The outlines: kind 2 down to 32 m, kind 4 every square down to 1,024 m.
        let leaves = |o: &Outline| {
            let mut by_side = std::collections::BTreeMap::new();
            o.leaves
                .iter()
                .for_each(|(_, s)| *by_side.entry(*s as u32).or_insert(0) += 1);
            by_side.into_iter().collect::<Vec<_>>()
        };
        let outlines: Vec<_> = tree
            .outlines
            .iter()
            .map(|o| (o.kind, o.side, o.nodes, leaves(o)))
            .collect();
        assert_eq!(
            outlines,
            [
                (
                    2,
                    68,
                    32_021,
                    vec![(32, 128), (64, 18_656), (128, 3_072), (256, 2_160)]
                ),
                (4, 137, 341, vec![(1_024, 256)])
            ]
        );
        // The surface map: 521 maps of 256 × 256 values over 512 and 1,024 m squares,
        // every row decoding to its 256 values; a table of 16.
        let s = tree.surfaces.as_ref().unwrap();
        assert_eq!(
            (s.side, s.nodes, s.maps.len(), s.table.len()),
            (256, 797, 521, 16)
        );
        let mut counts = [0usize; SURFACE_KINDS];
        let mut sides = (0, 0);
        for map in s.maps.values() {
            map.values(s.side)
                .unwrap()
                .iter()
                .for_each(|&v| counts[usize::from(v)] += 1);
            match map.size {
                512.0 => sides.0 += 1,
                1_024.0 => sides.1 += 1,
                other => panic!("a map of {other} m"),
            }
        }
        assert_eq!(sides, (414, 107));
        assert_eq!(
            (counts[0], counts[1], counts[15]),
            (4_206_202, 19_289_697, 715_785)
        );
        let mut chunks = Chunks::new(&store).unwrap();
        // The root and the four 8,192 m quadrants have no chunk.
        let without: Vec<_> = tree
            .tiles
            .iter()
            .filter(|t| t.chunk.is_none())
            .map(|t| t.size)
            .collect();
        assert_eq!(without, [16_384.0, 8_192.0, 8_192.0, 8_192.0, 8_192.0]);
        // Every other tile's chunk has one of the two sizes, so ends in its colour.
        assert!(
            tree.tiles
                .iter()
                .all(|t| t.chunk.is_none() || t.has_colour())
        );
        // A tile's pages are the paged layers at its square (none without a node).
        assert!(
            tree.tiles
                .iter()
                .all(|t| t.page_count() == tree.pages(t).len())
        );
        let records: usize = tree.layers.values().map(Vec::len).sum();
        assert_eq!((tree.layers.len(), records), (18_689, 37_033));
        let speedway = tree
            .tiles
            .iter()
            .find(|t| (t.min, t.size) == ([3_712.0, 2_944.0], 128.0))
            .unwrap();
        assert_eq!(tree.pages(speedway), [2, 8, 9, 24, 25]);
        // Every record's pyramid nests: its square is touched, and no cell is touched
        // whose parent cell isn't (counting the touched cells with no touched child).
        let mut childless = 0;
        for r in tree.layers.values().flatten() {
            assert!(r.touches(0, 0, 0), "a record not touching its square");
            for level in 1..CELL_LEVELS {
                for row in 0..1 << level {
                    for column in 0..1 << level {
                        let parent = r.touches(level - 1, row / 2, column / 2);
                        assert!(parent || !r.touches(level, row, column));
                    }
                }
                for row in 0..1 << (level - 1) {
                    for column in 0..1 << (level - 1) {
                        let kids =
                            (0..4).any(|k| r.touches(level, 2 * row + k / 2, 2 * column + k % 2));
                        childless += usize::from(r.touches(level - 1, row, column) && !kids);
                    }
                }
            }
        }
        assert_eq!(childless, 205);
        // A square listing a layer is touched by its parent's record of it (counting
        // the records whose parent doesn't list their layer).
        let mut orphans = 0;
        let root = square([-8_192.0; 2], 16_384.0);
        for (key, records) in tree.layers.iter().filter(|(k, _)| **k != root) {
            let [x, z, s] = key.map(f32::from_bits);
            let parent_min = [x, z].map(|v| v - (v + 8_192.0).rem_euclid(2.0 * s));
            let parent = &tree.layers[&square(parent_min, 2.0 * s)];
            let (row, column) = (
                usize::from(z != parent_min[1]),
                usize::from(x != parent_min[0]),
            );
            for r in records {
                match parent.iter().find(|p| p.layer == r.layer) {
                    Some(p) => assert!(p.touches(1, row, column), "layer {} at {key:?}", r.layer),
                    None => orphans += 1,
                }
            }
        }
        assert_eq!(orphans, 163);
        let mut bounds = HashMap::new();
        // Per level, the set bits whose cell, grown by the pages' border, has no
        // weight in the layer's page, and the cells with weight inside but no bit.
        let mut cells = [[0usize; 2]; CELL_LEVELS];
        let mut coarse = HashMap::new();
        let heights: Vec<Option<Heightfield>> = tree
            .tiles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let (id, _) = t.chunk?;
                let chunk = chunks.read(&id).unwrap();
                Colour::parse(&chunk).unwrap();
                let records = tree.layers.get(&square(t.min, t.size));
                let pages = Pages::parse(&chunk, tree.pages(t)).unwrap();
                for (k, r) in records
                    .iter()
                    .copied()
                    .flatten()
                    .filter(|r| r.paged)
                    .enumerate()
                {
                    let mask = pages.mask(k);
                    for (level, count) in cells.iter_mut().enumerate() {
                        let (n, side) = (1 << level, CELLS >> level);
                        for row in 0..n {
                            for column in 0..n {
                                let weight = |grow: usize| {
                                    let rows = BORDER + row * side - grow
                                        ..BORDER + (row + 1) * side + grow;
                                    rows.into_iter().any(|y| {
                                        let at = y * PAGE_SIDE + BORDER + column * side;
                                        mask[at - grow..at + side + grow].iter().any(|&w| w > 0)
                                    })
                                };
                                let touched = r.touches(level, row, column);
                                count[0] += usize::from(touched && !weight(BORDER));
                                count[1] += usize::from(!touched && weight(0));
                            }
                        }
                    }
                }
                if t.has_heights() {
                    bounds.insert(i, height_bounds(&chunk).unwrap());
                    coarse.insert(i, coarse_heights(&chunk).unwrap());
                }
                t.has_heights().then(|| Heightfield::parse(&chunk).unwrap())
            })
            .collect();
        assert_eq!(heights.iter().flatten().count(), 7_618);
        // Next to no touched cell lacks weight; about 1% of the cells with weight are
        // not touched, mostly where layers above share them (docs/FORMATS.md).
        assert_eq!(cells, [[0, 0], [0, 278], [2, 1_970], [4, 12_613]]);
        // Pairs whose samples all agree, and those agreeing only within 2 units
        // (1/16 m) or where one of them has a hole: holes are cut at the finer tile.
        let (mut equal, mut near, mut holes) = (0, 0, 0);
        for (p, t) in tree.tiles.iter().enumerate() {
            let (Some(parent), Some(children)) = (&heights[p], t.children) else {
                continue;
            };
            for (c, [qx, qz]) in children.iter().zip(QUADRANTS) {
                let Some(child) = &heights[*c as usize] else {
                    continue;
                };
                let (mut differ, mut far, mut hole) = (false, false, false);
                for i in 0..=CELLS / 2 {
                    for j in 0..=CELLS / 2 {
                        let a = child.samples[(BORDER + 2 * i) * SAMPLES + BORDER + 2 * j];
                        let (r, c) = (BORDER + i + qz as usize * 64, BORDER + j + qx as usize * 64);
                        let b = parent.samples[r * SAMPLES + c];
                        differ |= a != b;
                        hole |= a != b && (a == 0 || b == 0);
                        far |= a != 0 && b != 0 && a.abs_diff(b) > 2;
                    }
                }
                assert!(!far, "tile {c} differs from its parent {p} by over 2 units");
                match (differ, hole) {
                    (false, _) => equal += 1,
                    (true, false) => near += 1,
                    (true, true) => holes += 1,
                }
            }
        }
        assert_eq!((equal, near, holes), (7_477, 61, 64));
        // A tile's border is its neighbour's samples, which normals across the edge
        // rely on: each tile against the one of its size to its east and its south,
        // counting the pairs whose shared samples all agree (holes aside).
        let at: HashMap<[u32; 3], usize> = tree
            .tiles
            .iter()
            .enumerate()
            .map(|(i, t)| (square(t.min, t.size), i))
            .collect();
        let (mut pairs, mut agree) = (0, 0);
        for (i, t) in tree.tiles.iter().enumerate() {
            let Some(a) = &heights[i] else { continue };
            for (k, [dx, dz]) in [[1.0, 0.0], [0.0, 1.0]].into_iter().enumerate() {
                let next = [t.min[0] + dx * t.size, t.min[1] + dz * t.size];
                let Some(b) = at
                    .get(&square(next, t.size))
                    .and_then(|&j| heights[j].as_ref())
                else {
                    continue;
                };
                // Along the edge (`along`), every sample both hold: one's from 4 short
                // of its edge to its border's end, the other's from its border's start.
                let mut same = true;
                for along in 0..SAMPLES {
                    for d in 0..=2 * BORDER {
                        let (x, y) = (CELLS + d, d);
                        let (s, u) = if k == 0 {
                            (
                                a.samples[along * SAMPLES + x],
                                b.samples[along * SAMPLES + y],
                            )
                        } else {
                            (
                                a.samples[x * SAMPLES + along],
                                b.samples[y * SAMPLES + along],
                            )
                        };
                        same &= s == u || s == 0 || u == 0;
                    }
                }
                pairs += 1;
                agree += usize::from(same);
            }
        }
        assert_eq!((pairs, agree), (14_852, 14_852));
        // Each heightfield's bounds hold its own samples, holes aside, block by block
        // (they bound its finer tiles' too, so they may be wider).
        let mut exact = 0;
        for (i, b) in &bounds {
            let h = heights[*i].as_ref().unwrap();
            let mut same = true;
            let mut k = 0;
            for level in 0..4 {
                let (n, side) = (1 << level, CELLS >> level);
                for row in 0..n {
                    for column in 0..n {
                        let (mut lo, mut hi) = (u16::MAX, 0);
                        for r in BORDER + row * side..=BORDER + (row + 1) * side {
                            for c in BORDER + column * side..=BORDER + (column + 1) * side {
                                let s = h.samples[r * SAMPLES + c];
                                if s != 0 {
                                    (lo, hi) = (lo.min(s), hi.max(s));
                                }
                            }
                        }
                        let [min, max] = b[k];
                        assert!(
                            hi == 0 || (min <= lo && hi <= max),
                            "tile {i}, block {k}: samples {lo} to {hi}, bounds {min} to {max}"
                        );
                        same &= hi == 0 || [lo, hi] == b[k];
                        k += 1;
                    }
                }
            }
            exact += usize::from(same);
        }
        // Most are exactly their own samples' (the others have finer tiles).
        assert_eq!((exact, bounds.len()), (5_489, 7_618));
        // Each coarse level's value is at or below the next level's at the points
        // they share (counting those above), and next to none is above the tile's
        // greatest height (counting those), though many are below its least.
        let (mut above, mut over) = (0, 0);
        for (i, levels) in &coarse {
            for (k, pair) in levels.windows(2).enumerate() {
                let (n, m) = (COARSE_SIDES[k], COARSE_SIDES[k + 1]);
                for r in 0..n {
                    for c in 0..n {
                        above += usize::from(pair[0][r * n + c] > pair[1][2 * r * m + 2 * c]);
                    }
                }
            }
            let greatest = bounds[i][0][1];
            over += levels.iter().flatten().filter(|&&v| v > greatest).count();
        }
        assert_eq!((above, over, coarse.len()), (11, 87, 7_618));
        // The level's ground height grid is the least height of each of its squares:
        // against the bounds of the smallest heightfield (of 256 to 2,048 m) with a
        // block at the square, every square of sea floor (5 units, the root's least)
        // reads under 0.1 m, and next to every square of land is within 0.1 m of its
        // least (counting those that are); the
        // greatest is the span's top, and the squares under no such block are counted.
        let g = GroundHeights::read(&store).unwrap();
        assert_eq!((g.side, g.world, g.span[0]), (64, 16_384.0, 0.0));
        let cell = g.cell();
        let mut least: HashMap<(usize, usize), (f32, u16)> = HashMap::new();
        for (i, b) in &bounds {
            let t = &tree.tiles[*i];
            let n = (t.size / cell) as usize;
            if !(1..=8).contains(&n) {
                continue;
            }
            for r in 0..n {
                for c in 0..n {
                    let lo = b[(n * n - 1) / 3 + r * n + c][0];
                    let row = ((t.min[1] + g.world / 2.0) / cell) as usize + r;
                    let column = ((t.min[0] + g.world / 2.0) / cell) as usize + c;
                    let e = least.entry((row, column)).or_insert((f32::MAX, 0));
                    if t.size < e.0 {
                        *e = (t.size, lo);
                    }
                }
            }
        }
        let (mut sea, mut land, mut near, mut top) = (0, 0, 0, 0.0f32);
        let floor = least.values().filter(|e| e.1 == 5).count();
        for (&(row, column), &(_, lo)) in &least {
            let v = g.at(row, column).unwrap();
            if lo == 5 {
                sea += usize::from(v < 0.1);
            } else {
                land += 1;
                near += usize::from((v - f32::from(lo) / 32.0).abs() < 0.1);
                top = top.max(f32::from(lo) / 32.0);
            }
        }
        assert!((top - g.span[1]).abs() < 0.05, "{top} against {:?}", g.span);
        assert_eq!(
            (floor, sea, land, near, 64 * 64 - least.len()),
            (407, 407, 3_305, 3_292, 384)
        );
        // Every finer chunk's size holds the paged layers at its quadrants' squares.
        let finer: Vec<(usize, ([u8; 16], u32))> = tree
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| Some((i, t.finer?)))
            .collect();
        let mut kinds = (0, 0);
        for &(i, (_, size)) in &finer {
            let listed: usize = tree.finer_pages(&tree.tiles[i]).iter().map(Vec::len).sum();
            let (with_heights, count, with_colours) = finer_layout(size).unwrap();
            assert_eq!(count, listed, "tile {i}'s finer chunk");
            kinds.0 += usize::from(with_heights);
            kinds.1 += usize::from(with_colours);
        }
        assert_eq!((finer.len(), kinds.0, kinds.1), (6_133, 601, 5_942));
        // The finer heightfields: each quadrant's even samples are its tile's there.
        let (mut equal, mut near, mut holes) = (0, 0, 0);
        for &(i, (id, size)) in &finer {
            if !finer_layout(size).unwrap().0 {
                continue;
            }
            let t = &tree.tiles[i];
            let chunk = chunks.read(&id).unwrap();
            let f = Finer::parse(&chunk, tree.finer_pages(t)).unwrap();
            let parent = heights[i].as_ref().unwrap();
            for (child, [qx, qz]) in f.heights.unwrap().iter().zip(QUADRANTS) {
                let (mut differ, mut far, mut hole) = (false, false, false);
                for a in 0..=CELLS / 2 {
                    for b in 0..=CELLS / 2 {
                        let c = child.samples[(BORDER + 2 * a) * SAMPLES + BORDER + 2 * b];
                        let (r, k) = (BORDER + a + qz as usize * 64, BORDER + b + qx as usize * 64);
                        let p = parent.samples[r * SAMPLES + k];
                        differ |= c != p;
                        hole |= c != p && (c == 0 || p == 0);
                        far |= c != 0 && p != 0 && c.abs_diff(p) > 2;
                    }
                }
                assert!(!far, "tile {i}'s finer quadrant differs by over 2 units");
                match (differ, hole) {
                    (false, _) => equal += 1,
                    (true, false) => near += 1,
                    (true, true) => holes += 1,
                }
            }
        }
        assert_eq!((equal, near, holes), (2_365, 37, 2));
        // Where detail changes: every leaf heightfield at its finest (its finer
        // chunk's quadrants when it has them), each edge against the neighbour of its
        // size or larger across it. Counted by the gap from the coarser edge's lines
        // (within 2 units, within 5, more), for edges between pieces of one size and
        // of two; edges with a hole aside.
        let mut pieces: Vec<([f32; 2], f32, Heightfield)> = Vec::new();
        for (i, t) in tree.tiles.iter().enumerate() {
            let Some(h) = heights[i].as_ref().filter(|_| t.children.is_none()) else {
                continue;
            };
            let halves = t
                .finer
                .filter(|&(_, size)| finer_layout(size).unwrap().0)
                .map(|(id, _)| {
                    let chunk = chunks.read(&id).unwrap();
                    Finer::parse(&chunk, tree.finer_pages(t))
                        .unwrap()
                        .heights
                        .unwrap()
                });
            match halves {
                Some(q) => {
                    for (h, min) in q.into_iter().zip(quadrants(t.min, t.size)) {
                        pieces.push((min, t.size / 2.0, h));
                    }
                }
                None => pieces.push((t.min, t.size, h.clone())),
            }
        }
        // The axis a side lies across, and its x or z.
        let line = |min: [f32; 2], size: f32, e: Edge| match e {
            Edge::MinX => (0, min[0]),
            Edge::MaxX => (0, min[0] + size),
            Edge::MinZ => (1, min[1]),
            Edge::MaxZ => (1, min[1] + size),
        };
        let mut by_line: HashMap<(u8, u32), Vec<usize>> = HashMap::new();
        for (n, (min, size, _)) in pieces.iter().enumerate() {
            for e in Edge::ALL {
                let at = line(*min, *size, e).1;
                by_line.entry((e as u8, at.to_bits())).or_default().push(n);
            }
        }
        let (mut same, mut mixed, mut holes) = ([0usize; 3], [0usize; 3], 0);
        for (n, (min, size, h)) in pieces.iter().enumerate() {
            for e in Edge::ALL {
                let (axis, at) = line(*min, *size, e);
                let along = 1 - axis;
                let Some(across) = by_line.get(&(e.opposite() as u8, at.to_bits())) else {
                    continue;
                };
                for &m in across {
                    let (min1, size1, h1) = &pieces[m];
                    let inside =
                        min1[along] <= min[along] && min[along] + size <= min1[along] + size1;
                    // Each pair of one size once, from its lower side.
                    if size1 < size || !inside || (size1 == size && m < n) {
                        continue;
                    }
                    let ratio = (size1 / size) as usize;
                    let first = ((min[along] - min1[along]) / size1 * CELLS as f32) as usize;
                    let coarse = &h1.edge(e.opposite())[first..=first + CELLS / ratio];
                    let Some(gap) = edge_gap(&h.edge(e), coarse) else {
                        holes += 1;
                        continue;
                    };
                    let count = if ratio == 1 { &mut same } else { &mut mixed };
                    count[if gap <= 2.0 {
                        0
                    } else if gap <= 5.0 {
                        1
                    } else {
                        2
                    }] += 1;
                }
            }
        }
        assert_eq!(pieces.len(), 7_469);
        assert_eq!((same, mixed, holes), ([14_554, 0, 0], [252, 24, 2], 33));
    }

    /// Reads the surface map's table and checks it against Palm City's material grid
    /// (the `MaterialGridData` ebx): each entry packs a material of the grid's
    /// `MaterialIndexMap`, each a distinct row with race physics, the trailing value is
    /// the grid's `DefaultMaterial`, and value 14, which lies only on the burnt forest
    /// floor (layer 6), is the burnt forest floor's material.
    #[test]
    fn the_surface_table_names_the_material_grids_materials() {
        use crate::ebx::{Partition, Ref, Value};
        use crate::level::{elements, field};
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let tree = Tree::read(&store).unwrap();
        let s = tree.surfaces.as_ref().unwrap();
        let data = store
            .entries()
            .unwrap()
            .read(
                store
                    .ebx("levels/mainlevel/mainlevel/materialgrid_win32")
                    .unwrap()
                    .location,
            )
            .unwrap();
        let p = Partition::parse(&data).unwrap();
        let instance = |r: &Value| match r {
            Value::Class(Ref::Internal(i)) => p.object(&p.instances[*i]),
            other => panic!("{other:?}"),
        };
        let grid = p
            .instances
            .iter()
            .map(|i| p.object(i))
            .find(|o| p.type_name(o.type_index) == "MaterialGridData")
            .unwrap();
        let packed = |o| match field(&p, o, "Packed").unwrap() {
            Some(Value::UInt(v)) => v as u32,
            other => panic!("{other:?}"),
        };
        let Some(Value::Struct(default)) = field(&p, grid, "DefaultMaterial").unwrap() else {
            panic!("no DefaultMaterial");
        };
        assert_eq!((s.default, packed(default)), (0x80, 0x80));
        let map: Vec<usize> = elements(&p, grid, "MaterialIndexMap")
            .unwrap()
            .iter()
            .map(|v| match v {
                Value::UInt(v) => *v as usize,
                other => panic!("{other:?}"),
            })
            .collect();
        let rows = elements(&p, grid, "MaterialProperties").unwrap();
        assert_eq!((map.len(), rows.len()), (140, 123));
        let row_of = |packed: u32| {
            let Value::Struct(row) = rows[map[usize::from(material(packed))]] else {
                panic!("a row is not a struct");
            };
            row
        };
        let types = |row, name: &str| -> Vec<String> {
            elements(&p, row, name)
                .unwrap()
                .iter()
                .map(|r| p.type_name(instance(r).type_index))
                .collect()
        };
        let mut seen = std::collections::HashSet::new();
        for &e in &s.table {
            let row = map[usize::from(material(e))];
            assert!(
                e & 0xFF == 0x80 && row != 0 && seen.insert(row),
                "{e:#010x}"
            );
            let physics = types(row_of(e), "PhysicsPropertyProperties");
            assert!(
                physics
                    .iter()
                    .any(|t| t == "RaceMaterialPropertyPhysicsData")
            );
            // The modifier's row holds race physics and nothing else.
            assert_eq!(e >> 16 & 0xF, 0, "{e:#010x}");
            let Value::Struct(modifier) = rows[map[usize::from(property(e))]] else {
                panic!("a row is not a struct");
            };
            let held: Vec<String> = ["PhysicsPropertyProperties", "PhysicsMaterialProperties"]
                .iter()
                .flat_map(|name| types(modifier, name))
                .collect();
            assert_eq!(held, ["RaceMaterialPropertyPhysicsData"], "{e:#010x}");
        }
        assert_eq!(
            s.table.iter().map(|&e| property(e)).collect::<Vec<_>>(),
            [
                81, 81, 82, 81, 97, 81, 81, 97, 82, 97, 81, 82, 81, 10, 81, 10
            ]
        );
        // Value 14's sounds are the burnt forest floor's.
        let row = row_of(s.table[14]);
        let sound = ["PhysicsPropertyProperties", "PhysicsMaterialProperties"]
            .iter()
            .flat_map(|name| elements(&p, row, name).unwrap())
            .map(|r| instance(&r))
            .find(|&o| p.type_name(o.type_index) == "MaterialPropertySoundData")
            .unwrap();
        let settings = instance(&field(&p, sound, "LicenseeSoundData").unwrap().unwrap());
        let Some(Value::Class(Ref::Import(debris))) = field(&p, settings, "DebrisSound").unwrap()
        else {
            panic!("no DebrisSound");
        };
        let name = store.partition_name(&p.imports[debris].partition).unwrap();
        assert!(name.ends_with("debris_forestfloor_burned_config"), "{name}");
    }
}
