//! Textures: a `TextureAsset` ebx names its `Texture` res by id; the res is a fixed
//! header (kind, format, size, mip sizes) naming the chunk that holds the pixels, every
//! mip of every layer, block-compressed. See `docs/FORMATS.md`, "Textures".
//!
//! [`Texture::parse`] checks the header, including every mip's size against its format
//! where the format is worked out; [`Texture::mip`] finds one mip of one layer in the
//! chunk. [`materials`] lists what a mesh asset's materials name (their shader, textures
//! and vector parameters), and [`Textures`] finds a texture's res and chunk in nfsh's
//! store.

use std::collections::HashMap;
use std::ops::Range;

use crate::ebx::{Object, Partition, Ref, Value};
use crate::level::{elements, field, float, instance_object};
use crate::reader::Reader;
use crate::store::{Entries, Store, guid_order};
use crate::{Location, Result, bail};

/// The res type of a texture: the hash of `Texture` (see [`crate::sb::res_type_hash`]).
pub const TEXTURE: u32 = 0x6BDE_20BA;

/// The header's size; every texture res is exactly this long.
pub const HEADER_SIZE: usize = 0x84;
/// The most mips the header has room for.
pub const MAX_MIPS: usize = 15;

/// What a texture's layers are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One image (one layer).
    Flat,
    /// Six faces, as six layers.
    Cube,
    /// A volume, its depth slices as layers.
    Volume,
    /// An array of images of the same size.
    Array,
}

/// A pixel format: Heat's number for it, and what it is where that is worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub code: u32,
    pub name: &'static str,
    /// Bytes per 4×4 block.
    pub block: u32,
    /// The DXGI format with the same bytes (for a .dds file, or a Vulkan format).
    pub dxgi: u32,
}

/// The formats worked out so far: the block-compressed ones, from each mip's size and
/// from decoding them (see `docs/FORMATS.md`).
pub const FORMATS: [Format; 11] = [
    Format {
        code: 0x36,
        name: "BC1_UNORM",
        block: 8,
        dxgi: 71,
    },
    Format {
        code: 0x37,
        name: "BC1_SRGB",
        block: 8,
        dxgi: 72,
    },
    Format {
        code: 0x38,
        name: "BC1A_UNORM",
        block: 8,
        dxgi: 71,
    },
    Format {
        code: 0x39,
        name: "BC1A_SRGB",
        block: 8,
        dxgi: 72,
    },
    Format {
        code: 0x3C,
        name: "BC3_UNORM",
        block: 16,
        dxgi: 77,
    },
    Format {
        code: 0x3D,
        name: "BC3_SRGB",
        block: 16,
        dxgi: 78,
    },
    Format {
        code: 0x3E,
        name: "BC4_UNORM",
        block: 8,
        dxgi: 80,
    },
    Format {
        code: 0x3F,
        name: "BC5_UNORM",
        block: 16,
        dxgi: 83,
    },
    Format {
        code: 0x40,
        name: "BC6H_UF16",
        block: 16,
        dxgi: 95,
    },
    Format {
        code: 0x42,
        name: "BC7_UNORM",
        block: 16,
        dxgi: 98,
    },
    Format {
        code: 0x43,
        name: "BC7_SRGB",
        block: 16,
        dxgi: 99,
    },
];

/// The format Heat numbers `code`, if it is worked out.
pub fn format(code: u32) -> Option<Format> {
    FORMATS.iter().copied().find(|f| f.code == code)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub kind: Kind,
    /// Heat's format number; see [`Texture::format`].
    pub format: u32,
    /// Flags (not worked out; the same value as byte 4 of the res metadata).
    pub flags: u16,
    pub width: u16,
    pub height: u16,
    /// Layers: 1, 6 for a cube, the slices of an array or a volume.
    pub layers: u16,
    /// Each mip's size in bytes for one layer, largest first.
    pub mip_sizes: Vec<u32>,
    /// The first mips, which only the whole chunk holds: a bundle stores just the
    /// chunk's end, from the mip after them (see [`Texture::tail_offset`]).
    pub streamed: u8,
    /// The chunk holding the pixels, as a bundle lists its id.
    pub chunk: [u8; 16],
    /// The texture group (`Props`, `Props_NSMD`, …).
    pub group: String,
}

