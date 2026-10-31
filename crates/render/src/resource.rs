//! GPU buffers and images, and the offscreen render target. Each owns its memory and
//! frees it when dropped.

use std::sync::Arc;

use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

use crate::gpu::{Error, Gpu, Result, lock};

/// Allocates and binds memory for a buffer or image; `None` while the device is
/// being dropped.
fn allocate(
    gpu: &Gpu,
    requirements: vk::MemoryRequirements,
    location: MemoryLocation,
    linear: bool,
) -> Result<Allocation> {
    let mut allocator = lock(&gpu.allocator);
    let Some(allocator) = allocator.as_mut() else {
        return Err(Error("the GPU is shutting down".into()));
    };
    Ok(allocator.allocate(&AllocationCreateDesc {
        name: if linear { "buffer" } else { "image" },
        requirements,
        location,
        linear,
        allocation_scheme: AllocationScheme::GpuAllocatorManaged,
    })?)
}

fn free(gpu: &Gpu, allocation: Option<Allocation>) {
    if let (Some(a), Some(allocator)) = (allocation, lock(&gpu.allocator).as_mut()) {
        let _ = allocator.free(a);
    }
}

pub struct Buffer {
    gpu: Arc<Gpu>,
    pub buffer: vk::Buffer,
    pub size: u64,
    allocation: Option<Allocation>,
}

impl Buffer {
    pub fn new(
        gpu: &Arc<Gpu>,
        usage: vk::BufferUsageFlags,
        size: u64,
        location: MemoryLocation,
    ) -> Result<Self> {
        let d = &gpu.device;
        let buffer = unsafe {
            d.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size.max(1))
                    .usage(usage),
                None,
            )?
        };
        let requirements = unsafe { d.get_buffer_memory_requirements(buffer) };
        let bound = allocate(gpu, requirements, location, true).and_then(|a| {
            match unsafe { d.bind_buffer_memory(buffer, a.memory(), a.offset()) } {
                Ok(()) => Ok(a),
                Err(e) => {
                    free(gpu, Some(a));
                    Err(e.into())
                }
            }
        });
        match bound {
            Ok(a) => Ok(Self {
                gpu: gpu.clone(),
                buffer,
                size,
                allocation: Some(a),
            }),
            Err(e) => {
                unsafe { d.destroy_buffer(buffer, None) };
                Err(e)
            }
        }
    }

    /// A buffer the GPU reads, filled with `data` through a staging buffer.
    pub fn upload(gpu: &Arc<Gpu>, usage: vk::BufferUsageFlags, data: &[u8]) -> Result<Self> {
        let size = data.len() as u64;
        let mut staging = Self::new(
            gpu,
            vk::BufferUsageFlags::TRANSFER_SRC,
            size,
            MemoryLocation::CpuToGpu,
        )?;
        staging.mapped_mut()?.copy_from_slice(data);
        let buffer = Self::new(
            gpu,
            usage | vk::BufferUsageFlags::TRANSFER_DST,
            size,
            MemoryLocation::GpuOnly,
        )?;
        if size > 0 {
            gpu.submit_and_wait(|cmd| unsafe {
                gpu.device.cmd_copy_buffer(
                    cmd,
                    staging.buffer,
                    buffer.buffer,
                    &[vk::BufferCopy::default().size(size)],
                );
            })?;
        }
        Ok(buffer)
    }

    pub fn mapped(&self) -> Result<&[u8]> {
        self.allocation
            .as_ref()
            .and_then(|a| a.mapped_slice())
            .map(|s| &s[..self.size as usize])
            .ok_or_else(|| Error("buffer is not CPU-visible".into()))
    }

    pub fn mapped_mut(&mut self) -> Result<&mut [u8]> {
        let size = self.size as usize;
        self.allocation
            .as_mut()
            .and_then(|a| a.mapped_slice_mut())
            .map(|s| &mut s[..size])
            .ok_or_else(|| Error("buffer is not CPU-visible".into()))
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe { self.gpu.device.destroy_buffer(self.buffer, None) };
        free(&self.gpu, self.allocation.take());
    }
}

pub struct Image {
    gpu: Arc<Gpu>,
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    allocation: Option<Allocation>,
}

/// One mip of a texture's pixels: where it starts in the bytes uploaded, and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mip {
    pub offset: u64,
    pub width: u32,
    pub height: u32,
}

