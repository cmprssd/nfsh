//! The sky behind everything: a time of day's panorama, gradient and mask (a visual
//! environment's [`frostbite::lighting::Sky`]) drawn as one triangle over the screen
//! at infinite depth, after the zone, where nothing else was drawn. One dome per time
//! of day, all uploaded once; each frame draws the one asked for.

use std::ffi::CStr;
use std::sync::Arc;

use ash::vk;
use frostbite::lighting::{Lighting, Sky as SkyData};

use crate::Camera;
use crate::gpu::{Gpu, Result};
use crate::resource::{DEPTH_FORMAT, Image, Mip};
use crate::zone::{Loaded, mips, sampled_format, shader_module};

const VERT: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/sky.vs_sky.spv")),
    c"vs_sky",
);
const FRAG: (&[u8], &CStr) = (
    include_bytes!(concat!(env!("OUT_DIR"), "/sky.fs_sky.spv")),
    c"fs_sky",
);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Push {
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    sun: [f32; 4],
    disc: [f32; 4],
    uv: [f32; 4],
    scale: [f32; 4],
}

/// How a dome is drawn, already exposed: what the shader takes besides the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// The textures' values to exposed light: `LuminanceScale` × the exposure.
    pub scale: f32,
    /// The bearing of the panorama's left edge, in turns.
    pub rotation: f32,
    /// The part of the panorama over the dome: left and zenith, right and horizon.
    pub uv: [f32; 4],
    /// Towards the sun.
    pub sun: [f32; 3],
    /// The sun's disc: its angular radius in radians, and its light, exposed (zero
    /// for none).
    pub disc_radius: f32,
    pub disc: [f32; 3],
}

impl Look {
    /// `sky` lit and exposed as `lighting` says: its textures' values times
    /// `LuminanceScale` are luminance (cd/m²), and the disc's is `SunScale` times the
    /// sun's colour.
    pub fn new(sky: &SkyData, lighting: &Lighting) -> Self {
        let exposure = lighting.exposure();
        let disc = if sky.sun_disc && sky.sun_scale > 0.0 {
            lighting.sun_colour.map(|c| c * sky.sun_scale * exposure)
        } else {
            [0.0; 3]
        };
        Self {
            scale: sky.luminance_scale * exposure,
            rotation: sky.rotation,
            uv: sky.uv,
            sun: lighting.sun_direction(),
            disc_radius: sky.sun_size.max(0.0),
            disc,
        }
    }
}

/// A dome's textures as loaded; a mask that is missing shows the panorama
/// everywhere, a missing panorama or gradient is black.
pub struct Textures {
    pub panorama: Option<Loaded>,
    pub gradient: Option<Loaded>,
    pub mask: Option<Loaded>,
}

struct Dome {
    look: Look,
    set: vk::DescriptorSet,
}

pub struct Sky {
    gpu: Arc<Gpu>,
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    pool: vk::DescriptorPool,
    images: Vec<Image>,
    domes: Vec<Option<Dome>>,
    /// Textures that couldn't be used, with why: each is drawn black (white for a
    /// mask).
    pub missing: Vec<(String, String)>,
}