impl Texture {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() != HEADER_SIZE {
            bail!("texture header of {} bytes, not {HEADER_SIZE}", data.len());
        }
        let mut r = Reader::at(data, 8)?;
        let kind = match r.u32_le()? {
            0 => Kind::Flat,
            1 => Kind::Cube,
            2 => Kind::Volume,
            3 => Kind::Array,
            other => bail!("texture kind {other}"),
        };
        let format = r.u32_le()?;
        r.u32_le()?; // usually 0; a hash on a few character textures (not worked out)
        let flags = r.u16_le()?;
        let width = r.u16_le()?;
        let height = r.u16_le()?;
        let depth = r.u16_le()?;
        let layers = r.u16_le()?;
        let mips = usize::from(r.u8()?);
        let streamed = r.u8()?;
        let chunk = guid_order(r.array()?);
        let slots: Vec<u32> = (0..MAX_MIPS).map(|_| r.u32_le()).collect::<Result<_>>()?;
        let chain = r.u32_le()?;
        r.u32_le()?; // not worked out (not the group name's hash: close values for close groups)
        let group = r.bytes(HEADER_SIZE - 0x74)?;
        let group = group.split(|&b| b == 0).next().unwrap_or_default();
        let group = String::from_utf8_lossy(group).into_owned();

        if width == 0 || height == 0 || layers == 0 {
            bail!("texture of {width}×{height}, {layers} layers");
        }
        if depth != layers {
            bail!("texture depth {depth} and layers {layers} differ");
        }
        let expected_layers = match kind {
            Kind::Flat => Some(1),
            Kind::Cube => Some(6),
            Kind::Volume | Kind::Array => None,
        };
        if expected_layers.is_some_and(|n| n != layers) {
            bail!("{kind:?} texture with {layers} layers");
        }
        if mips == 0 || mips > MAX_MIPS || usize::from(streamed) > mips {
            bail!("texture of {mips} mips, {streamed} streamed");
        }
        let (mip_sizes, unused) = slots.split_at(mips);
        if unused.iter().any(|&s| s != 0) {
            bail!("texture of {mips} mips has sizes past them: {slots:?}");
        }
        let sum: u64 = mip_sizes.iter().map(|&s| u64::from(s)).sum();
        if sum != u64::from(chain) {
            bail!("texture mips add up to {sum} bytes, the header says {chain}");
        }
        let texture = Self {
            kind,
            format,
            flags,
            width,
            height,
            layers,
            mip_sizes: mip_sizes.to_vec(),
            streamed,
            chunk,
            group,
        };
        if let Some(f) = texture.format() {
            for (level, &size) in texture.mip_sizes.iter().enumerate() {
                let (w, h) = texture.mip_extent(level);
                let blocks = u64::from(w.div_ceil(4)) * u64::from(h.div_ceil(4));
                let expected = blocks * u64::from(f.block) * u64::from(texture.slices(level));
                if u64::from(size) != expected {
                    bail!(
                        "{} mip {level} ({w}×{h}) of {size} bytes, not {expected}",
                        f.name
                    );
                }
            }
        }
        Ok(texture)
    }

    pub fn format(&self) -> Option<Format> {
        format(self.format)
    }

    pub fn mips(&self) -> usize {
        self.mip_sizes.len()
    }

    /// Mip `level`'s width and height: halved each level, at least 1.
    pub fn mip_extent(&self, level: usize) -> (u32, u32) {
        let shrink = |n: u16| (u32::from(n) >> level.min(31)).max(1);
        (shrink(self.width), shrink(self.height))
    }

    /// The chunk's size: every mip of every layer.
    pub fn chunk_bytes(&self) -> u64 {
        self.offset(self.mips())
    }

    /// The depth slices in each of mip `level`'s sizes: a volume's halve with each mip,
    /// as its width and height do; other kinds give each layer's size, so 1.
    fn slices(&self, level: usize) -> u16 {
        match self.kind {
            Kind::Volume => (self.layers >> level.min(15)).max(1),
            _ => 1,
        }
    }

    /// Where mip `level` starts in the chunk: the mips are stored largest first, each
    /// with all its layers back to back (a volume's sizes already count its slices).
    fn offset(&self, level: usize) -> u64 {
        let before: u64 = self.mip_sizes[..level].iter().map(|&s| u64::from(s)).sum();
        match self.kind {
            Kind::Volume => before,
            _ => before * u64::from(self.layers),
        }
    }

    /// Where the mips a bundle stores start in the chunk: after the streamed ones.
    pub fn tail_offset(&self) -> u64 {
        self.offset(usize::from(self.streamed))
    }

    /// The bytes of mip `level` of `layer` in the chunk, if both exist (a volume has
    /// fewer slices in each smaller mip).
    pub fn mip(&self, level: usize, layer: u16) -> Option<Range<usize>> {
        let layers = match self.kind {
            Kind::Volume => self.slices(level),
            _ => self.layers,
        };
        if level >= self.mips() || layer >= layers {
            return None;
        }
        let size = u64::from(self.mip_sizes[level]) / u64::from(self.slices(level));
        let start = self.offset(level) + size * u64::from(layer);
        Some(usize::try_from(start).ok()?..usize::try_from(start + size).ok()?)
    }
}

