//! Mesh geometry: a `RigidMeshAsset` or `CompositeMeshAsset` ebx names its
//! `MeshSetResource` res by id, and the res describes the LODs and their sections,
//! whose vertices and indices are in one chunk per LOD. See `docs/FORMATS.md`,
//! "Meshes".
//!
//! [`MeshSet::parse`] checks every pointer and table in the res before anything is
//! read; [`Lod::section`] decodes one section's positions, texture coordinates and
//! triangles from the LOD's chunk, checking each stream and every index. [`Meshes`]
//! finds a mesh's res and chunks in nfsh's store.

use std::collections::HashMap;

use crate::ebx::{Partition, Value};
use crate::reader::{Reader, cstr_at};
use crate::store::{Entries, Store};
use crate::{Location, Result, bail};

/// The res type of a mesh set: the hash of `MeshSet` (see [`crate::sb::res_type_hash`]).
pub const MESH_SET: u32 = 0x49B1_56D4;

/// The most LODs a mesh set's header has room for.
const MAX_LODS: usize = 7;
const LOD_SIZE: usize = 0xA0;
const SECTION_SIZE: usize = 0xE0;
const ELEMENTS: usize = 16;
/// An element slot that holds no element.
const NO_ELEMENT: [u8; 4] = [0, 0, 0xFF, 0];
/// The LOD's inline offset when its geometry is in its chunk.
const IN_CHUNK: u32 = u32::MAX;

/// Vertex element usages read so far.
pub const POSITION: u8 = 0x01;
pub const TEXCOORD0: u8 = 0x21;

/// Vertex element formats read so far, with their sizes.
pub const FLOAT2: u8 = 0x02;
pub const FLOAT3: u8 = 0x03;
pub const HALF2: u8 = 0x06;
pub const HALF4: u8 = 0x08;

/// Index formats (the LOD's word after its flags).
const INDEX_U16: u32 = 0x21;
const INDEX_U32: u32 = 0x2E;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    /// Whether `p` is inside, allowing `slack` on every side.
    pub fn contains(&self, p: [f32; 3], slack: f32) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] - slack && p[i] <= self.max[i] + slack)
    }
}

/// One vertex attribute: what it is, how it is stored, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Element {
    pub usage: u8,
    pub format: u8,
    /// Offset in its stream's vertex.
    pub offset: u8,
    pub stream: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub name: String,
    /// Index into the mesh asset's `Materials`.
    pub material: u32,
    pub triangles: u32,
    /// In indices, from the start of the LOD's index buffer.
    pub first_index: u32,
    /// In bytes, from the start of the LOD's chunk.
    pub vertex_offset: u32,
    pub vertices: u32,
    pub elements: Vec<Element>,
    /// Each stream's bytes per vertex.
    pub strides: [u16; ELEMENTS],
    /// The low byte of the word after the vertex count: the strides' sum.
    pub stride: u8,
    pub bounds: Bounds,
}

impl Section {
    /// Where stream `s` starts, from the start of the chunk: a section's streams are
    /// stored one after another, each with all the section's vertices.
    fn stream_start(&self, s: usize) -> u64 {
        let before: u64 = self.strides[..s].iter().map(|&w| u64::from(w)).sum();
        u64::from(self.vertex_offset) + u64::from(self.vertices) * before
    }

    /// The bytes all the section's streams take.
    pub fn vertex_bytes(&self) -> u64 {
        self.stream_start(ELEMENTS)
    }

    pub fn element(&self, usage: u8) -> Option<Element> {
        self.elements.iter().copied().find(|e| e.usage == usage)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lod {
    pub name: String,
    /// The chunk holding the geometry, as a bundle lists its id; `None` when the
    /// geometry is inside the res instead (character meshes; not read yet).
    pub chunk: Option<[u8; 16]>,
    pub flags: u32,
    /// 2 or 4 bytes an index.
    pub index_width: u8,
    /// The index buffer's size; it follows the vertices in the chunk.
    pub index_bytes: u32,
    /// The vertices' size: where the index buffer starts.
    pub vertex_bytes: u32,
    pub sections: Vec<Section>,
}

/// One section's geometry, ready to draw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Geometry {
    pub positions: Vec<[f32; 3]>,
    /// The first texture coordinates, if the section has them.
    pub uvs: Option<Vec<[f32; 2]>>,
    /// Three a triangle, counting from the section's first vertex.
    pub indices: Vec<u32>,
}

impl Lod {
    /// The chunk's size the LOD needs: its vertices, then its indices.
    pub fn chunk_bytes(&self) -> u64 {
        u64::from(self.vertex_bytes) + u64::from(self.index_bytes)
    }