impl Sky {
    /// Uploads each of `domes` (none for a time of day without a sky) and makes the
    /// pipeline for drawing into `color_format`.
    pub fn new(
        gpu: &Arc<Gpu>,
        color_format: vk::Format,
        domes: Vec<Option<(Look, Textures)>>,
    ) -> Result<Self> {
        let d = &gpu.device;
        // Made into `s`, so that its Drop frees what was made if a later step fails.
        let mut s = Self {
            gpu: gpu.clone(),
            pipeline: vk::Pipeline::null(),
            layout: vk::PipelineLayout::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            sampler: vk::Sampler::null(),
            pool: vk::DescriptorPool::null(),
            images: Vec::new(),
            domes: Vec::new(),
            missing: Vec::new(),
        };
        s.sampler = unsafe {
            d.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::REPEAT)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .max_lod(vk::LOD_CLAMP_NONE),
                None,
            )?
        };
        let samplers = [s.sampler];
        let image = |binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        };
        let bindings = [
            image(0),
            image(1),
            image(2),
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                .immutable_samplers(&samplers),
        ];
        s.set_layout = unsafe {
            d.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?
        };
        let ranges = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::FRAGMENT,
            offset: 0,
            size: size_of::<Push>() as u32,
        }];
        let set_layouts = [s.set_layout];
        s.layout = unsafe {
            d.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&ranges),
                None,
            )?
        };
        s.pipeline = pipeline(gpu, s.layout, color_format)?;

        let count = domes.iter().flatten().count().max(1) as u32;
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: count * 3,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: count,
            },
        ];
        s.pool = unsafe {
            d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(count)
                    .pool_sizes(&sizes),
                None,
            )?
        };
        let black = s.image_at(Image::sampled(gpu, ONE_TEXEL, &ONE, &[0, 0, 0, 255])?);
        let white = s.image_at(Image::sampled(gpu, ONE_TEXEL, &ONE, &[255; 4])?);
        for dome in domes {
            let Some((look, textures)) = dome else {
                s.domes.push(None);
                continue;
            };
            let images = [
                (textures.panorama, black),
                (textures.gradient, black),
                (textures.mask, white),
            ]
            .map(|(loaded, fallback)| s.upload(loaded).unwrap_or(fallback));
            let layouts = [s.set_layout];
            let set = unsafe {
                d.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(s.pool)
                        .set_layouts(&layouts),
                )?[0]
            };
            for (binding, image) in images.into_iter().enumerate() {
                let info = [vk::DescriptorImageInfo::default()
                    .image_view(s.images[image].view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
                let write = vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(binding as u32)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&info);
                unsafe { d.update_descriptor_sets(&[write], &[]) };
            }
            s.domes.push(Some(Dome { look, set }));
        }
        Ok(s)
    }

    fn image_at(&mut self, image: Image) -> usize {
        self.images.push(image);
        self.images.len() - 1
    }

    /// `loaded` as an image, by its index; none, listed in `missing`, if it can't be
    /// sampled or uploaded.
    fn upload(&mut self, loaded: Option<Loaded>) -> Option<usize> {
        let (name, texture, data) = loaded?;
        let made = sampled_format(&self.gpu, &texture).and_then(|format| {
            let mips = mips(&texture, data.len())?;
            Image::sampled(&self.gpu, format, &mips, &data).map_err(|e| e.0)
        });
        match made {
            Ok(image) => Some(self.image_at(image)),
            Err(why) => {
                self.missing.push((name, why));
                None
            }
        }
    }

    /// Draws the dome `dome` (a time of day, as given to [`Sky::new`]) behind what was
    /// drawn, inside rendering begun at `extent` with the zone's viewport; nothing
    /// for a time without a sky.
    pub fn record(&self, cmd: vk::CommandBuffer, camera: &Camera, dome: usize) {
        let Some(Some(dome)) = self.domes.get(dome) else {
            return;
        };
        let look = &dome.look;
        let [f, r, u] = camera.axes();
        let [tx, ty] = camera.tangents();
        // A disc smaller than a pixel would flicker in and out between pixels: it is
        // drawn at least as wide as one.
        let pixel = 2.0 * ty / camera.height.max(1) as f32;
        let radius = look.disc_radius.max(pixel * 0.75);
        let [sx, sy, sz] = look.sun;
        let [dr, dg, db] = look.disc;
        let [u0, v0, u1, v1] = look.uv;
        let push = Push {
            forward: [f[0], f[1], f[2], 0.0],
            right: [r[0] * tx, r[1] * tx, r[2] * tx, 0.0],
            up: [u[0] * ty, u[1] * ty, u[2] * ty, 0.0],
            sun: [sx, sy, sz, radius.cos()],
            disc: [dr, dg, db, 0.0],
            uv: [u0, v0, u1, v1],
            scale: [look.scale, look.rotation, 0.0, 0.0],
        };
        let d = &self.gpu.device;
        unsafe {
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[dome.set],
                &[],
            );
            d.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&push),
            );
            d.cmd_draw(cmd, 3, 1, 0, 0);
        }
    }
}

const ONE_TEXEL: vk::Format = vk::Format::R8G8B8A8_UNORM;
const ONE: [Mip; 1] = [Mip {
    offset: 0,
    width: 1,
    height: 1,
}];

impl Drop for Sky {
    fn drop(&mut self) {
        // Destroying a null handle does nothing, so a half-made sky is fine.
        let d = &self.gpu.device;
        unsafe {
            d.destroy_descriptor_pool(self.pool, None);
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_sampler(self.sampler, None);
        }
    }
}

/// One triangle over the screen, no vertex input; drawn where the depth is still
/// cleared (0, infinitely far, with reversed depth), writing none.
fn pipeline(
    gpu: &Gpu,
    layout: vk::PipelineLayout,
    color_format: vk::Format,
) -> Result<vk::Pipeline> {
    let d = &gpu.device;
    let vs = shader_module(gpu, VERT.0)?;
    let fs = match shader_module(gpu, FRAG.0) {
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
            .name(VERT.1),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs)
            .name(FRAG.1),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
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
        .depth_write_enable(false)
        .depth_compare_op(vk::CompareOp::EQUAL);
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
    Ok(made.map_err(|(_, e)| crate::gpu::Error::from(e))?[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKY: SkyData = SkyData {
        panorama: None,
        gradient: None,
        mask: None,
        luminance_scale: 983.0,
        rotation: 0.44,
        uv: [0.0, -0.02, 1.0, 1.0],
        sun_disc: true,
        sun_size: 0.001,
        sun_scale: 1e7,
    };

    #[test]
    fn the_sky_is_luminance_exposed_as_the_sun_is() {
        let l = Lighting {
            sun_colour: [1.0, 0.5, 0.25],
            ..Lighting::FALLBACK
        };
        let look = Look::new(&SKY, &l);
        let exposure = l.exposure();
        assert!((look.scale - 983.0 * exposure).abs() < 1e-9);
        assert_eq!(
            (look.rotation, look.uv, look.disc_radius),
            (0.44, SKY.uv, 0.001)
        );
        assert_eq!(look.sun, l.sun_direction());
        for (got, c) in look.disc.iter().zip([1.0, 0.5, 0.25]) {
            assert!((got - c * 1e7 * exposure).abs() < 1e-3);
        }
        // No disc when it is turned off or has no light.
        let off = Look::new(
            &SkyData {
                sun_disc: false,
                ..SKY
            },
            &l,
        );
        let dark = Look::new(
            &SkyData {
                sun_scale: 0.0,
                ..SKY
            },
            &l,
        );
        assert_eq!((off.disc, dark.disc), ([0.0; 3], [0.0; 3]));
    }
}
