//! Palm City's terrain: the streaming tree's quadtree of tiles, a tile's heightfield
//! and colour, and a mesh of the ground under an area (see docs/FORMATS.md,
//! "Terrain").
//! Little-endian, packed with no alignment.

use std::collections::HashMap;

use crate::reader::Reader;
use crate::store::{Store, guid_order};
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
/// A tile's colour: the last bytes of its chunk, BC1 blocks of `COLOUR_SIDE` ×
/// `COLOUR_SIDE` texels with a `COLOUR_BORDER` beyond the tile on each side.
pub const COLOUR_SIDE: usize = 68;
pub const COLOUR_BORDER: usize = 2;
const COLOUR_BYTES: usize = COLOUR_SIDE * COLOUR_SIDE / 2;
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
    /// An index into the `TerrainData`'s `TerrainLayers`.
    pub layer: u16,
    /// Whether a tile at this square has a page (a weight mask) for the layer.
    pub paged: bool,
    /// Whether no square below lists the layer.
    pub leaf: bool,
    /// Which of the 1 + 4 + 16 + 64 cells below the square the layer touches, as
    /// bits; not read yet.
    pub cells: [u8; 11],
}

/// The streaming tree's tiles; the first is the root.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub tiles: Vec<Tile>,
    /// The node tree's node count.
    pub nodes: usize,
    /// The layer tree: each square's layers, in ascending order, by [`square`].
    pub layers: HashMap<[u32; 3], Vec<LayerRecord>>,
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
        loop {
            let kind = r.u8()?;
            if kind == LAST_BLOCK {
                break;
            }
            let len = r.u32_le()? as usize;
            let block = r.bytes(len)?;
            if kind == LAYER_BLOCK {
                layers = read_layers(block).map_err(|e| Error(format!("the layer tree: {e}")))?;
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
        })
    }

    /// The layers of `tile`'s pages, in page order: the paged layers at its square,
    /// ascending (likely; see docs/FORMATS.md, "Terrain layers"). Empty when no node of
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
    let extra = r.u8()?;
    // Further chunks (block-compressed data, or a leaf's finer heightfields): size
    // and id each; not read yet.
    r.bytes(20 * usize::from(extra))?;
    let _unknown = r.u8()?;
    let has_children = r.u8()?;
    let chunk = (chunk_size != 0 || id != [0; 16]).then_some((guid_order(id), chunk_size));
    tiles.push(Tile {
        min,
        size,
        chunk,
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

/// A BC1 block's four colours: its two end points (RGB 5:6:5) and the two between
/// them, or their middle and black when the first is not the greater.
fn bc1_palette(block: &[u8; 8]) -> [[u8; 3]; 4] {
    let end = |at: usize| {
        let c = u16::from_le_bytes([block[at], block[at + 1]]);
        let (r, g, b) = (c >> 11, (c >> 5) & 63, c & 31);
        [r * 255 / 31, g * 255 / 63, b * 255 / 31].map(|v| v as u8)
    };
    let (a, b) = (end(0), end(2));
    let mix = |wa: u16, wb: u16| {
        [0, 1, 2].map(|k| ((u16::from(a[k]) * wa + u16::from(b[k]) * wb) / (wa + wb)) as u8)
    };
    if u16::from_le_bytes([block[0], block[1]]) > u16::from_le_bytes([block[2], block[3]]) {
        [a, b, mix(2, 1), mix(1, 2)]
    } else {
        [a, b, mix(1, 1), [0; 3]]
    }
}

/// The ground under an area, in world space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ground {
    pub positions: Vec<[f32; 3]>,
    /// One per position, from the tile's colour (sRGB-encoded; alpha 255).
    pub colours: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
    /// How many leaf tiles it covers.
    pub tiles: usize,
    /// Tiles whose heightfield could not be read, with why; their area is drawn from
    /// a coarser tile's heights when there is one.
    pub failed: Vec<(usize, String)>,
}

/// The tile whose heights or colour the walk draws from: its square and its data.
type Source<T> = ([f32; 2], f32, std::rc::Rc<T>);

/// The grey a vertex takes when no tile on its way down has a colour.
const NO_COLOUR: [u8; 3] = [128; 3];

/// The ground over `lo`..`hi` (x and z): every leaf tile overlapping it, drawn from
/// its own heightfield or its nearest ancestor's with one, at a vertex every
/// `spacing` metres or the heightfield's own spacing if coarser, and coloured by its
/// own colour or its nearest ancestor's. Holes are left out.
pub fn ground(
    tree: &Tree,
    lo: [f32; 2],
    hi: [f32; 2],
    spacing: f32,
    mut load: impl FnMut(&[u8; 16]) -> Result<Vec<u8>>,
) -> Ground {
    let mut ground = Ground::default();
    if !tree.tiles.is_empty() {
        walk(
            tree,
            0,
            (None, None),
            lo,
            hi,
            spacing,
            &mut load,
            &mut ground,
        );
    }
    ground
}

#[allow(clippy::too_many_arguments)]
fn walk(
    tree: &Tree,
    index: usize,
    sources: (Option<Source<Heightfield>>, Option<Source<Colour>>),
    lo: [f32; 2],
    hi: [f32; 2],
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
    let (mut heights, mut colour) = sources;
    if let Some((id, _)) = tile.chunk.filter(|_| tile.has_colour()) {
        let read = load(&id).and_then(|c| {
            let h = tile
                .has_heights()
                .then(|| Heightfield::parse(&c))
                .transpose()?;
            Ok((h, Colour::parse(&c)?))
        });
        match read {
            Ok((h, c)) => {
                if let Some(h) = h {
                    heights = Some((tile.min, tile.size, h.into()));
                }
                colour = Some((tile.min, tile.size, c.into()));
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
                    (heights.clone(), colour.clone()),
                    lo,
                    hi,
                    spacing,
                    load,
                    ground,
                );
            }
        }
        None => {
            if let Some(h) = &heights {
                ground.tiles += 1;
                emit(tile, h, colour.as_ref(), spacing, ground);
            }
        }
    }
}

