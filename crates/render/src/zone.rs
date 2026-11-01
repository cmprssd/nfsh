//! Draws a [`Scene`] on the GPU: its buffers and textures uploaded once, one instanced
//! draw per mesh section, lit by a time of day's [`Light`]; opaque draws first, then
//! the ground, cut out, then decals blended over them.

use std::ffi::CStr;
use std::sync::Arc;

use ash::vk;
use frostbite::lighting::Lighting;
use frostbite::terrain::GroundMap;
use frostbite::texture::{Kind, Texture, bc1_mean};

use crate::Camera;
use crate::gpu::{Error, Gpu, Result};
use crate::resource::{Buffer, DEPTH_FORMAT, Image, Mip};
use crate::scene::{Draw, Instance, Scene, Shading};

const VERT: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.vs_main.spv")),
    c"vs_main",
);
/// The ground's shaders: its vertices carry weights where others have texture
/// coordinates.
const GROUND_VERT: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.vs_ground.spv")),
    c"vs_ground",
);
const GROUND: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.fs_ground.spv")),
    c"fs_ground",
);
/// The fragment shaders, opaque, cut out and decal, with their entry points.
const FRAG: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.fs_main.spv")),
    c"fs_main",
);
const CUTOUT: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.fs_cutout.spv")),
    c"fs_cutout",
);
const DECAL: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.fs_decal.spv")),
    c"fs_decal",
);
const ASPHALT: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.fs_asphalt.spv")),
    c"fs_asphalt",
);

/// The pipelines, in the order they draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pass {
    Opaque,
    Asphalt,
    Ground,
    Cutout,
    Decal,
}

impl Pass {
    fn of(shading: &Shading) -> Self {
        match shading {
            Shading::Colour | Shading::Grey { .. } => Self::Opaque,
            Shading::Asphalt(_) => Self::Asphalt,
            Shading::Ground => Self::Ground,
            Shading::Cutout => Self::Cutout,
            Shading::Decal { .. } => Self::Decal,
        }
    }
}

/// A draw's [`Shading`] as the fragment shader reads it (std140).
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Look {
    /// Multiplies the colour; `w` is 1 when the colour is the texture's red as grey.
    tint: [f32; 4],
    /// A decal's opacity: the texture's channels weighted by this…
    select: [f32; 4],
    /// …times `opacity[0]`.
    opacity: [f32; 4],
    /// A decal's colour; `w` is 1 when it is set, else the texture's red times the
    /// vertex colour.
    colour: [f32; 4],
    /// The ground's: each detail layer's scale (0 for none)…
    scale: [[f32; 4]; 2],
    /// …and the inverse of its average colour.
    inverse_mean: [[f32; 4]; 8],
    /// Asphalt's ([`crate::scene::Asphalt`]; its clean layer's tint is `tint`): the damaged
    /// layer's tint…
    tint_bg: [f32; 4],
    /// …both layers' tiling (clean, damaged)…
    tiling: [f32; 4],
    /// …the layers' blend and the wheel paths' (offset, contrast each)…
    blend: [f32; 4],
    /// …the wheel paths' tint, with the bias both blends add in `w`…
    tyre_tint: [f32; 4],
    /// …the detail colours…
    detail: [[f32; 4]; 5],
    /// …and its edge ([`crate::scene::Edge`]): the shoulder's tiling, with 1 in `z`
    /// where the road gives way to the ground…
    edge: [f32; 4],
    /// …the dirt's offset, contrast and paint boost…
    dirt: [f32; 4],
    /// …and where the ground map lies: its corner (x, z) and one over its size.
    ground_map: [f32; 4],
}

/// Where each look starts in the looks buffer: a multiple of 256 bytes, the most any
/// device may need a dynamic uniform offset aligned to.
const LOOK_STRIDE: usize = size_of::<Look>().next_multiple_of(256);