impl Image {
    pub fn new(
        gpu: &Arc<Gpu>,
        extent: vk::Extent2D,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        aspect: vk::ImageAspectFlags,
    ) -> Result<Self> {
        Self::with_mips(gpu, extent, format, usage, aspect, 1)
    }

    fn with_mips(
        gpu: &Arc<Gpu>,
        extent: vk::Extent2D,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        aspect: vk::ImageAspectFlags,
        mips: u32,
    ) -> Result<Self> {
        let d = &gpu.device;
        let image = unsafe {
            d.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D {
                        width: extent.width.max(1),
                        height: extent.height.max(1),
                        depth: 1,
                    })
                    .mip_levels(mips)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage),
                None,
            )?
        };
        let requirements = unsafe { d.get_image_memory_requirements(image) };
        let allocation = match allocate(gpu, requirements, MemoryLocation::GpuOnly, false) {
            Ok(a) => a,
            Err(e) => {
                unsafe { d.destroy_image(image, None) };
                return Err(e);
            }
        };
        let made = unsafe {
            d.bind_image_memory(image, allocation.memory(), allocation.offset())
                .and_then(|()| {
                    d.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(format)
                            .subresource_range(vk::ImageSubresourceRange {
                                level_count: mips,
                                ..subresource(aspect)
                            }),
                        None,
                    )
                })
        };
        match made {
            Ok(view) => Ok(Self {
                gpu: gpu.clone(),
                image,
                view,
                format,
                extent,
                allocation: Some(allocation),
            }),
            Err(e) => {
                unsafe { d.destroy_image(image, None) };
                free(gpu, Some(allocation));
                Err(e.into())
            }
        }
    }

    /// A texture to sample, its `mips` (largest first) copied from `data` as they
    /// are, so a block-compressed format's blocks go up unchanged; left ready for
    /// fragment shaders to read.
    pub fn sampled(gpu: &Arc<Gpu>, format: vk::Format, mips: &[Mip], data: &[u8]) -> Result<Self> {
        let Some(first) = mips.first() else {
            return Err(Error("a texture with no mips".into()));
        };
        let extent = vk::Extent2D {
            width: first.width,
            height: first.height,
        };
        let usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
        let color = vk::ImageAspectFlags::COLOR;
        let image = Self::with_mips(gpu, extent, format, usage, color, mips.len() as u32)?;
        let mut staging = Buffer::new(
            gpu,
            vk::BufferUsageFlags::TRANSFER_SRC,
            data.len() as u64,
            MemoryLocation::CpuToGpu,
        )?;
        staging.mapped_mut()?.copy_from_slice(data);
        let regions: Vec<_> = (0u32..)
            .zip(mips)
            .map(|(level, m)| {
                vk::BufferImageCopy::default()
                    .buffer_offset(m.offset)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: color,
                        mip_level: level,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: m.width,
                        height: m.height,
                        depth: 1,
                    })
            })
            .collect();
        let all = vk::ImageSubresourceRange {
            level_count: mips.len() as u32,
            ..subresource(color)
        };
        gpu.submit_and_wait(|cmd| unsafe {
            use vk::AccessFlags2 as A;
            use vk::ImageLayout as L;
            use vk::PipelineStageFlags2 as S;
            let d = &gpu.device;
            let to_copy = [barrier(
                image.image,
                color,
                (L::UNDEFINED, S::NONE, A::NONE),
                (L::TRANSFER_DST_OPTIMAL, S::COPY, A::TRANSFER_WRITE),
            )
            .subresource_range(all)];
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_copy),
            );
            d.cmd_copy_buffer_to_image(
                cmd,
                staging.buffer,
                image.image,
                L::TRANSFER_DST_OPTIMAL,
                &regions,
            );
            let to_read = [barrier(
                image.image,
                color,
                (L::TRANSFER_DST_OPTIMAL, S::COPY, A::TRANSFER_WRITE),
                (
                    L::SHADER_READ_ONLY_OPTIMAL,
                    S::FRAGMENT_SHADER,
                    A::SHADER_SAMPLED_READ,
                ),
            )
            .subresource_range(all)];
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_read),
            );
        })?;
        Ok(image)
    }

    /// A depth buffer for drawing at `extent`.
    pub fn depth(gpu: &Arc<Gpu>, extent: vk::Extent2D) -> Result<Self> {
        Self::new(
            gpu,
            extent,
            DEPTH_FORMAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::ImageAspectFlags::DEPTH,
        )
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        unsafe {
            self.gpu.device.destroy_image_view(self.view, None);
            self.gpu.device.destroy_image(self.image, None);
        }
        free(&self.gpu, self.allocation.take());
    }
}