/// One texture a material names: the shader parameter and the texture's partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureParameter {
    /// `Diffuse`, `Normal`, `SMD`, …
    pub name: String,
    /// The `TextureAsset` partition's GUID; `None` when the parameter names none.
    pub texture: Option<[u8; 16]>,
}

/// What one of a mesh asset's materials names: its shader's `Shader`,
/// `TextureParameters` and `VectorParameters`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Material {
    /// The shader's partition (e.g. `shaders/world/roads/rd_drv_blendwithterrain`);
    /// `None` when null.
    pub shader: Option<[u8; 16]>,
    pub textures: Vec<TextureParameter>,
    /// Each vector parameter's name and value (a scalar is in `x`, a colour in `xyz`).
    pub vectors: Vec<(String, [f32; 4])>,
}

impl Material {
    /// The texture the parameter `name` names, if it names one.
    pub fn texture(&self, name: &str) -> Option<[u8; 16]> {
        self.textures.iter().find(|t| t.name == name)?.texture
    }

    /// The value of the vector parameter `name`, if the material sets it.
    pub fn vector(&self, name: &str) -> Option<[f32; 4]> {
        Some(self.vectors.iter().find(|v| v.0 == name)?.1)
    }
}

/// Each of the mesh asset's `Materials`, in order (a section's material indexes this
/// list).
pub fn materials(p: &Partition) -> Result<Vec<Material>> {
    let mut asset = None;
    for instance in &p.instances {
        let o = p.object(instance);
        if field(p, o, "Materials")?.is_some() {
            asset = Some(o);
            break;
        }
    }
    let Some(asset) = asset else {
        bail!("{}: no instance has Materials", p.name);
    };
    let mut materials = Vec::new();
    for m in elements(p, asset, "Materials")? {
        let Value::Class(Ref::Internal(i)) = m else {
            bail!("{}: a material is {m:?}, not one of its instances", p.name);
        };
        let Some(Value::Struct(shader)) = field(p, instance_object(p, i)?, "Shader")? else {
            bail!("{}: material {i} has no Shader", p.name);
        };
        let mut material = Material {
            shader: import(p, field(p, shader, "Shader")?)?,
            ..Material::default()
        };
        for t in elements(p, shader, "TextureParameters")? {
            let Value::Struct(t) = t else {
                bail!("{}: a texture parameter is {t:?}", p.name);
            };
            material.textures.push(TextureParameter {
                name: parameter_name(p, t)?,
                texture: import(p, field(p, t, "Value")?)?,
            });
        }
        // Not every shader struct has vector parameters.
        if field(p, shader, "VectorParameters")?.is_some() {
            for v in elements(p, shader, "VectorParameters")? {
                let Value::Struct(v) = v else {
                    bail!("{}: a vector parameter is {v:?}", p.name);
                };
                let Some(Value::Struct(value)) = field(p, v, "Value")? else {
                    bail!("{}: a vector parameter has no Vec4 Value", p.name);
                };
                let [x, y, z, w] = ["x", "y", "z", "w"].map(|c| float(p, value, c));
                material
                    .vectors
                    .push((parameter_name(p, v)?, [x?, y?, z?, w?]));
            }
        }
        materials.push(material);
    }
    Ok(materials)
}

