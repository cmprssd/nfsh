//! Draws a [`Scene`] on the GPU: its buffers and textures uploaded once, one instanced
//! draw per mesh section, lit by a time of day's [`Light`].

use std::ffi::CStr;
use std::sync::Arc;

use ash::vk;
use frostbite::lighting::Lighting;
use frostbite::texture::{Kind, Texture};

use crate::Camera;
use crate::gpu::{Error, Gpu, Result};
use crate::resource::{Buffer, DEPTH_FORMAT, Image, Mip};
use crate::scene::{Draw, Instance, Scene};

const VERT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/zone.vs_main.spv"));
/// The fragment shaders, opaque and cut out, with their entry points.
const FRAG: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.fs_main.spv")),
    c"fs_main",
);
const CUTOUT: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/zone.fs_cutout.spv")),
    c"fs_cutout",
);

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

pub struct ZoneRenderer {
    gpu: Arc<Gpu>,
    /// Opaque, then cut out.
    pipelines: [vk::Pipeline; 2],
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    pool: vk::DescriptorPool,
    /// One per image: the grey one first, then each texture that uploaded.
    sets: Vec<vk::DescriptorSet>,
    images: Vec<Image>,
    /// Each of the scene's textures' set; the grey one's for those that failed.
    texture_sets: Vec<usize>,
    /// `None` when the scene draws nothing (Vulkan has no empty buffers to bind).
    buffers: Option<[Buffer; 4]>,
    /// Opaque draws first, then cut out, each sorted by texture.
    draws: Vec<Draw>,
    /// Textures drawn grey, with why.
    pub missing: Vec<(String, String)>,
    /// The pixels uploaded, in bytes.
    pub texture_bytes: u64,
}

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
    /// partition), and makes the pipelines for drawing into `color_format`. A texture
    /// that can't be loaded or sampled is drawn grey and listed in `missing`.
    pub fn new(
        gpu: &Arc<Gpu>,
        scene: &Scene,
        color_format: vk::Format,
        mut load: impl FnMut(&[u8; 16]) -> frostbite::Result<Loaded>,
    ) -> Result<Self> {
        let d = &gpu.device;
        // Everything is made into `r`, so whatever was made is destroyed by its Drop
        // if a later step fails.
        let mut r = Self {
            gpu: gpu.clone(),
            pipelines: [vk::Pipeline::null(); 2],
            layout: vk::PipelineLayout::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            sampler: vk::Sampler::null(),
            pool: vk::DescriptorPool::null(),
            sets: Vec::new(),
            images: Vec::new(),
            texture_sets: Vec::new(),
            buffers: None,
            draws: Vec::new(),
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
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                .immutable_samplers(&samplers),
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
        let set_layouts = [r.set_layout];
        r.layout = unsafe {
            d.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&ranges),
                None,
            )?
        };
        r.pipelines = [
            pipeline(gpu, r.layout, color_format, FRAG)?,
            pipeline(gpu, r.layout, color_format, CUTOUT)?,
        ];

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
        for guid in &scene.textures {
            match r.upload(load(guid)) {
                Ok(image) => {
                    r.texture_sets.push(r.images.len());
                    r.images.push(image);
                }
                Err((name, why)) => {
                    let name = name.unwrap_or_else(|| hex(guid));
                    r.missing.push((name, why));
                    r.texture_sets.push(0);
                }
            }
        }

        let count = r.images.len() as u32;
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: count,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: count,
            },
        ];
        r.pool = unsafe {
            d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(count)
                    .pool_sizes(&sizes),
                None,
            )?
        };
        let layouts = vec![r.set_layout; r.images.len()];
        r.sets = unsafe {
            d.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(r.pool)
                    .set_layouts(&layouts),
            )?
        };
        for (set, image) in r.sets.iter().zip(&r.images) {
            let info = [vk::DescriptorImageInfo::default()
                .image_view(image.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(*set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&info);
            unsafe { d.update_descriptor_sets(&[write], &[]) };
        }

        r.draws = scene.draws.clone();
        r.draws.sort_by_key(|d| (d.cutout, d.texture));
        Ok(r)
    }

    /// Uploads one loaded texture; on failure, its name if known and why.
    fn upload(
        &mut self,
        loaded: frostbite::Result<Loaded>,
    ) -> std::result::Result<Image, (Option<String>, String)> {
        let (name, texture, data) = loaded.map_err(|e| (None, e.0))?;
        let fail = |why: String| (Some(name.clone()), why);
        let Some(format) = vk_format(texture.format) else {
            return Err(fail(format!(
                "format {:#04x} not worked out",
                texture.format
            )));
        };
        let wanted = vk::FormatFeatureFlags::SAMPLED_IMAGE
            | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
        let props = unsafe {
            self.gpu
                .instance
                .get_physical_device_format_properties(self.gpu.physical, format)
        };
        if !props.optimal_tiling_features.contains(wanted) {
            return Err(fail(format!("{format:?} can't be sampled on this device")));
        }
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
            let Some([positions, instances, uvs, indices]) = &self.buffers else {
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
                &[positions.buffer, instances.buffer, uvs.buffer],
                &[0, 0, 0],
            );
            d.cmd_bind_index_buffer(cmd, indices.buffer, 0, vk::IndexType::UINT32);
            let (mut pipeline, mut set) = (None, None);
            for draw in &self.draws {
                if pipeline != Some(draw.cutout) {
                    let p = self.pipelines[usize::from(draw.cutout)];
                    d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, p);
                    pipeline = Some(draw.cutout);
                }
                let wanted = draw
                    .texture
                    .and_then(|t| self.texture_sets.get(t as usize).copied())
                    .unwrap_or(0);
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
            d.destroy_sampler(self.sampler, None);
        }
    }
}

/// Triangle lists: positions and texture coordinates per vertex, the transform per
/// instance; depth test and write (reversed depth); no culling, since transforms may
/// mirror. `fragment` is the fragment shader's SPIR-V and entry point.
fn pipeline(
    gpu: &Gpu,
    layout: vk::PipelineLayout,
    color_format: vk::Format,
    (fragment, entry): (&[u8], &CStr),
) -> Result<vk::Pipeline> {
    let d = &gpu.device;
    let vs = shader_module(gpu, VERT)?;
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
            .name(c"vs_main"),
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
    ];
    let attribute = |location, binding, format, offset| vk::VertexInputAttributeDescription {
        location,
        binding,
        format,
        offset,
    };
    let row = vk::Format::R32G32B32A32_SFLOAT;
    let attributes = [
        attribute(0, 0, vk::Format::R32G32B32_SFLOAT, 0),
        attribute(1, 1, row, 0),
        attribute(2, 1, row, 16),
        attribute(3, 1, row, 32),
        attribute(4, 2, vk::Format::R32G32_SFLOAT, 0),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL);
    let blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)];
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