    /// Decodes `section` from the LOD's chunk `data`.
    pub fn section(&self, section: &Section, data: &[u8]) -> Result<Geometry> {
        if (data.len() as u64) < self.chunk_bytes() {
            bail!(
                "{}: the chunk has {} bytes, the LOD needs {}",
                self.name,
                data.len(),
                self.chunk_bytes()
            );
        }
        if section.vertex_bytes() > u64::from(self.vertex_bytes) {
            bail!(
                "{}: section {} ends at {}, past the vertices ({})",
                self.name,
                section.name,
                section.vertex_bytes(),
                self.vertex_bytes
            );
        }
        let Some(position) = section.element(POSITION) else {
            bail!("{}: section {} has no positions", self.name, section.name);
        };
        let positions = self.attribute(section, position, data, |v| match position.format {
            FLOAT3 => Some([f32_at(v, 0)?, f32_at(v, 4)?, f32_at(v, 8)?]),
            HALF4 => Some([half_at(v, 0)?, half_at(v, 2)?, half_at(v, 4)?]),
            _ => None,
        })?;
        let uvs = match section.element(TEXCOORD0) {
            None => None,
            Some(uv) => Some(self.attribute(section, uv, data, |v| match uv.format {
                FLOAT2 => Some([f32_at(v, 0)?, f32_at(v, 4)?]),
                HALF2 => Some([half_at(v, 0)?, half_at(v, 2)?]),
                _ => None,
            })?),
        };

        let width = usize::from(self.index_width);
        let count = u64::from(section.triangles) * 3;
        let end = (u64::from(section.first_index) + count) * width as u64;
        if end > u64::from(self.index_bytes) {
            bail!(
                "{}: section {} indices end at byte {end}, past the index buffer ({})",
                self.name,
                section.name,
                self.index_bytes
            );
        }
        let start = self.vertex_bytes as usize + section.first_index as usize * width;
        let bytes = &data[start..start + count as usize * width];
        let indices: Vec<u32> = match width {
            2 => bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| u32::from(u16::from_le_bytes(b)))
                .collect(),
            _ => bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&b| u32::from_le_bytes(b))
                .collect(),
        };
        if let Some(&bad) = indices.iter().find(|&&i| i >= section.vertices) {
            bail!(
                "{}: section {} has index {bad}, past its {} vertices",
                self.name,
                section.name,
                section.vertices
            );
        }
        Ok(Geometry {
            positions,
            uvs,
            indices,
        })
    }

    /// Every vertex's `element`, decoded by `decode` from its bytes.
    fn attribute<T>(
        &self,
        section: &Section,
        element: Element,
        data: &[u8],
        decode: impl Fn(&[u8]) -> Option<T>,
    ) -> Result<Vec<T>> {
        let stream = usize::from(element.stream);
        let Some(&stride) = section.strides.get(stream) else {
            bail!("{}: element in stream {stream}", self.name);
        };
        // Inside the chunk: the caller checked that the section's streams are.
        let start = section.stream_start(stream) as usize;
        let (stride, offset) = (usize::from(stride), usize::from(element.offset));
        (0..section.vertices as usize)
            .map(|i| {
                let vertex = start + i * stride;
                let value = data
                    .get(vertex + offset.min(stride)..vertex + stride)
                    .and_then(&decode);
                match value {
                    Some(value) => Ok(value),
                    None => bail!(
                        "{}: section {}: element {:#04x} in format {:#04x} doesn't fit \
                         its {stride}-byte stream",
                        self.name,
                        section.name,
                        element.usage,
                        element.format
                    ),
                }
            })
            .collect()
    }
}

fn f32_at(v: &[u8], at: usize) -> Option<f32> {
    let b = v.get(at..at + 4)?;
    Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn half_at(v: &[u8], at: usize) -> Option<f32> {
    let b = v.get(at..at + 2)?;
    Some(half(u16::from_le_bytes([b[0], b[1]])))
}

/// An IEEE 754 half-precision float.
pub fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1F);
    let frac = f32::from(h & 0x3FF);
    sign * match exp {
        0 => frac * (-24f32).exp2(),
        0x1F if frac == 0.0 => f32::INFINITY,
        0x1F => f32::NAN,
        _ => (1.0 + frac / 1024.0) * ((exp - 15) as f32).exp2(),
    }
}