fn parameter_name(p: &Partition, o: Object) -> Result<String> {
    let Some(Value::String(name)) = field(p, o, "ParameterName")? else {
        bail!("{}: a shader parameter has no ParameterName", p.name);
    };
    Ok(name.unwrap_or_default().to_owned())
}

/// The partition a reference to another partition's instance names; `None` for null.
fn import(p: &Partition, v: Option<Value>) -> Result<Option<[u8; 16]>> {
    match v {
        Some(Value::Class(Ref::Null)) => Ok(None),
        Some(Value::Class(Ref::Import(n))) => match p.imports.get(n) {
            Some(import) => Ok(Some(import.partition)),
            None => bail!("{}: import {n} is out of range", p.name),
        },
        other => bail!(
            "{}: {other:?} is not a reference to another partition",
            p.name
        ),
    }
}

/// Finds textures' res and chunks in a store.
pub struct Textures<'s> {
    store: &'s Store,
    entries: Entries<'s>,
    /// Every texture res by id.
    res: HashMap<u64, Location>,
    /// Every chunk stored whole, by id.
    chunks: HashMap<[u8; 16], Location>,
}

impl<'s> Textures<'s> {
    pub fn new(store: &'s Store) -> Result<Self> {
        Ok(Self {
            store,
            entries: store.entries()?,
            res: store.res_of_type(TEXTURE),
            chunks: store.whole_chunks(),
        })
    }

    /// The texture asset in the partition `guid`, with its name.
    pub fn by_partition(&mut self, guid: &[u8; 16]) -> Result<(String, Texture)> {
        let copy = self.store.partitions(guid)?.next();
        let Some((_, asset)) = copy.and_then(|at| self.store.ebx_at(at)) else {
            bail!("no partition {guid:02x?} in the store");
        };
        let name = asset.name.clone();
        let texture = self.by_location(asset.location, &name)?;
        Ok((name, texture))
    }

    /// The texture asset in the ebx `name`.
    pub fn by_name(&mut self, name: &str) -> Result<Texture> {
        let Some(asset) = self.store.ebx(name) else {
            bail!("no ebx named {name} in the store");
        };
        self.by_location(asset.location, name)
    }

    fn by_location(&mut self, location: Location, name: &str) -> Result<Texture> {
        let data = self.entries.read(location)?;
        let p = Partition::parse(&data).map_err(|e| crate::Error(format!("{name}: {e}")))?;
        let id = p.resource("Resource")?;
        let Some(&res) = self.res.get(&id) else {
            bail!("{name}: no texture res {id:016x} in the store");
        };
        let data = self.entries.read(res)?;
        Texture::parse(&data).map_err(|e| crate::Error(format!("{name}: {e}")))
    }