impl Look {
    fn new(shading: &Shading) -> Self {
        let mut look = Self {
            tint: [1.0, 1.0, 1.0, 0.0],
            select: [0.0; 4],
            opacity: [1.0, 0.0, 0.0, 0.0],
            colour: [0.0; 4],
            scale: [[0.0; 4]; 2],
            inverse_mean: [[0.0; 4]; 8],
            tint_bg: [0.0; 4],
            tiling: [0.0; 4],
            blend: [0.0; 4],
            tyre_tint: [0.0; 4],
            detail: [[0.0; 4]; 5],
            edge: [0.0; 4],
            dirt: [0.0; 4],
            ground_map: [0.0; 4],
        };
        let rgb = |c: [f32; 3], w| [c[0], c[1], c[2], w];
        match *shading {
            Shading::Colour | Shading::Cutout | Shading::Ground => {}
            Shading::Grey { tint } => look.tint = rgb(tint, 1.0),
            Shading::Asphalt(a) => {
                look.tint = rgb(a.tint[0], 1.0);
                look.tint_bg = rgb(a.tint[1], 0.0);
                let [[fu, fv], [bu, bv]] = a.tiling;
                look.tiling = [fu, fv, bu, bv];
                look.blend = [a.blend[0], a.blend[1], a.tyre[0], a.tyre[1]];
                look.tyre_tint = rgb(a.tyre_tint, a.bias);
                look.detail = a.detail.map(|c| rgb(c, 0.0));
                if let Some(e) = a.edge {
                    look.edge = [e.tiling[0], e.tiling[1], 1.0, 0.0];
                    look.dirt = rgb(e.dirt, 0.0);
                }
            }
            Shading::Decal {
                select,
                opacity,
                colour,
            } => {
                look.select = select;
                look.opacity[0] = opacity;
                if let Some(c) = colour {
                    look.colour = [c[0], c[1], c[2], 1.0];
                }
            }
        }
        look
    }
}

/// The shader's push constants (128 bytes, the most Vulkan 1.3 guarantees).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Push {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    sun: [f32; 4],
    /// The ground's light in `w`.
    sun_light: [f32; 4],
    sky: [f32; 4],
}

/// A time of day's light, as the shader takes it: already exposed, so 1 is white
/// before the tonemap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Towards the sun.
    pub sun: [f32; 3],
    /// What a white surface facing the sun gives back of it.
    pub sun_light: [f32; 3],
    /// What the sky and the ground give back from above and below (grey below).
    pub sky: [f32; 3],
    pub ground: f32,
}

impl Light {
    /// The sun is as bright as its illuminance makes a white diffuse surface
    /// (E / π), times the exposure. How bright the sky's light is has not been worked
    /// out (the sky's `LuminanceScale` is not in known units), so its colours are
    /// taken as they are: about what auto exposure leaves them at in the shade.
    pub fn new(l: &Lighting) -> Self {
        let exposed = l.sun_intensity / std::f32::consts::PI * l.exposure();
        let g = l.ground_colour;
        Self {
            sun: l.sun_direction(),
            sun_light: l.sun_colour.map(|c| c * exposed),
            sky: l.sky_colour,
            ground: (g[0] + g[1] + g[2]) / 3.0,
        }
    }
}

/// The Vulkan format with the same blocks as Heat's format `code`, where it is worked
/// out. Plain BC1 has no alpha; BC1 with 1-bit alpha keeps it.
pub fn vk_format(code: u32) -> Option<vk::Format> {
    use vk::Format as F;
    Some(match code {
        0x36 => F::BC1_RGB_UNORM_BLOCK,
        0x37 => F::BC1_RGB_SRGB_BLOCK,
        0x38 => F::BC1_RGBA_UNORM_BLOCK,
        0x39 => F::BC1_RGBA_SRGB_BLOCK,
        0x3C => F::BC3_UNORM_BLOCK,
        0x3D => F::BC3_SRGB_BLOCK,
        0x3E => F::BC4_UNORM_BLOCK,
        0x3F => F::BC5_UNORM_BLOCK,
        0x40 => F::BC6H_UFLOAT_BLOCK,
        0x42 => F::BC7_UNORM_BLOCK,
        0x43 => F::BC7_SRGB_BLOCK,
        _ => return None,
    })
}

/// Where each mip of a flat texture is in its chunk `data`, if they are all there.
pub fn mips(t: &Texture, data_len: usize) -> std::result::Result<Vec<Mip>, String> {
    if t.kind != Kind::Flat {
        return Err(format!("a {:?} texture, not a flat one", t.kind));
    }
    (0..t.mips())
        .map(|level| {
            let range = t
                .mip(level, 0)
                .filter(|r| r.end <= data_len)
                .ok_or_else(|| format!("mip {level} is past the chunk"))?;
            let (width, height) = t.mip_extent(level);
            Ok(Mip {
                offset: range.start as u64,
                width,
                height,
            })
        })
        .collect()
}