/// A mesh set res: the LODs and their sections.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshSet {
    pub name: String,
    /// 0 for a rigid mesh, 1 skinned, 2 composite.
    pub kind: u32,
    pub bounds: Bounds,
    pub lods: Vec<Lod>,
}

impl MeshSet {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let at = |pos: u64| -> Result<Reader> {
            match usize::try_from(pos) {
                Ok(pos) => Reader::at(data, pos),
                Err(_) => bail!("mesh set pointer {pos:#x} is out of range"),
            }
        };
        let string = |pos: u64| -> Result<String> {
            match usize::try_from(pos) {
                Ok(pos) => Ok(cstr_at(data, pos)?.to_owned()),
                Err(_) => bail!("mesh set string {pos:#x} is out of range"),
            }
        };
        let mut r = Reader::new(data);
        let bounds = read_bounds(&mut r)?;
        let lod_at: Vec<u64> = (0..MAX_LODS).map(|_| r.u64_le()).collect::<Result<_>>()?;
        let name = string(r.u64_le()?)?;
        r.u64_le()?; // the name without its folders
        r.u32_le()?; // the name's hash
        let kind = r.u32_le()?;
        let mut r = at(0x90)?;
        let lod_count = usize::from(r.u16_le()?);
        let section_count = usize::from(r.u16_le()?);
        if lod_count > MAX_LODS {
            bail!("{name}: {lod_count} LODs, at most {MAX_LODS} fit");
        }

        let mut lods = Vec::with_capacity(lod_count);
        for &pos in &lod_at[..lod_count] {
            let mut r = at(pos)?;
            r.bytes(LOD_SIZE)?; // the whole record is there
            let mut r = at(pos + 8)?;
            let sections = r.u32_le()? as usize;
            let sections_at = r.u64_le()?;
            if sections > section_count {
                bail!("{name}: a LOD has {sections} sections of {section_count}");
            }
            let mut r = at(pos + 0x50)?;
            let flags = r.u32_le()?;
            let index_width = match r.u32_le()? {
                INDEX_U16 => 2,
                INDEX_U32 => 4,
                other => bail!("{name}: index format {other:#x}"),
            };
            let index_bytes = r.u32_le()?;
            let vertex_bytes = r.u32_le()?;
            let id: [u8; 16] = r.array()?;
            let inline = r.u32_le()?;
            let lod_name = string(r.u64_le()?)?;
            let chunk = match (id == [0; 16], inline) {
                (false, IN_CHUNK) => Some(guid_order(id)),
                (true, offset) if offset != IN_CHUNK => None,
                _ => bail!("{lod_name}: chunk {id:02x?} with inline offset {inline:#x}"),
            };
            let sections = (0..sections as u64)
                .map(|s| read_section(data, sections_at + s * SECTION_SIZE as u64))
                .collect::<Result<Vec<_>>>()
                .map_err(|e| crate::Error(format!("{lod_name}: {e}")))?;
            lods.push(Lod {
                name: lod_name,
                chunk,
                flags,
                index_width,
                index_bytes,
                vertex_bytes,
                sections,
            });
        }
        let total: usize = lods.iter().map(|l| l.sections.len()).sum();
        if total != section_count {
            bail!("{name}: the LODs have {total} sections, the header says {section_count}");
        }
        Ok(Self {
            name,
            kind,
            bounds,
            lods,
        })
    }
}

fn read_bounds(r: &mut Reader) -> Result<Bounds> {
    let mut vec = || -> Result<[f32; 3]> {
        let v = [r.u32_le()?, r.u32_le()?, r.u32_le()?].map(f32::from_bits);
        r.u32_le()?; // padding to 16 bytes
        Ok(v)
    };
    Ok(Bounds {
        min: vec()?,
        max: vec()?,
    })
}