/// Every frame's depth format: 32-bit float, reversed (1 near, 0 far).
pub const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;

/// The whole of one mip level and layer.
pub fn subresource(aspect: vk::ImageAspectFlags) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: aspect,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// An image layout change, waiting for `src` and blocking `dst`.
pub fn barrier(
    image: vk::Image,
    aspect: vk::ImageAspectFlags,
    (old, src_stage, src_access): (vk::ImageLayout, vk::PipelineStageFlags2, vk::AccessFlags2),
    (new, dst_stage, dst_access): (vk::ImageLayout, vk::PipelineStageFlags2, vk::AccessFlags2),
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .image(image)
        .subresource_range(subresource(aspect))
        .old_layout(old)
        .new_layout(new)
        .src_stage_mask(src_stage)
        .src_access_mask(src_access)
        .dst_stage_mask(dst_stage)
        .dst_access_mask(dst_access)
}

/// An offscreen colour and depth target whose pixels can be read back (screenshots,
/// tests).
pub struct Target {
    gpu: Arc<Gpu>,
    pub color: Image,
    pub depth: Image,
    readback: Buffer,
}

impl Target {
    /// sRGB, like the window's swapchain, so a screenshot shows what the window does.
    pub const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;

    pub fn new(gpu: &Arc<Gpu>, width: u32, height: u32) -> Result<Self> {
        let extent = vk::Extent2D { width, height };
        Ok(Self {
            gpu: gpu.clone(),
            color: Image::new(
                gpu,
                extent,
                Self::COLOR_FORMAT,
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                vk::ImageAspectFlags::COLOR,
            )?,
            depth: Image::depth(gpu, extent)?,
            readback: Buffer::new(
                gpu,
                vk::BufferUsageFlags::TRANSFER_DST,
                u64::from(width) * u64::from(height) * 4,
                MemoryLocation::GpuToCpu,
            )?,
        })
    }

    /// Records `draw` into the target (colour and depth set up as attachments), waits
    /// for it, and returns the pixels as RGBA, row by row from the top.
    pub fn render(&mut self, draw: impl FnOnce(vk::CommandBuffer)) -> Result<Vec<u8>> {
        let (gpu, color, depth) = (&self.gpu, &self.color, &self.depth);
        let d = &gpu.device;
        let extent = color.extent;
        gpu.submit_and_wait(|cmd| unsafe {
            use vk::AccessFlags2 as A;
            use vk::ImageLayout as L;
            use vk::PipelineStageFlags2 as S;
            let to_draw = [
                barrier(
                    color.image,
                    vk::ImageAspectFlags::COLOR,
                    (L::UNDEFINED, S::NONE, A::NONE),
                    (
                        L::COLOR_ATTACHMENT_OPTIMAL,
                        S::COLOR_ATTACHMENT_OUTPUT,
                        A::COLOR_ATTACHMENT_WRITE,
                    ),
                ),
                barrier(
                    depth.image,
                    vk::ImageAspectFlags::DEPTH,
                    (L::UNDEFINED, S::NONE, A::NONE),
                    (
                        L::DEPTH_ATTACHMENT_OPTIMAL,
                        S::EARLY_FRAGMENT_TESTS | S::LATE_FRAGMENT_TESTS,
                        A::DEPTH_STENCIL_ATTACHMENT_WRITE | A::DEPTH_STENCIL_ATTACHMENT_READ,
                    ),
                ),
            ];
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_draw),
            );
            crate::frame::begin_rendering(d, cmd, color.view, depth.view, extent);
            draw(cmd);
            d.cmd_end_rendering(cmd);
            let to_copy = [barrier(
                color.image,
                vk::ImageAspectFlags::COLOR,
                (
                    L::COLOR_ATTACHMENT_OPTIMAL,
                    S::COLOR_ATTACHMENT_OUTPUT,
                    A::COLOR_ATTACHMENT_WRITE,
                ),
                (L::TRANSFER_SRC_OPTIMAL, S::COPY, A::TRANSFER_READ),
            )];
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_copy),
            );
            d.cmd_copy_image_to_buffer(
                cmd,
                color.image,
                L::TRANSFER_SRC_OPTIMAL,
                self.readback.buffer,
                &[vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    })],
            );
        })?;
        Ok(self.readback.mapped()?.to_vec())
    }
}
