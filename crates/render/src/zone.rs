//! Draws a [`Scene`] on the GPU: its buffers uploaded once, one instanced draw per
//! mesh.

use std::sync::Arc;

use ash::vk;

use crate::Camera;
use crate::gpu::{Error, Gpu, Result};
use crate::resource::{Buffer, DEPTH_FORMAT};
use crate::scene::{Draw, Instance, Scene};

const VERT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/zone.vert.spv"));
const FRAG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/zone.frag.spv"));

/// The shader's push constants: the view-projection matrix and the eye.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Push {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
}

pub struct ZoneRenderer {
    gpu: Arc<Gpu>,
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    /// `None` when the scene draws nothing (Vulkan has no empty buffers to bind).
    buffers: Option<[Buffer; 3]>,
    draws: Vec<Draw>,
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
    /// Uploads `scene` and makes the pipeline for drawing into `color_format`.
    pub fn new(gpu: &Arc<Gpu>, scene: &Scene, color_format: vk::Format) -> Result<Self> {
        let buffers = match scene.draws.is_empty() {
            true => None,
            false => Some([
                Buffer::upload(
                    gpu,
                    vk::BufferUsageFlags::VERTEX_BUFFER,
                    bytemuck::cast_slice(&scene.positions),
                )?,
                Buffer::upload(
                    gpu,
                    vk::BufferUsageFlags::VERTEX_BUFFER,
                    bytemuck::cast_slice(&scene.instances),
                )?,
                Buffer::upload(
                    gpu,
                    vk::BufferUsageFlags::INDEX_BUFFER,
                    bytemuck::cast_slice(&scene.indices),
                )?,
            ]),
        };
        let d = &gpu.device;
        let ranges = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            offset: 0,
            size: size_of::<Push>() as u32,
        }];
        let layout = unsafe {
            d.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&ranges),
                None,
            )?
        };
        let pipeline = match pipeline(gpu, layout, color_format) {
            Ok(p) => p,
            Err(e) => {
                unsafe { d.destroy_pipeline_layout(layout, None) };
                return Err(e);
            }
        };
        Ok(Self {
            gpu: gpu.clone(),
            pipeline,
            layout,
            buffers,
            draws: scene.draws.clone(),
        })
    }

    /// Records the zone's draws, inside rendering begun at `extent`.
    pub fn record(&self, cmd: vk::CommandBuffer, camera: &Camera, extent: vk::Extent2D) {
        let d = &self.gpu.device;
        let push = Push {
            view_proj: camera.view_proj(),
            eye: [camera.origin[0], camera.origin[1], camera.origin[2], 1.0],
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
            let Some([positions, instances, indices]) = &self.buffers else {
                return;
            };
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            d.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&push),
            );
            d.cmd_bind_vertex_buffers(cmd, 0, &[positions.buffer, instances.buffer], &[0, 0]);
            d.cmd_bind_index_buffer(cmd, indices.buffer, 0, vk::IndexType::UINT32);
            for draw in &self.draws {
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

impl Drop for ZoneRenderer {
    fn drop(&mut self) {
        unsafe {
            self.gpu.device.destroy_pipeline(self.pipeline, None);
            self.gpu.device.destroy_pipeline_layout(self.layout, None);
        }
    }
}

/// Triangle lists, positions per vertex and the transform per instance; depth test
/// and write (reversed depth); no culling, since transforms may mirror.
fn pipeline(
    gpu: &Gpu,
    layout: vk::PipelineLayout,
    color_format: vk::Format,
) -> Result<vk::Pipeline> {
    let d = &gpu.device;
    let vs = shader_module(gpu, VERT)?;
    let fs = match shader_module(gpu, FRAG) {
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
            .name(c"fs_main"),
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