/// A texture's name, its header and its chunk's bytes.
pub type Loaded = (String, Texture, Vec<u8>);

/// The ground's detail as the shader takes it: each slot's scale, and its average
/// colour's inverse.
type DetailLook = ([[f32; 4]; 2], [[f32; 4]; 8]);

pub struct ZoneRenderer {
    gpu: Arc<Gpu>,
    /// One per [`Pass`], in its order.
    pipelines: [vk::Pipeline; 5],
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    /// The looks' set: one dynamic uniform buffer.
    look_layout: vk::DescriptorSetLayout,
    look_set: vk::DescriptorSet,
    sampler: vk::Sampler,
    pool: vk::DescriptorPool,
    /// One per combination of images the draws use ([`ImageSet`]).
    sets: Vec<vk::DescriptorSet>,
    /// The grey one first, then each texture that uploaded, then the ground's detail
    /// layers (a texture array).
    images: Vec<Image>,
    /// By pass; opaque and cut out draws sorted by their images, decals in the
    /// scene's order, so the later of two overlapping decals is drawn over the
    /// earlier. Each with its look's offset in the looks buffer and its set.
    draws: Vec<(Draw, u32, usize)>,
    /// `None` when the scene draws nothing (Vulkan has no empty buffers to bind).
    buffers: Option<[Buffer; 10]>,
    /// The looks, each at a multiple of [`LOOK_STRIDE`].
    looks: Option<Buffer>,
    /// Textures drawn grey, with why.
    pub missing: Vec<(String, String)>,
    /// The pixels uploaded, in bytes.
    pub texture_bytes: u64,
}

/// A draw's images, as indices into [`ZoneRenderer`]'s: its colour (binding 0),
/// asphalt's other layers (bindings 2 to 7) and its normal map (10), grey (a flat
/// normal) where it has none. Every set also binds the ground's map (8) and detail
/// layers (9).
type ImageSet = [usize; 8];

/// The bindings of an [`ImageSet`]'s images.
const SET_BINDINGS: [u32; 8] = [0, 2, 3, 4, 5, 6, 7, 10];

fn shader_module(gpu: &Gpu, spirv: &[u8]) -> Result<vk::ShaderModule> {
    let words = ash::util::read_spv(&mut std::io::Cursor::new(spirv))
        .map_err(|e| Error(format!("bad SPIR-V: {e}")))?;
    Ok(unsafe {
        gpu.device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)?
    })
}