/// Adds `tile`'s square, drawn from the heights of `source` (itself or an ancestor)
/// and coloured by `colour`.
fn emit(
    tile: &Tile,
    (min, size, heights): &Source<Heightfield>,
    colour: Option<&Source<Colour>>,
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
    let mut index_of: HashMap<(usize, usize), u32> = HashMap::new();
    for i in 0..side {
        for j in 0..side {
            let (row, column) = (first[1] + i * step, first[0] + j * step);
            if let Some(y) = heights.at(row, column) {
                index_of.insert((i, j), base + index_of.len() as u32);
                let (x, z) = (
                    min[0] + (column - BORDER) as f32 * cell,
                    min[1] + (row - BORDER) as f32 * cell,
                );
                ground.positions.push([x, y, z]);
                let [r, g, b] =
                    colour.map_or(NO_COLOUR, |(at, side, c)| c.sample(*at, *side, [x, z]));
                ground.colours.push([r, g, b, 255]);
            }
        }
    }
    for i in 0..cells {
        for j in 0..cells {
            let at = |k| index_of.get(&k).copied();
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
        d.extend_from_slice(&[3, 3, 0, 0, 0, 9, 9, 9, LAST_BLOCK]);
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
        let g = ground(&tree, [-128.0, -128.0], [128.0, -1.0], 16.0, load);
        assert_eq!(asked, [1, 2]);
        // The second quadrant has no heights and the root none to fall back on.
        assert_eq!(g.tiles, 1);
        assert_eq!(g.positions.len(), 81);
        assert_eq!(g.indices.len(), 8 * 8 * 6);
        assert_eq!(g.positions[0], [-128.0, 5.0 / 32.0, -128.0]);
        assert_eq!(g.positions[80], [0.0, 133.0 / 32.0, 0.0]);
        // The fourth quadrant, at full spacing, misses the hole's four cells.
        let g = ground(&tree, [-128.0, 0.0], [0.0, 128.0], 0.5, |_| Ok(rows()));
        assert_eq!(g.positions.len(), 129 * 129 - 1);
        assert_eq!(g.colours.len(), g.positions.len());
        assert_eq!(g.indices.len(), (128 * 128 - 4) * 6);
        let g = ground(&tree, [0.0, 0.0], [128.0, 128.0], 1.0, |_| {
            Err(Error("no".into()))
        });
        assert_eq!((g.tiles, g.failed.len()), (0, 1));
    }

    #[test]
    fn each_leaf_is_coloured_by_its_own_colour_or_an_ancestors() {
        // The root and the first tile have grey heights; the second has no heights
        // and is green, so it takes its heights from the root and its own colour.
        let tree = Tree::parse(&sample_with_root(HEIGHTFIELD)).unwrap();
        let grey = coloured(rows(), block(0x8410, 0x8410, 0), block(0x8410, 0x8410, 0));
        let green = coloured(vec![], block(0x07E0, 0x07E0, 0), block(0x07E0, 0x07E0, 0));
        let g = ground(&tree, [-128.0, -128.0], [128.0, -1.0], 16.0, |id| {
            Ok(if id[0] == 2 {
                green.clone()
            } else {
                grey.clone()
            })
        });
        assert_eq!((g.tiles, g.colours.len()), (2, 2 * 81));
        assert!(g.colours[..81].iter().all(|&c| c == [131, 129, 131, 255]));
        assert!(g.colours[81..].iter().all(|&c| c == [0, 255, 0, 255]));
        // The second's chunk unreadable: it is drawn in the root's colour.
        let g = ground(&tree, [0.0, -128.0], [128.0, -1.0], 16.0, |id| {
            if id[0] == 2 {
                Err(Error("no".into()))
            } else {
                Ok(grey.clone())
            }
        });
        assert_eq!((g.tiles, g.failed.len()), (1, 1));
        assert!(g.colours.iter().all(|&c| c == [131, 129, 131, 255]));
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
        let heights: Vec<Option<Heightfield>> = tree
            .tiles
            .iter()
            .map(|t| {
                let (id, _) = t.chunk?;
                let chunk = chunks.read(&id).unwrap();
                Colour::parse(&chunk).unwrap();
                t.has_heights().then(|| Heightfield::parse(&chunk).unwrap())
            })
            .collect();
        assert_eq!(heights.iter().flatten().count(), 7_618);
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
    }
}