    /// The chunk holding `texture`'s pixels, checked to be exactly its mips.
    pub fn pixels(&mut self, texture: &Texture) -> Result<Vec<u8>> {
        let id = texture.chunk;
        let Some(&location) = self.chunks.get(&id) else {
            bail!("no whole chunk {id:02x?} in the store");
        };
        let data = self.entries.read(location)?;
        if data.len() as u64 != texture.chunk_bytes() {
            bail!(
                "chunk {id:02x?} of {} bytes, the texture's mips take {}",
                data.len(),
                texture.chunk_bytes()
            );
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};
    use std::path::Path;

    use crate::ebx::tests::{Builder, F, V};
    use crate::level::Level;

    fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
        d[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A texture res: `kind`, `format`, `width`×`height`, `layers`, with these mip sizes.
    fn header(kind: u32, format: u32, size: (u16, u16), layers: u16, mips: &[u32]) -> Vec<u8> {
        let mut d = vec![0; HEADER_SIZE];
        put(&mut d, 0x08, &kind.to_le_bytes());
        put(&mut d, 0x0C, &format.to_le_bytes());
        put(&mut d, 0x14, &1u16.to_le_bytes());
        put(&mut d, 0x16, &size.0.to_le_bytes());
        put(&mut d, 0x18, &size.1.to_le_bytes());
        put(&mut d, 0x1A, &layers.to_le_bytes());
        put(&mut d, 0x1C, &layers.to_le_bytes());
        d[0x1E] = mips.len() as u8;
        d[0x1F] = 1;
        put(&mut d, 0x20, &std::array::from_fn::<u8, 16, _>(|i| i as u8));
        for (i, s) in mips.iter().enumerate() {
            put(&mut d, 0x30 + 4 * i, &s.to_le_bytes());
        }
        put(&mut d, 0x6C, &mips.iter().sum::<u32>().to_le_bytes());
        put(&mut d, 0x74, b"Props\0");
        d
    }

    #[test]
    fn the_res_type_is_the_hash_of_texture() {
        assert_eq!(crate::sb::res_type_hash("Texture"), TEXTURE);
    }

    #[test]
    fn reads_a_header() {
        // BC1, 16×8: 4×2 blocks, then 2×1, 1×1, 1×1 (a block at least).
        let t = Texture::parse(&header(0, 0x37, (16, 8), 1, &[64, 16, 8, 8])).unwrap();
        assert_eq!(t.kind, Kind::Flat);
        assert_eq!(t.format().map(|f| f.name), Some("BC1_SRGB"));
        assert_eq!((t.width, t.height, t.layers, t.flags), (16, 8, 1, 1));
        assert_eq!(
            t.chunk,
            [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15]
        );
        assert_eq!(t.group, "Props");
        assert_eq!(t.mip_extent(3), (2, 1));
        assert_eq!(t.chunk_bytes(), 96);
        assert_eq!(t.tail_offset(), 64);
        assert_eq!(t.mip(1, 0), Some(64..80));
        assert_eq!(t.mip(4, 0), None);
        assert_eq!(t.mip(0, 1), None);
    }

    #[test]
    fn each_mip_holds_all_its_layers() {
        // A BC7 array of 3 layers, 8×8: mips of 64 and 16 bytes a layer.
        let t = Texture::parse(&header(3, 0x42, (8, 8), 3, &[64, 16, 16, 16])).unwrap();
        assert_eq!(t.kind, Kind::Array);
        assert_eq!(t.chunk_bytes(), 3 * 112);
        assert_eq!(t.mip(0, 2), Some(128..192));
        assert_eq!(t.mip(1, 0), Some(192..208));
        assert_eq!(t.mip(3, 2), Some(320..336));
        assert_eq!(t.tail_offset(), 192);
    }

    #[test]
    fn a_volume_counts_its_slices_in_each_mip() {
        // BC4, 8×8×4: mip 0 is 4 slices of 4 blocks, mip 1 two slices of one.
        let t = Texture::parse(&header(2, 0x3E, (8, 8), 4, &[128, 16, 8])).unwrap();
        assert_eq!(t.kind, Kind::Volume);
        assert_eq!(t.chunk_bytes(), 152);
        assert_eq!(t.mip(0, 3), Some(96..128));
        assert_eq!(t.mip(1, 1), Some(136..144));
        assert_eq!(t.mip(1, 2), None);
        assert_eq!(t.mip(2, 0), Some(144..152));
        assert!(Texture::parse(&header(2, 0x3E, (8, 8), 4, &[32, 8, 8])).is_err());
    }

    #[test]
    fn a_format_not_worked_out_is_read_without_its_sizes_checked() {
        let t = Texture::parse(&header(0, 0x12, (4, 4), 1, &[123])).unwrap();
        assert_eq!(t.format(), None);
        assert_eq!(t.chunk_bytes(), 123);
    }

    #[test]
    fn a_damaged_header_is_an_error() {
        let good = header(1, 0x40, (4, 4), 6, &[16, 16, 16]);
        assert!(Texture::parse(&good).is_ok());
        assert!(Texture::parse(&good[..HEADER_SIZE - 1]).is_err());
        let damaged = [
            (0x08, vec![4, 0, 0, 0]),  // kind
            (0x1C, vec![5, 0]),        // a cube of 5 layers
            (0x1A, vec![7, 0]),        // depth and layers differ
            (0x16, vec![0, 0]),        // no width
            (0x1E, vec![0]),           // no mips
            (0x1E, vec![16]),          // more than fit
            (0x1F, vec![4]),           // more streamed than there are
            (0x30, vec![32, 0, 0, 0]), // a BC6H 4×4 mip is 16 bytes
            (0x3C, vec![16, 0, 0, 0]), // a size past the mips
            (0x6C, vec![47, 0, 0, 0]), // the sizes add up to 48
        ];
        for (at, bytes) in damaged {
            let mut d = good.clone();
            put(&mut d, at, &bytes);
            assert!(Texture::parse(&d).is_err(), "{at:#x}");
        }
    }

    #[test]
    fn lists_what_each_material_names() {
        let diffuse = [0xD1; 16];
        let normal = [0xA1; 16];
        let shader = [0x5A; 16];
        let p = Builder::default()
            .ty(
                "TextureShaderParameter",
                false,
                &[("Value", F::Ref), ("ParameterName", F::Str)],
            )
            .array(
                "TextureShaderParameter[]",
                F::Struct("TextureShaderParameter"),
            )
            .ty(
                "Vec4",
                false,
                &[("x", F::F32), ("y", F::F32), ("z", F::F32), ("w", F::F32)],
            )
            .ty(
                "VectorShaderParameter",
                false,
                &[("Value", F::Struct("Vec4")), ("ParameterName", F::Str)],
            )
            .array(
                "VectorShaderParameter[]",
                F::Struct("VectorShaderParameter"),
            )
            .ty(
                "SurfaceShaderInstanceDataStruct",
                false,
                &[
                    ("VectorParameters", F::Array("VectorShaderParameter[]")),
                    ("TextureParameters", F::Array("TextureShaderParameter[]")),
                    ("Shader", F::Ref),
                ],
            )
            .ty(
                "MeshMaterial",
                true,
                &[("Shader", F::Struct("SurfaceShaderInstanceDataStruct"))],
            )
            .array("DataContainer[]", F::Ref)
            .ty(
                "RigidMeshAsset",
                true,
                &[("Materials", F::Array("DataContainer[]"))],
            )
            .import(diffuse, [1; 16])
            .import(normal, [2; 16])
            .import(shader, [3; 16])
            .instance(
                "RigidMeshAsset",
                [0x10; 16],
                vec![V::Array(vec![V::Ref(3), V::Ref(2)])],
            )
            .instance(
                "MeshMaterial",
                [0x11; 16],
                vec![V::Struct(vec![
                    V::Array(vec![V::Struct(vec![
                        V::Struct([0.2, 0.3, 0.4, 1.0].map(V::F32).to_vec()),
                        V::Str("Tint"),
                    ])]),
                    V::Array(vec![
                        V::Struct(vec![V::Ref(0x8000_0000), V::Str("Diffuse")]),
                        V::Struct(vec![V::Ref(0x8000_0001), V::Str("Normal")]),
                    ]),
                    V::Ref(0x8000_0002),
                ])],
            )
            .instance(
                "MeshMaterial",
                [0x12; 16],
                vec![V::Struct(vec![
                    V::Array(vec![]),
                    V::Array(vec![V::Struct(vec![V::Ref(0), V::Str("Mask")])]),
                    V::Ref(0),
                ])],
            )
            .build("props/crate_mesh", [0x20; 16]);
        let p = Partition::parse(&p).unwrap();
        let param = |name: &str, texture| TextureParameter {
            name: name.to_owned(),
            texture,
        };
        let found = materials(&p).unwrap();
        assert_eq!(
            found,
            [
                Material {
                    shader: None,
                    textures: vec![param("Mask", None)],
                    vectors: vec![],
                },
                Material {
                    shader: Some(shader),
                    textures: vec![
                        param("Diffuse", Some(diffuse)),
                        param("Normal", Some(normal))
                    ],
                    vectors: vec![("Tint".to_owned(), [0.2, 0.3, 0.4, 1.0])],
                },
            ]
        );
        assert_eq!(found[1].texture("Normal"), Some(normal));
        assert_eq!(found[0].texture("Mask"), None);
        assert_eq!(found[1].vector("Tint"), Some([0.2, 0.3, 0.4, 1.0]));
        assert_eq!(found[1].vector("Opacity"), None);
    }

    #[test]
    fn reads_every_texture_palm_city_places() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let mut level = Level::new(&store, "Win32/levels/mainlevel/mainlevel").unwrap();
        let mut meshes = HashSet::new();
        for z in level.sub_levels("levels/mainlevel/mainlevel").unwrap() {
            let zone = z.name.strip_prefix("levels/mainlevel/worldart_zs_");
            if !zone.is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())) {
                continue;
            }
            meshes.extend(
                level
                    .zone(&z.name)
                    .unwrap()
                    .meshes
                    .iter()
                    .map(|m| m.partition),
            );
        }
        drop(level);