impl ZoneRenderer {
    /// Uploads `scene` and its textures, each from `load` (by `TextureAsset`
    /// partition), and the ground's detail layers' textures `detail` (one per
    /// [`Scene::detail`]), and makes the pipelines for drawing into `color_format`. A
    /// texture that can't be loaded or sampled is drawn grey, a detail layer's left
    /// out, and listed in `missing`.
    pub fn new(
        gpu: &Arc<Gpu>,
        scene: &Scene,
        color_format: vk::Format,
        mut load: impl FnMut(&[u8; 16]) -> frostbite::Result<Loaded>,
        detail: Vec<frostbite::Result<Loaded>>,
    ) -> Result<Self> {
        let d = &gpu.device;
        // Everything is made into `r`, so whatever was made is destroyed by its Drop
        // if a later step fails.
        let mut r = Self {
            gpu: gpu.clone(),
            pipelines: [vk::Pipeline::null(); 5],
            layout: vk::PipelineLayout::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            look_layout: vk::DescriptorSetLayout::null(),
            look_set: vk::DescriptorSet::null(),
            sampler: vk::Sampler::null(),
            pool: vk::DescriptorPool::null(),
            sets: Vec::new(),
            images: Vec::new(),
            draws: Vec::new(),
            buffers: None,
            looks: None,
            missing: Vec::new(),
            texture_bytes: 0,
        };
        if !scene.draws.is_empty() {
            let upload = |usage, bytes: &[u8]| Buffer::upload(gpu, usage, bytes);
            let vertex = vk::BufferUsageFlags::VERTEX_BUFFER;
            r.buffers = Some([
                upload(vertex, bytemuck::cast_slice(&scene.positions))?,
                upload(vertex, bytemuck::cast_slice(&scene.instances))?,
                upload(vertex, bytemuck::cast_slice(&scene.uvs))?,
                upload(vertex, bytemuck::cast_slice(&scene.colours))?,
                upload(vertex, bytemuck::cast_slice(&scene.uvs2))?,
                upload(vertex, bytemuck::cast_slice(&scene.colours2))?,
                upload(vertex, bytemuck::cast_slice(&scene.uvs3))?,
                upload(vertex, bytemuck::cast_slice(&scene.normals))?,
                upload(vertex, bytemuck::cast_slice(&scene.tangents))?,
                upload(
                    vk::BufferUsageFlags::INDEX_BUFFER,
                    bytemuck::cast_slice(&scene.indices),
                )?,
            ]);
        }

        let mut sampler = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .max_lod(vk::LOD_CLAMP_NONE);
        if let Some(max) = gpu.max_anisotropy {
            sampler = sampler.anisotropy_enable(true).max_anisotropy(max.min(8.0));
        }
        r.sampler = unsafe { d.create_sampler(&sampler, None)? };
        let samplers = [r.sampler];
        let image = |binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        };
        let bindings = [
            image(0),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                .immutable_samplers(&samplers),
            image(2),
            image(3),
            image(4),
            image(5),
            image(6),
            image(7),
            image(8),
            image(9),
            image(10),
        ];
        r.set_layout = unsafe {
            d.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?
        };
        let ranges = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            offset: 0,
            size: size_of::<Push>() as u32,
        }];
        let look_bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
        r.look_layout = unsafe {
            d.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&look_bindings),
                None,
            )?
        };
        let set_layouts = [r.set_layout, r.look_layout];
        r.layout = unsafe {
            d.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&ranges),
                None,
            )?
        };
        for (at, (fragment, pass)) in [
            (FRAG, Pass::Opaque),
            (ASPHALT, Pass::Asphalt),
            (GROUND, Pass::Ground),
            (CUTOUT, Pass::Cutout),
            (DECAL, Pass::Decal),
        ]
        .into_iter()
        .enumerate()
        {
            r.pipelines[at] = pipeline(gpu, r.layout, color_format, fragment, pass)?;
        }

        // Grey (0.5, as linear values) for what has no texture.
        let grey = Mip {
            offset: 0,
            width: 1,
            height: 1,
        };
        let grey = Image::sampled(
            gpu,
            vk::Format::R8G8B8A8_UNORM,
            &[grey],
            &[128, 128, 128, 255],
        )?;
        r.images.push(grey);
        // Each of the scene's textures' image; the grey one for those that failed.
        let mut texture_images = Vec::with_capacity(scene.textures.len());
        for guid in &scene.textures {
            match r.upload(load(guid)) {
                Ok(image) => {
                    texture_images.push(r.images.len());
                    r.images.push(image);
                }
                Err((name, why)) => {
                    let name = name.unwrap_or_else(|| hex(guid));
                    r.missing.push((name, why));
                    texture_images.push(0);
                }
            }
        }
        let (ground, detail_look) = r.upload_detail(scene, detail)?;
        let ground_image = r.images.len();
        r.images.push(ground);
        let (map, map_look) = r.upload_ground_map(scene.ground_map.as_ref())?;
        let map_image = r.images.len();
        r.images.push(map);

        let image_of = |t: Option<u32>| {
            t.and_then(|t| texture_images.get(t as usize).copied())
                .unwrap_or(0)
        };
        let image_sets: Vec<ImageSet> = scene
            .draws
            .iter()
            .map(|draw| {
                let mut set = [0; 8];
                set[0] = image_of(draw.texture);
                for (at, layer) in set[1..7].iter_mut().zip(draw.layers) {
                    *at = image_of(layer);
                }
                set[7] = image_of(draw.normal_map);
                set
            })
            .collect();
        let mut distinct = image_sets.clone();
        distinct.sort_unstable();
        distinct.dedup();

        let count = distinct.len().max(1) as u32;
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: count * 10,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: count,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
                descriptor_count: 1,
            },
        ];
        r.pool = unsafe {
            d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(count + 1)
                    .pool_sizes(&sizes),
                None,
            )?
        };
        if !distinct.is_empty() {
            let layouts = vec![r.set_layout; distinct.len()];
            r.sets = unsafe {
                d.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(r.pool)
                        .set_layouts(&layouts),
                )?
            };
        }
        for (set, images) in r.sets.iter().zip(&distinct) {
            let shared = [(8, map_image), (9, ground_image)];
            for (binding, image) in SET_BINDINGS
                .into_iter()
                .zip(images.iter().copied())
                .chain(shared)
            {
                let info = [vk::DescriptorImageInfo::default()
                    .image_view(r.images[image].view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(binding)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&info);
                unsafe { d.update_descriptor_sets(&[write], &[]) };
            }
        }

        // Each distinct look once (a zone has a few dozen), at its offset.
        let mut looks: Vec<Look> = Vec::new();
        r.draws = scene
            .draws
            .iter()
            .zip(&image_sets)
            .map(|(draw, images)| {
                let mut look = Look::new(&draw.shading);
                // The ground, and asphalt where it gives way to it, draw its detail.
                let edge = matches!(draw.shading, Shading::Asphalt(a) if a.edge.is_some());
                if draw.shading == Shading::Ground || edge {
                    (look.scale, look.inverse_mean) = detail_look;
                }
                if edge {
                    look.ground_map = map_look;
                }
                let index = looks.iter().position(|l| *l == look).unwrap_or_else(|| {
                    looks.push(look);
                    looks.len() - 1
                });
                let set = distinct.binary_search(images).unwrap_or(0);
                (*draw, (index * LOOK_STRIDE) as u32, set)
            })
            .collect();
        r.draws.sort_by_key(|&(d, _, set)| {
            let pass = Pass::of(&d.shading);
            (pass, if pass == Pass::Decal { None } else { Some(set) })
        });
        if !looks.is_empty() {
            let mut bytes = vec![0u8; looks.len() * LOOK_STRIDE];
            for (look, at) in looks.iter().zip(bytes.chunks_mut(LOOK_STRIDE)) {
                at[..size_of::<Look>()].copy_from_slice(bytemuck::bytes_of(look));
            }
            let buffer = Buffer::upload(gpu, vk::BufferUsageFlags::UNIFORM_BUFFER, &bytes)?;
            let layouts = [r.look_layout];
            r.look_set = unsafe {
                d.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(r.pool)
                        .set_layouts(&layouts),
                )?[0]
            };
            let info = [vk::DescriptorBufferInfo::default()
                .buffer(buffer.buffer)
                .offset(0)
                .range(size_of::<Look>() as u64)];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(r.look_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .buffer_info(&info);
            unsafe { d.update_descriptor_sets(&[write], &[]) };
            r.looks = Some(buffer);
        }
        Ok(r)
    }

    /// The Vulkan format `texture` is sampled in, if this device can filter it.
    fn sampled_format(&self, texture: &Texture) -> std::result::Result<vk::Format, String> {
        let Some(format) = vk_format(texture.format) else {
            return Err(format!("format {:#04x} not worked out", texture.format));
        };
        let wanted = vk::FormatFeatureFlags::SAMPLED_IMAGE
            | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
        let props = unsafe {
            self.gpu
                .instance
                .get_physical_device_format_properties(self.gpu.physical, format)
        };
        if !props.optimal_tiling_features.contains(wanted) {
            return Err(format!("{format:?} can't be sampled on this device"));
        }
        Ok(format)
    }

    /// Uploads the ground's detail layers' textures as one array, a layer per slot,
    /// with each slot's look: its scale and its average colour's inverse. A texture
    /// that can't be read, or is not like the first that can (BC1, the same format,
    /// size and mips), leaves its slot without detail (scale 0, its layer zeros) and
    /// is listed in `missing`. With none, a grey array of one layer.
    fn upload_detail(
        &mut self,
        scene: &Scene,
        detail: Vec<frostbite::Result<Loaded>>,
    ) -> Result<(Image, DetailLook)> {
        let mut look: DetailLook = ([[0.0; 4]; 2], [[0.0; 4]; 8]);
        // The first usable layer's format, mips and size; then every slot's bytes.
        let mut like: Option<(vk::Format, Vec<Mip>, usize)> = None;
        let mut layers: Vec<Option<Vec<u8>>> = Vec::new();
        for (slot, (d, loaded)) in scene.detail.iter().zip(detail).take(8).enumerate() {
            let read = loaded.map_err(|e| e.0).and_then(|(_, texture, data)| {
                let format = self.sampled_format(&texture)?;
                let mips = mips(&texture, data.len())?;
                let mean = bc1_mean(&texture, &data).ok_or("not BC1: no average colour")?;
                if let Some((f, m, size)) = &like
                    && (*f, m, *size) != (format, &mips, data.len())
                {
                    return Err(format!(
                        "{format:?} {mips:?}, unlike the first detail layer"
                    ));
                }
                like.get_or_insert((format, mips, data.len()));
                Ok((mean, data))
            });
            match read {
                Ok((mean, data)) => {
                    look.0[slot / 4][slot % 4] = d.scale;
                    let [r, g, b] = mean.map(|m| 1.0 / m.max(1e-3));
                    look.1[slot] = [r, g, b, 0.0];
                    layers.push(Some(data));
                }
                Err(why) => {
                    self.missing.push((d.texture.clone(), why));
                    layers.push(None);
                }
            }
        }
        let gpu = self.gpu.clone();
        let Some((format, mips, size)) = like else {
            let grey = [Mip {
                offset: 0,
                width: 1,
                height: 1,
            }];
            let data = [128, 128, 128, 255];
            let image =
                Image::sampled_array(&gpu, vk::Format::R8G8B8A8_UNORM, &grey, (1, 4), &data)?;
            return Ok((image, ([[0.0; 4]; 2], [[0.0; 4]; 8])));
        };
        let mut data = Vec::with_capacity(size * layers.len());
        for layer in layers {
            match layer {
                Some(bytes) => data.extend(bytes),
                None => data.resize(data.len() + size, 0),
            }
        }
        let count = (data.len() / size) as u32;
        let image = Image::sampled_array(&gpu, format, &mips, (count, size as u64), &data)?;
        self.texture_bytes += data.len() as u64;
        Ok((image, look))
    }

    /// Uploads the ground's map as a three-layer array: its colours, then its weights
    /// (four in each layer); with its look: its corner and one over its size. Without
    /// a map, one grey texel without weights.
    fn upload_ground_map(&mut self, map: Option<&GroundMap>) -> Result<(Image, [f32; 4])> {
        let gpu = self.gpu.clone();
        let format = vk::Format::R8G8B8A8_UNORM;
        let Some(m) = map.filter(|m| m.width > 0 && m.height > 0) else {
            let one = [Mip {
                offset: 0,
                width: 1,
                height: 1,
            }];
            let data = [128, 128, 128, 255, 0, 0, 0, 0, 0, 0, 0, 0];
            return Ok((
                Image::sampled_array(&gpu, format, &one, (3, 4), &data)?,
                [0.0; 4],
            ));
        };
        let texels = m.width * m.height;
        let mut data: Vec<u8> = Vec::with_capacity(texels * 12);
        data.extend(m.colours.iter().flatten());
        data.resize(texels * 4, 0);
        for half in [0, 4] {
            data.extend(m.weights.iter().flat_map(|w| &w[half..half + 4]));
            data.resize(data.len().next_multiple_of(texels * 4), 0);
        }
        let mip = [Mip {
            offset: 0,
            width: m.width as u32,
            height: m.height as u32,
        }];
        let image = Image::sampled_array(&gpu, format, &mip, (3, texels as u64 * 4), &data)?;
        self.texture_bytes += data.len() as u64;
        let size = [m.width, m.height].map(|n| n as f32 * m.texel);
        Ok((image, [m.min[0], m.min[1], 1.0 / size[0], 1.0 / size[1]]))
    }

    /// Uploads one loaded texture; on failure, its name if known and why.
    fn upload(
        &mut self,
        loaded: frostbite::Result<Loaded>,
    ) -> std::result::Result<Image, (Option<String>, String)> {
        let (name, texture, data) = loaded.map_err(|e| (None, e.0))?;
        let fail = |why: String| (Some(name.clone()), why);
        let format = self.sampled_format(&texture).map_err(fail)?;
        let mips = mips(&texture, data.len()).map_err(fail)?;
        let image = Image::sampled(&self.gpu, format, &mips, &data).map_err(|e| fail(e.0))?;
        self.texture_bytes += data.len() as u64;
        Ok(image)
    }

    /// Records the zone's draws, inside rendering begun at `extent`.
    pub fn record(
        &self,
        cmd: vk::CommandBuffer,
        camera: &Camera,
        extent: vk::Extent2D,
        light: &Light,
    ) {
        let d = &self.gpu.device;
        let [s, l, k] = [light.sun, light.sun_light, light.sky];
        let push = Push {
            view_proj: camera.view_proj(),
            eye: [camera.origin[0], camera.origin[1], camera.origin[2], 1.0],
            sun: [s[0], s[1], s[2], 0.0],
            sun_light: [l[0], l[1], l[2], light.ground],
            sky: [k[0], k[1], k[2], 0.0],
        };
        unsafe {
            d.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: extent.width as f32,
                    height: extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            d.cmd_set_scissor(
                cmd,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                }],
            );
            let Some(
                [
                    positions,
                    instances,
                    uvs,
                    colours,
                    uvs2,
                    colours2,
                    uvs3,
                    normals,
                    tangents,
                    indices,
                ],
            ) = &self.buffers
            else {
                return;
            };
            d.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&push),
            );
            d.cmd_bind_vertex_buffers(
                cmd,
                0,
                &[
                    positions.buffer,
                    instances.buffer,
                    uvs.buffer,
                    colours.buffer,
                    uvs2.buffer,
                    colours2.buffer,
                    uvs3.buffer,
                    normals.buffer,
                    tangents.buffer,
                ],
                &[0; 9],
            );
            d.cmd_bind_index_buffer(cmd, indices.buffer, 0, vk::IndexType::UINT32);
            let (mut pipeline, mut set, mut look) = (None, None, None);
            for &(draw, offset, wanted) in &self.draws {
                let pass = Pass::of(&draw.shading);
                if pipeline != Some(pass) {
                    let p = self.pipelines[pass as usize];
                    d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, p);
                    pipeline = Some(pass);
                }
                if look != Some(offset) {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.layout,
                        1,
                        &[self.look_set],
                        &[offset],
                    );
                    look = Some(offset);
                }
                if set != Some(wanted) {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.layout,
                        0,
                        &[self.sets[wanted]],
                        &[],
                    );
                    set = Some(wanted);
                }
                d.cmd_draw_indexed(
                    cmd,
                    draw.index_count,
                    draw.instance_count,
                    draw.first_index,
                    draw.vertex_offset,
                    draw.first_instance,
                );
            }
        }
    }
}