fn read_section(data: &[u8], pos: u64) -> Result<Section> {
    let Ok(pos) = usize::try_from(pos) else {
        bail!("section pointer {pos:#x} is out of range");
    };
    let mut r = Reader::at(data, pos)?;
    let record = r.bytes(SECTION_SIZE)?;
    let mut r = Reader::new(record);
    r.u64_le()?;
    let name_at = r.u64_le()?;
    let Ok(name_at) = usize::try_from(name_at) else {
        bail!("section name {name_at:#x} is out of range");
    };
    let name = cstr_at(data, name_at)?.to_owned();
    let material = r.u32_le()?;
    r.u32_le()?;
    let triangles = r.u32_le()?;
    let first_index = r.u32_le()?;
    let vertex_offset = r.u32_le()?;
    let vertices = r.u32_le()?;
    let stride = r.u8()?;
    let mut r = Reader::at(record, 0x40)?;
    let mut elements = Vec::new();
    for _ in 0..ELEMENTS {
        let e: [u8; 4] = r.array()?;
        if e != NO_ELEMENT {
            elements.push(Element {
                usage: e[0],
                format: e[1],
                offset: e[2],
                stream: e[3],
            });
        }
    }
    let mut strides = [0; ELEMENTS];
    for s in &mut strides {
        *s = r.u16_le()?;
    }
    if let Some(e) = elements.iter().find(|e| usize::from(e.stream) >= ELEMENTS) {
        bail!("section {name}: an element in stream {}", e.stream);
    }
    let mut r = Reader::at(record, 0xC0)?;
    let bounds = read_bounds(&mut r)?;
    Ok(Section {
        name,
        material,
        triangles,
        first_index,
        vertex_offset,
        vertices,
        elements,
        strides,
        stride,
        bounds,
    })
}

/// A GUID as stored in memory (a res, an ebx) in the order a bundle's chunk list
/// stores it: its first three fields byte-swapped.
pub fn guid_order(mut id: [u8; 16]) -> [u8; 16] {
    id[0..4].reverse();
    id[4..6].reverse();
    id[6..8].reverse();
    id
}

/// The chunk id a `.toc` lists in the order a bundle's chunk list stores it: a toc
/// stores the in-memory GUID with all 16 bytes reversed.
pub fn toc_chunk_id(mut id: [u8; 16]) -> [u8; 16] {
    id.reverse();
    guid_order(id)
}

/// The `MeshSetResource` id of the mesh asset in `p`: the first instance with one.
pub fn mesh_set_id(p: &Partition) -> Result<u64> {
    for instance in &p.instances {
        let o = p.object(instance);
        for f in p.fields_of(o.type_index)? {
            if p.name(f.name_hash) == Some("MeshSetResource") {
                match p.value(f, o.base)? {
                    Value::ResourceRef(id) if id != 0 => return Ok(id),
                    _ => bail!("{}: MeshSetResource names no res", p.name),
                }
            }
        }
    }
    bail!("{}: no instance has a MeshSetResource", p.name)
}

/// Finds meshes' res and chunks in a store.
pub struct Meshes<'s> {
    store: &'s Store,
    entries: Entries<'s>,
    /// Every mesh set res by id.
    sets: HashMap<u64, Location>,
    /// Every chunk by id (as bundles list it), with its size.
    chunks: HashMap<[u8; 16], Location>,
}

impl<'s> Meshes<'s> {
    pub fn new(store: &'s Store) -> Result<Self> {
        let mut sets = HashMap::new();
        let mut chunks = HashMap::new();
        for sb in &store.superbundles {
            for b in &sb.bundles {
                for r in b.bundle.res.iter().filter(|r| r.res_type == MESH_SET) {
                    sets.entry(r.id).or_insert(r.asset.location);
                }
                // A bundle stores only a chunk's blocks that cover its logical range;
                // from offset 0 that is the whole chunk.
                for c in b.bundle.chunks.iter().filter(|c| c.logical_offset == 0) {
                    chunks.entry(c.id).or_insert(c.location);
                }
            }
            for c in &sb.chunks {
                chunks
                    .entry(toc_chunk_id(c.chunk.id))
                    .or_insert(c.chunk.location);
            }
        }
        Ok(Self {
            store,
            entries: store.entries()?,
            sets,
            chunks,
        })
    }

    /// The mesh set of the mesh asset in the partition `guid`.
    pub fn by_partition(&mut self, guid: &[u8; 16]) -> Result<MeshSet> {
        let copy = self.store.partitions(guid)?.next();
        let Some((_, asset)) = copy.and_then(|at| self.store.ebx_at(at)) else {
            bail!("no partition {guid:02x?} in the store");
        };
        self.by_location(asset.location, &asset.name)
    }