        let mut entries = store.entries().unwrap();
        let mut count: BTreeMap<&str, u64> = BTreeMap::new();
        let mut used = HashSet::new();
        let mut meshes: Vec<_> = meshes.into_iter().collect();
        meshes.sort_unstable();
        for guid in &meshes {
            let at = store.partitions(guid).unwrap().next().unwrap();
            let (_, asset) = store.ebx_at(at).unwrap();
            let data = entries.read(asset.location).unwrap();
            let p = Partition::parse(&data).unwrap();
            for material in materials(&p).unwrap() {
                *count.entry("materials").or_default() += 1;
                for t in material.textures {
                    *count.entry("texture parameters").or_default() += 1;
                    used.extend(t.texture);
                }
            }
        }

        // Where a bundle stores only the end of a chunk: the first such entry by id.
        let mut tails = HashMap::new();
        for b in store.superbundles.iter().flat_map(|sb| &sb.bundles) {
            for c in b.bundle.chunks.iter().filter(|c| c.logical_offset > 0) {
                tails.entry(c.id).or_insert(c);
            }
        }

        let mut textures = Textures::new(&store).unwrap();
        let mut formats: BTreeMap<(String, String), u64> = BTreeMap::new();
        let mut used: Vec<_> = used.into_iter().collect();
        used.sort_unstable();
        for guid in &used {
            let (name, t) = textures.by_partition(guid).unwrap();
            let format = t
                .format()
                .map_or(format!("{:#04x}", t.format), |f| f.name.into());
            *formats
                .entry((format, format!("{:?}", t.kind)))
                .or_default() += 1;
            *count.entry("textures").or_default() += 1;
            let pixels = textures
                .pixels(&t)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            *count.entry("bytes").or_default() += pixels.len() as u64;
            if let Some(tail) = tails.get(&t.chunk) {
                *count.entry("bundle tails").or_default() += 1;
                let end = u64::from(tail.logical_offset) + u64::from(tail.logical_size);
                assert_eq!(u64::from(tail.logical_offset), t.tail_offset(), "{name}");
                assert_eq!(end, t.chunk_bytes(), "{name}");
                // The bundle's entry starts at the 0x10000 boundary before the offset.
                let stored = entries.read(tail.location).unwrap();
                let from = tail.logical_offset as usize & !0xFFFF;
                assert_eq!(stored[..], pixels[from..], "{name}");
            }
        }
        eprintln!("{count:?}");
        eprintln!("{formats:?}");
        // Counted on the whole install, 2026-10-31: each texture once.
        let count: Vec<(&str, u64)> = count.into_iter().collect();
        assert_eq!(
            count,
            [
                ("bundle tails", 7_010),
                ("bytes", 5_032_130_944),
                ("materials", 14_806),
                ("texture parameters", 37_345),
                ("textures", 8_015),
            ]
        );
        let formats: Vec<(&str, u64)> = formats
            .iter()
            .map(|((format, kind), &n)| {
                assert_eq!(kind, "Flat", "{format}");
                (format.as_str(), n)
            })
            .collect();
        assert_eq!(
            formats,
            [
                ("BC1A_SRGB", 13),
                ("BC1A_UNORM", 1),
                ("BC1_SRGB", 1_069),
                ("BC1_UNORM", 1_244),
                ("BC3_SRGB", 772),
                ("BC3_UNORM", 633),
                ("BC4_UNORM", 728),
                ("BC5_UNORM", 14),
                ("BC6H_UF16", 590),
                ("BC7_SRGB", 1_537),
                ("BC7_UNORM", 1_414),
            ]
        );
    }
}
