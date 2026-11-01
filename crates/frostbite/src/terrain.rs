//! Palm City's terrain: the streaming tree's quadtree of tiles, a tile's heightfield,
//! and a mesh of the ground under an area (see docs/FORMATS.md, "Terrain").
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
/// Deeper than any tree the readers accept (Palm City's goes 9 levels down, to 64 m),
/// so a malformed file can't recurse without end.
const MAX_DEPTH: usize = 16;

/// A heightfield's samples along each side, its border on each side, and the cells
/// between the border's inner samples.
pub const SAMPLES: usize = 137;
pub const BORDER: usize = 4;
pub const CELLS: usize = 128;
/// A heightfield chunk's size is `HEIGHTFIELD + PAGES·k` bytes.
const HEIGHTFIELD: u32 = 42_231;
const PAGES: u32 = 18_496;
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

    fn overlaps(&self, lo: [f32; 2], hi: [f32; 2]) -> bool {
        (0..2).all(|k| self.min[k] < hi[k] && lo[k] < self.min[k] + self.size)
    }
}

/// The streaming tree's tiles; the first is the root.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub tiles: Vec<Tile>,
    /// The node tree's node count.
    pub nodes: usize,
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
        loop {
            if r.u8()? == LAST_BLOCK {
                break;
            }
            let len = r.u32_le()? as usize;
            r.bytes(len)?;
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
        Ok(Self { tiles, nodes })
    }

    /// Reads Palm City's tree from `store`.
    pub fn read(store: &Store) -> Result<Self> {
        let location = store
            .superbundles
            .iter()
            .flat_map(|sb| &sb.bundles)
            .flat_map(|b| &b.bundle.res)
            .find(|r| r.asset.name == STREAMING_TREE)
            .map(|r| r.asset.location)
            .ok_or_else(|| Error(format!("{STREAMING_TREE}: not in the store")))?;
        let data = store.entries()?.read(location)?;
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

/// The ground under an area, in world space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ground {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// How many leaf tiles it covers.
    pub tiles: usize,
    /// Tiles whose heightfield could not be read, with why; their area is drawn from
    /// a coarser tile's heights when there is one.
    pub failed: Vec<(usize, String)>,
}

/// The tile whose heights the walk draws from: its index, square and heights.
type Source = (usize, [f32; 2], f32, std::rc::Rc<Heightfield>);

/// The ground over `lo`..`hi` (x and z): every leaf tile overlapping it, drawn from
/// its own heightfield or its nearest ancestor's with one, at a vertex every
/// `spacing` metres or the heightfield's own spacing if coarser. Holes are left out.
pub fn ground(
    tree: &Tree,
    lo: [f32; 2],
    hi: [f32; 2],
    spacing: f32,
    mut load: impl FnMut(&[u8; 16]) -> Result<Vec<u8>>,
) -> Ground {
    let mut ground = Ground::default();
    if !tree.tiles.is_empty() {
        walk(tree, 0, None, lo, hi, spacing, &mut load, &mut ground);
    }
    ground
}

#[allow(clippy::too_many_arguments)]
fn walk(
    tree: &Tree,
    index: usize,
    source: Option<Source>,
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
    let mut source = source;
    if let Some((id, _)) = tile.chunk.filter(|_| tile.has_heights()) {
        match load(&id).and_then(|c| Heightfield::parse(&c)) {
            Ok(h) => source = Some((index, tile.min, tile.size, h.into())),
            Err(e) => ground.failed.push((index, e.0)),
        }
    }
    match tile.children {
        Some(children) => {
            for c in children {
                walk(
                    tree,
                    c as usize,
                    source.clone(),
                    lo,
                    hi,
                    spacing,
                    load,
                    ground,
                );
            }
        }
        None => {
            if let Some(s) = &source {
                ground.tiles += 1;
                emit(tile, s, spacing, ground);
            }
        }
    }
}

/// Adds `tile`'s square, drawn from the heights of `source` (itself or an ancestor).
fn emit(tile: &Tile, (_, min, size, heights): &Source, spacing: f32, ground: &mut Ground) {
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
                ground.positions.push([
                    min[0] + (column - BORDER) as f32 * cell,
                    y,
                    min[1] + (row - BORDER) as f32 * cell,
                ]);
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

    /// A 256 m tree from −128: the root, with four children, the third empty in the
    /// node tree; the root has no chunk, the first child a heightfield, the second
    /// none (2,312 bytes), and the third and fourth heightfields, the fourth with an
    /// extra chunk.
    fn sample() -> Vec<u8> {
        let mut d = vec![0; HEADER];
        d[TILE_COUNT_AT..TILE_COUNT_AT + 4].copy_from_slice(&5u32.to_le_bytes());
        d[NODE_COUNT_AT..NODE_COUNT_AT + 4].copy_from_slice(&5u32.to_le_bytes());
        node(&mut d, [-128.0, -128.0], 256.0, false, true, true);
        node(&mut d, [-128.0, -128.0], 128.0, false, false, false);
        node(&mut d, [0.0, -128.0], 128.0, false, false, false);
        node(&mut d, [0.0, 0.0], 128.0, true, false, false);
        node(&mut d, [-128.0, 0.0], 128.0, false, false, false);
        d.extend_from_slice(&[1, 3, 0, 0, 0, 9, 9, 9, LAST_BLOCK]);
        tile(&mut d, 0, 0, false, true);
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
        // Only the first two quadrants (z < 0), a vertex every 16 m (8 cells).
        let g = ground(&tree, [-128.0, -128.0], [128.0, -1.0], 16.0, load);
        assert_eq!(asked, [1]);
        // The second quadrant has no heights and the root none to fall back on.
        assert_eq!(g.tiles, 1);
        assert_eq!(g.positions.len(), 81);
        assert_eq!(g.indices.len(), 8 * 8 * 6);
        assert_eq!(g.positions[0], [-128.0, 5.0 / 32.0, -128.0]);
        assert_eq!(g.positions[80], [0.0, 133.0 / 32.0, 0.0]);
        // The fourth quadrant, at full spacing, misses the hole's four cells.
        let g = ground(&tree, [-128.0, 0.0], [0.0, 128.0], 0.5, |_| Ok(rows()));
        assert_eq!(g.positions.len(), 129 * 129 - 1);
        assert_eq!(g.indices.len(), (128 * 128 - 4) * 6);
        let g = ground(&tree, [0.0, 0.0], [128.0, 128.0], 1.0, |_| {
            Err(Error("no".into()))
        });
        assert_eq!((g.tiles, g.failed.len()), (0, 1));
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
        let heights: Vec<Option<Heightfield>> = tree
            .tiles
            .iter()
            .map(|t| {
                let (id, _) = t.chunk.filter(|_| t.has_heights())?;
                Some(Heightfield::parse(&chunks.read(&id).unwrap()).unwrap())
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