    /// The mesh set of the mesh asset in the ebx `name`.
    pub fn by_name(&mut self, name: &str) -> Result<MeshSet> {
        let Some(asset) = self.store.ebx(name) else {
            bail!("no ebx named {name} in the store");
        };
        self.by_location(asset.location, name)
    }

    fn by_location(&mut self, location: Location, name: &str) -> Result<MeshSet> {
        let data = self.entries.read(location)?;
        let p = Partition::parse(&data).map_err(|e| crate::Error(format!("{name}: {e}")))?;
        let id = mesh_set_id(&p)?;
        let Some(&res) = self.sets.get(&id) else {
            bail!("{name}: no mesh set res {id:016x} in the store");
        };
        let data = self.entries.read(res)?;
        MeshSet::parse(&data).map_err(|e| crate::Error(format!("{name}: {e}")))
    }

    /// The chunk holding `lod`'s geometry.
    pub fn chunk(&mut self, lod: &Lod) -> Result<Vec<u8>> {
        let Some(id) = lod.chunk else {
            bail!("{}: the geometry is inline, not read yet", lod.name);
        };
        let Some(&location) = self.chunks.get(&id) else {
            bail!("{}: no chunk {id:02x?} in the store", lod.name);
        };
        self.entries.read(location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};
    use std::path::Path;

    use crate::level::Level;

    fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
        d[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A mesh set laid out as FORMATS.md describes, one LOD of one section with
    /// float3 positions and half2 texture coordinates in two streams, and its chunk:
    /// a triangle and its three 16-bit indices.
    fn sample() -> (Vec<u8>, Vec<u8>) {
        const LOD: usize = 0xA0;
        const SECTION: usize = LOD + LOD_SIZE;
        const NAMES: usize = SECTION + SECTION_SIZE;
        let mut d = vec![0u8; NAMES];
        d.extend_from_slice(b"props/box_mesh\0Mesh:props/box_mesh_lod0\0box_mat\0");
        let name = NAMES as u64;
        let lod_name = name + 15;
        let section_name = lod_name + 25;
        let bounds = |d: &mut Vec<u8>, at: usize| {
            for (i, v) in [0.0f32, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]
                .iter()
                .enumerate()
            {
                put(d, at + 4 * i, &v.to_le_bytes());
            }
        };
        bounds(&mut d, 0);
        put(&mut d, 0x20, &(LOD as u64).to_le_bytes());
        put(&mut d, 0x58, &name.to_le_bytes());
        put(&mut d, 0x90, &[1, 0, 1, 0]); // one LOD, one section

        put(&mut d, LOD + 8, &1u32.to_le_bytes());
        put(&mut d, LOD + 12, &(SECTION as u64).to_le_bytes());
        put(&mut d, LOD + 0x50, &0x40u32.to_le_bytes());
        put(&mut d, LOD + 0x54, &INDEX_U16.to_le_bytes());
        put(&mut d, LOD + 0x58, &8u32.to_le_bytes()); // 6 bytes of indices, to 4
        put(&mut d, LOD + 0x5C, &48u32.to_le_bytes()); // 3 × (12 + 4), to 16
        put(
            &mut d,
            LOD + 0x60,
            &[
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 9, 9, 9, 9, 9, 9, 9, 9,
            ],
        );
        put(&mut d, LOD + 0x70, &IN_CHUNK.to_le_bytes());
        put(&mut d, LOD + 0x74, &lod_name.to_le_bytes());

        put(&mut d, SECTION + 8, &section_name.to_le_bytes());
        put(&mut d, SECTION + 0x18, &1u32.to_le_bytes()); // a triangle
        put(&mut d, SECTION + 0x24, &3u32.to_le_bytes()); // three vertices
        put(&mut d, SECTION + 0x28, &[16, 3, 0, 0]);
        for e in 0..ELEMENTS {
            put(&mut d, SECTION + 0x40 + 4 * e, &NO_ELEMENT);
        }
        put(&mut d, SECTION + 0x40, &[POSITION, FLOAT3, 0, 0]);
        put(&mut d, SECTION + 0x44, &[TEXCOORD0, HALF2, 0, 1]);
        put(&mut d, SECTION + 0x80, &[12, 0, 4, 0]);
        bounds(&mut d, SECTION + 0xC0);

        let mut chunk = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            p.iter()
                .for_each(|v| chunk.extend_from_slice(&v.to_le_bytes()));
        }
        // 0.0, 0.5, 1.0 and 2.0 as halves
        for uv in [[0x0000u16, 0x3800], [0x3C00, 0x0000], [0x4000, 0x3C00]] {
            uv.iter()
                .for_each(|v| chunk.extend_from_slice(&v.to_le_bytes()));
        }
        chunk.resize(48, 0);
        for i in [0u16, 1, 2, 0] {
            chunk.extend_from_slice(&i.to_le_bytes());
        }
        (d, chunk)
    }

    #[test]
    fn the_res_type_is_the_hash_of_mesh_set() {
        assert_eq!(crate::sb::res_type_hash("MeshSet"), MESH_SET);
    }

    #[test]
    fn halves_convert() {
        assert_eq!(half(0x3C00), 1.0);
        assert_eq!(half(0xC000), -2.0);
        assert_eq!(half(0x0001), 2f32.powi(-24));
        assert_eq!(half(0x7C00), f32::INFINITY);
        assert!(half(0x7E00).is_nan());
    }

    #[test]
    fn reads_a_mesh_set_and_decodes_its_section() {
        let (res, chunk) = sample();
        let set = MeshSet::parse(&res).unwrap();
        assert_eq!(set.name, "props/box_mesh");
        assert_eq!(set.kind, 0);
        assert_eq!(set.bounds.max, [1.0, 1.0, 0.0]);
        let lod = &set.lods[0];
        assert_eq!(lod.name, "Mesh:props/box_mesh_lod0");
        assert_eq!(
            lod.chunk,
            Some([
                0x44, 0x33, 0x22, 0x11, 0x66, 0x55, 0x88, 0x77, 9, 9, 9, 9, 9, 9, 9, 9
            ])
        );
        assert_eq!(
            (lod.index_width, lod.index_bytes, lod.vertex_bytes),
            (2, 8, 48)
        );
        let s = &lod.sections[0];
        assert_eq!(
            (s.name.as_str(), s.triangles, s.vertices, s.stride),
            ("box_mat", 1, 3, 16)
        );
        assert_eq!(s.elements.len(), 2);
        let g = lod.section(s, &chunk).unwrap();
        assert_eq!(
            g.positions,
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        assert_eq!(g.uvs, Some(vec![[0.0, 0.5], [1.0, 0.0], [2.0, 1.0]]));
        assert_eq!(g.indices, [0, 1, 2]);
    }

    #[test]
    fn bad_geometry_is_an_error() {
        let (res, chunk) = sample();
        let set = MeshSet::parse(&res).unwrap();
        let lod = &set.lods[0];
        let s = &lod.sections[0];
        // An index past the section's vertices.
        let mut bad = chunk.clone();
        bad[50] = 3;
        assert!(lod.section(s, &bad).unwrap_err().0.contains("index 3"));
        // A chunk shorter than the LOD says.
        assert!(lod.section(s, &chunk[..50]).is_err());
        // More vertices than the vertex buffer holds.
        let mut more = s.clone();
        more.vertices = 4;
        assert!(lod.section(&more, &chunk).is_err());
        // More triangles than the index buffer holds.
        let mut more = s.clone();
        more.triangles = 2;
        assert!(lod.section(&more, &chunk).is_err());
        // A position format not read yet.
        let mut odd = s.clone();
        odd.elements[0].format = 0x7F;
        assert!(lod.section(&odd, &chunk).is_err());
    }

    #[test]
    fn a_damaged_res_is_an_error() {
        let (res, _) = sample();
        for cut in [0x10, 0x95, 0x120, 0x200, res.len() - 2] {
            assert!(MeshSet::parse(&res[..cut]).is_err(), "cut at {cut:#x}");
        }
        let mut far = res.clone();
        put(&mut far, 0x20, &u64::MAX.to_le_bytes());
        assert!(MeshSet::parse(&far).is_err());
        let mut lods = res.clone();
        put(&mut lods, 0x90, &[8, 0]);
        assert!(MeshSet::parse(&lods).is_err());
        let mut sections = res.clone();
        put(&mut sections, 0x92, &[2, 0]);
        assert!(MeshSet::parse(&sections).is_err());
        let mut format = res;
        put(&mut format, 0xA0 + 0x54, &7u32.to_le_bytes());
        assert!(MeshSet::parse(&format).is_err());
    }

    #[test]
    fn chunk_ids_in_toc_and_bundle_order() {
        let memory: [u8; 16] = std::array::from_fn(|i| i as u8);
        let bundle = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
        assert_eq!(guid_order(memory), bundle);
        let mut toc = memory;
        toc.reverse();
        assert_eq!(toc_chunk_id(toc), bundle);
    }

    /// Reads every mesh the 232 zones of Palm City place, in the store `NFSH_STORE`
    /// points at (local-ci sets it on the Windows side): every LOD's chunk, every
    /// section decoded, and the rules FORMATS.md gives checked on each.
    #[test]
    fn reads_every_mesh_palm_city_places() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let mut level = Level::new(&store, "Win32/levels/mainlevel/mainlevel").unwrap();
        let mut placed = Vec::new();
        let mut seen = HashSet::new();
        for z in level.sub_levels("levels/mainlevel/mainlevel").unwrap() {
            let zone = z.name.strip_prefix("levels/mainlevel/worldart_zs_");
            if !zone.is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())) {
                continue;
            }
            for m in level.zone(&z.name).unwrap().meshes {
                if seen.insert(m.partition) {
                    placed.push(m);
                }
            }
        }
        drop(level);