fn hex(guid: &[u8; 16]) -> String {
    guid.iter().map(|b| format!("{b:02x}")).collect()
}

impl Drop for ZoneRenderer {
    fn drop(&mut self) {
        // Destroying a null handle does nothing, so a half-made renderer is fine.
        let d = &self.gpu.device;
        unsafe {
            d.destroy_descriptor_pool(self.pool, None);
            for p in self.pipelines {
                d.destroy_pipeline(p, None);
            }
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_descriptor_set_layout(self.look_layout, None);
            d.destroy_sampler(self.sampler, None);
        }
    }
}

/// Triangle lists: positions, three sets of texture coordinates and two colours per
/// vertex, the transform per instance; depth test (reversed depth) and write; no culling, since
/// transforms may mirror. `fragment` is the fragment shader's SPIR-V and entry point.
/// The ground has its own vertex shader, in world space, which reads the texture
/// coordinates' 8 bytes as two sets of four weights.
/// A decal blends by its alpha, doesn't write depth, and is pulled a little towards
/// the eye so it stays over the road it lies on.
fn pipeline(
    gpu: &Gpu,
    layout: vk::PipelineLayout,
    color_format: vk::Format,
    (fragment, entry): (&[u8], &CStr),
    pass: Pass,
) -> Result<vk::Pipeline> {
    let d = &gpu.device;
    let (vertex, vertex_entry) = if pass == Pass::Ground {
        GROUND_VERT
    } else {
        VERT
    };
    let vs = shader_module(gpu, vertex)?;
    let fs = match shader_module(gpu, fragment) {
        Ok(fs) => fs,
        Err(e) => {
            unsafe { d.destroy_shader_module(vs, None) };
            return Err(e);
        }
    };
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs)
            .name(vertex_entry),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs)
            .name(entry),
    ];
    let bindings = [
        vk::VertexInputBindingDescription {
            binding: 0,
            stride: 12,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vk::VertexInputBindingDescription {
            binding: 1,
            stride: size_of::<Instance>() as u32,
            input_rate: vk::VertexInputRate::INSTANCE,
        },
        vk::VertexInputBindingDescription {
            binding: 2,
            stride: 8,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vk::VertexInputBindingDescription {
            binding: 3,
            stride: 4,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vk::VertexInputBindingDescription {
            binding: 4,
            stride: 8,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vk::VertexInputBindingDescription {
            binding: 5,
            stride: 4,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vk::VertexInputBindingDescription {
            binding: 6,
            stride: 8,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vk::VertexInputBindingDescription {
            binding: 7,
            stride: 4,
            input_rate: vk::VertexInputRate::VERTEX,
        },
        vk::VertexInputBindingDescription {
            binding: 8,
            stride: 4,
            input_rate: vk::VertexInputRate::VERTEX,
        },
    ];
    let attribute = |location, binding, format, offset| vk::VertexInputAttributeDescription {
        location,
        binding,
        format,
        offset,
    };
    let row = vk::Format::R32G32B32A32_SFLOAT;
    let bytes = vk::Format::R8G8B8A8_UNORM;
    let mesh = [
        attribute(0, 0, vk::Format::R32G32B32_SFLOAT, 0),
        attribute(1, 1, row, 0),
        attribute(2, 1, row, 16),
        attribute(3, 1, row, 32),
        attribute(4, 2, vk::Format::R32G32_SFLOAT, 0),
        attribute(5, 3, bytes, 0),
        attribute(7, 4, vk::Format::R32G32_SFLOAT, 0),
        attribute(8, 5, bytes, 0),
        attribute(9, 6, vk::Format::R32G32_SFLOAT, 0),
        attribute(10, 7, vk::Format::R8G8B8A8_SNORM, 0),
        attribute(11, 8, vk::Format::R8G8B8A8_SNORM, 0),
    ];
    let ground = [
        attribute(0, 0, vk::Format::R32G32B32_SFLOAT, 0),
        attribute(4, 2, bytes, 0),
        attribute(5, 3, bytes, 0),
        attribute(6, 2, bytes, 4),
    ];
    let attributes: &[_] = if pass == Pass::Ground { &ground } else { &mesh };
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(attributes);
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let decal = pass == Pass::Decal;
    // With reversed depth, a positive bias moves towards the eye.
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .depth_bias_enable(decal)
        .depth_bias_constant_factor(if decal { 4.0 } else { 0.0 })
        .depth_bias_slope_factor(if decal { 2.0 } else { 0.0 })
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(!decal)
        .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL);
    let blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(decal)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ZERO)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE)
        .alpha_blend_op(vk::BlendOp::ADD)];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let color_formats = [color_format];
    let mut rendering = vk::PipelineRenderingCreateInfo::default()
        .color_attachment_formats(&color_formats)
        .depth_attachment_format(DEPTH_FORMAT);
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .depth_stencil_state(&depth)
        .color_blend_state(&blend)
        .dynamic_state(&dynamic)
        .layout(layout)
        .push_next(&mut rendering);
    let made = unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None) };
    unsafe {
        d.destroy_shader_module(vs, None);
        d.destroy_shader_module(fs, None);
    }
    Ok(made.map_err(|(_, e)| Error::from(e))?[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_is_exposed_and_the_sky_taken_as_it_is() {
        let l = Lighting {
            sun_colour: [1.0, 0.5, 0.25],
            sun_intensity: std::f32::consts::PI * 1000.0,
            sun_rotation: [0.0, 90.0],
            sky_colour: [0.3, 0.3, 0.4],
            ground_colour: [0.1, 0.2, 0.3],
            ev: [10.0, 10.0],
            exposure_compensation: 0.0,
        };
        let light = Light::new(&l);
        // 1000 cd/m² for white, at 1 / (1.2 × 1024).
        let white = 1000.0 / 1228.8;
        for (got, want) in light
            .sun_light
            .iter()
            .zip([white, white / 2.0, white / 4.0])
        {
            assert!((got - want).abs() < 1e-5, "{got} {want}");
        }
        assert!((light.sun[1] - 1.0).abs() < 1e-6);
        assert_eq!(light.sky, [0.3, 0.3, 0.4]);
        assert!((light.ground - 0.2).abs() < 1e-6);
    }

    #[test]
    fn formats_map_to_vulkan_blocks() {
        assert_eq!(vk_format(0x37), Some(vk::Format::BC1_RGB_SRGB_BLOCK));
        assert_eq!(vk_format(0x39), Some(vk::Format::BC1_RGBA_SRGB_BLOCK));
        assert_eq!(vk_format(0x43), Some(vk::Format::BC7_SRGB_BLOCK));
        assert_eq!(vk_format(0x3A), None);
        // Every format the reader names has a Vulkan one.
        for f in frostbite::texture::FORMATS {
            assert!(vk_format(f.code).is_some(), "{}", f.name);
        }
    }
}