        let mut meshes = Meshes::new(&store).unwrap();
        let mut count: BTreeMap<&str, u64> = BTreeMap::new();
        let mut formats: BTreeMap<(u8, u8), u64> = BTreeMap::new();
        let mut broken: Vec<String> = Vec::new();
        let mut outliers: Vec<String> = Vec::new();
        for m in &placed {
            let set = meshes.by_partition(&m.partition).unwrap();
            *count.entry("meshes").or_default() += 1;
            for lod in &set.lods {
                *count.entry("lods").or_default() += 1;
                let data = meshes.chunk(lod).unwrap();
                if data.len() as u64 != lod.chunk_bytes().next_multiple_of(16) {
                    broken.push(format!("{}: chunk of {} bytes", lod.name, data.len()));
                }
                for s in &lod.sections {
                    *count.entry("sections").or_default() += 1;
                    for e in &s.elements {
                        *formats.entry((e.usage, e.format)).or_default() += 1;
                    }
                    let strides: u32 = s.strides.iter().map(|&w| u32::from(w)).sum();
                    if strides != u32::from(s.stride) {
                        broken.push(format!("{}: strides add to {strides}", lod.name));
                    }
                    let g = lod.section(s, &data).unwrap();
                    *count.entry("triangles").or_default() += u64::from(s.triangles);
                    *count.entry("vertices").or_default() += u64::from(s.vertices);
                    if g.uvs.is_some() {
                        *count.entry("sections with uvs").or_default() += 1;
                    }
                    // The bounds are the positions' before they were stored: floats
                    // keep them exactly, halves round by up to |x| / 2048.
                    let slack = match s.element(POSITION).map(|e| e.format) {
                        Some(HALF4) => {
                            (0..3)
                                .map(|i| s.bounds.min[i].abs().max(s.bounds.max[i].abs()))
                                .fold(0.0, f32::max)
                                / 1024.0
                        }
                        _ => 0.0,
                    };
                    let outside = g
                        .positions
                        .iter()
                        .filter(|&&p| !s.bounds.contains(p, slack));
                    if outside.count() > 0 {
                        outliers.push(format!("{}: {}", lod.name, s.name));
                    }
                }
            }
        }
        eprintln!("{count:?}");
        eprintln!("elements (usage, format) {formats:x?}");
        for b in broken.iter().take(20) {
            eprintln!("{b}");
        }
        assert!(broken.is_empty(), "{} broken", broken.len());
        // One section of one breakable prop has 15 used vertices up to 2 cm outside
        // its bounds, in each of its three LODs: the stored bounds, not the reading
        // (every other vertex, and the prop, are where they should be).
        assert_eq!(outliers.len(), 3, "{outliers:#?}");
        assert!(
            outliers
                .iter()
                .all(|o| o.contains("/prop_brk_abandoned_15_front_01_mesh_lod")),
            "{outliers:#?}"
        );
        // Counted on the whole install, 2026-10-31: each mesh once, every LOD.
        let count: Vec<(&str, u64)> = count.into_iter().collect();
        assert_eq!(
            count,
            [
                ("lods", 14_112),
                ("meshes", 5_475),
                ("sections", 53_194),
                ("sections with uvs", 35_110),
                ("triangles", 48_797_619),
                ("vertices", 50_467_702),
            ]
        );
    }
}
