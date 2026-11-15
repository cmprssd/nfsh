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
        Self::made(gpu, extent, format, usage, aspect, (1, 1), false)
    }

    /// An image of `mips` mips and `layers` layers, seen through an array view when
    /// `array`.
    fn made(
        gpu: &Arc<Gpu>,
        extent: vk::Extent2D,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        aspect: vk::ImageAspectFlags,
        (mips, layers): (u32, u32),
        array: bool,
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
                    .array_layers(layers)
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
                            .view_type(if array {
                                vk::ImageViewType::TYPE_2D_ARRAY
                            } else {
                                vk::ImageViewType::TYPE_2D
                            })
                            .format(format)
                            .subresource_range(vk::ImageSubresourceRange {
                                level_count: mips,
                                layer_count: layers,
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
        Self::sampled_layers(gpu, format, mips, (1, 0), data, false)
    }

    /// A texture array of `layers` textures alike, each `layer_bytes` after the last
    /// in `data`, with `mips` the first's; sampled through an array view even with one
    /// layer.
    pub fn sampled_array(
        gpu: &Arc<Gpu>,
        format: vk::Format,
        mips: &[Mip],
        (layers, layer_bytes): (u32, u64),
        data: &[u8],
    ) -> Result<Self> {
        Self::sampled_layers(gpu, format, mips, (layers, layer_bytes), data, true)
    }

    fn sampled_layers(
        gpu: &Arc<Gpu>,
        format: vk::Format,
        mips: &[Mip],
        (layers, layer_bytes): (u32, u64),
        data: &[u8],
        array: bool,
    ) -> Result<Self> {
        let image = Self::for_sampling(gpu, format, mips, layers, array)?;
        let mut staging = Buffer::new(
            gpu,
            vk::BufferUsageFlags::TRANSFER_SRC,
            data.len() as u64,
            MemoryLocation::CpuToGpu,
        )?;
        staging.mapped_mut()?.copy_from_slice(data);
        gpu.submit_and_wait(|cmd| {
            image.record_fill(cmd, staging.buffer, 0, mips, (layers, layer_bytes));
        })?;
        Ok(image)
    }

    /// An empty image for [`Image::record_fill`] to fill: `layers` layers of `mips`.
    fn for_sampling(
        gpu: &Arc<Gpu>,
        format: vk::Format,
        mips: &[Mip],
        layers: u32,
        array: bool,
    ) -> Result<Self> {
        let Some(first) = mips.first() else {
            return Err(Error("a texture with no mips".into()));
        };
        if layers == 0 {
            return Err(Error("a texture with no layers".into()));
        }
        let extent = vk::Extent2D {
            width: first.width,
            height: first.height,
        };
        let usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
        // The tests read the images back.
        #[cfg(test)]
        let usage = usage | vk::ImageUsageFlags::TRANSFER_SRC;
        let counts = (mips.len() as u32, layers);
        Self::made(
            gpu,
            extent,
            format,
            usage,
            vk::ImageAspectFlags::COLOR,
            counts,
            array,
        )
    }

    /// Records copying every layer's `mips` from `staging`, the bytes starting at
    /// `base`, into this image made by [`Image::for_sampling`], and leaving it ready
    /// for fragment shaders to read.
    fn record_fill(
        &self,
        cmd: vk::CommandBuffer,
        staging: vk::Buffer,
        base: u64,
        mips: &[Mip],
        (layers, layer_bytes): (u32, u64),
    ) {
        let color = vk::ImageAspectFlags::COLOR;
        let regions: Vec<_> = (0..layers)
            .flat_map(|layer| (0u32..).zip(mips).map(move |(level, m)| (layer, level, m)))
            .map(|(layer, level, m)| {
                vk::BufferImageCopy::default()
                    .buffer_offset(base + m.offset + u64::from(layer) * layer_bytes)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: color,
                        mip_level: level,
                        base_array_layer: layer,
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
            layer_count: layers,
            ..subresource(color)
        };
        use vk::AccessFlags2 as A;
        use vk::ImageLayout as L;
        use vk::PipelineStageFlags2 as S;
        let d = &self.gpu.device;
        let to_copy = [barrier(
            self.image,
            color,
            (L::UNDEFINED, S::NONE, A::NONE),
            (L::TRANSFER_DST_OPTIMAL, S::COPY, A::TRANSFER_WRITE),
        )
        .subresource_range(all)];
        let to_read = [barrier(
            self.image,
            color,
            (L::TRANSFER_DST_OPTIMAL, S::COPY, A::TRANSFER_WRITE),
            (
                L::SHADER_READ_ONLY_OPTIMAL,
                S::FRAGMENT_SHADER,
                A::SHADER_SAMPLED_READ,
            ),
        )
        .subresource_range(all)];
        unsafe {
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_copy),
            );
            d.cmd_copy_buffer_to_image(cmd, staging, self.image, L::TRANSFER_DST_OPTIMAL, &regions);
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_read),
            );
        }
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

/// Where each texture's bytes start in a staging half: a multiple of every block size
/// and of the copy alignment devices prefer.
const STAGING_ALIGN: u64 = 256;

/// Fills many sampled images through one reused staging buffer in two halves, each
/// with its own command buffer and fence: textures are copied into one half while the
/// GPU copies the other's into their images, several to a submit, instead of a staging
/// buffer and a wait for the GPU per texture. A texture larger than a half goes up on
/// its own. The images are only filled once [`Uploader::finish`] returns.
pub struct Uploader {
    gpu: Arc<Gpu>,
    pool: vk::CommandPool,
    halves: Vec<Half>,
    current: usize,
}

struct Half {
    staging: Buffer,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    /// Bytes of `staging` taken by the copies recorded since the last submit.
    used: u64,
    recording: bool,
    submitted: bool,
}

impl Uploader {
    /// An uploader whose two halves hold `half` bytes each.
    pub fn new(gpu: &Arc<Gpu>, half: u64) -> Result<Self> {
        let d = &gpu.device;
        let pool = unsafe {
            d.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .queue_family_index(gpu.queue_family),
                None,
            )?
        };
        // Made into `u` so that its Drop frees what was made if a later step fails.
        let mut u = Self {
            gpu: gpu.clone(),
            pool,
            halves: Vec::with_capacity(2),
            current: 0,
        };
        let cmds = unsafe {
            d.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .command_buffer_count(2),
            )?
        };
        for cmd in cmds {
            let staging = Buffer::new(
                gpu,
                vk::BufferUsageFlags::TRANSFER_SRC,
                half,
                MemoryLocation::CpuToGpu,
            )?;
            let fence = unsafe { d.create_fence(&vk::FenceCreateInfo::default(), None)? };
            u.halves.push(Half {
                staging,
                cmd,
                fence,
                used: 0,
                recording: false,
                submitted: false,
            });
        }
        Ok(u)
    }

    /// [`Image::sampled`], filled once [`Uploader::finish`] returns.
    pub fn sampled(&mut self, format: vk::Format, mips: &[Mip], data: &[u8]) -> Result<Image> {
        self.sampled_layers(format, mips, (1, 0), data, false)
    }

    /// [`Image::sampled_array`], filled once [`Uploader::finish`] returns.
    pub fn sampled_array(
        &mut self,
        format: vk::Format,
        mips: &[Mip],
        layers: (u32, u64),
        data: &[u8],
    ) -> Result<Image> {
        self.sampled_layers(format, mips, layers, data, true)
    }

    fn sampled_layers(
        &mut self,
        format: vk::Format,
        mips: &[Mip],
        layers: (u32, u64),
        data: &[u8],
        array: bool,
    ) -> Result<Image> {
        let size = data.len() as u64;
        if size > self.halves[self.current].staging.size {
            return Image::sampled_layers(&self.gpu, format, mips, layers, data, array);
        }
        let image = Image::for_sampling(&self.gpu, format, mips, layers.0, array)?;
        let mut base = self.halves[self.current]
            .used
            .next_multiple_of(STAGING_ALIGN);
        if base + size > self.halves[self.current].staging.size {
            self.submit()?;
            base = 0;
        }
        let half = self.begin()?;
        let at = base as usize;
        half.staging.mapped_mut()?[at..at + data.len()].copy_from_slice(data);
        half.used = base + size;
        image.record_fill(half.cmd, half.staging.buffer, base, mips, layers);
        Ok(image)
    }

    /// The current half, recording: once the GPU is done with its last submit.
    fn begin(&mut self) -> Result<&mut Half> {
        let d = &self.gpu.device;
        let half = &mut self.halves[self.current];
        if !half.recording {
            if half.submitted {
                unsafe {
                    d.wait_for_fences(&[half.fence], true, u64::MAX)?;
                    d.reset_fences(&[half.fence])?;
                }
                half.submitted = false;
            }
            unsafe {
                d.begin_command_buffer(
                    half.cmd,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )?;
            }
            half.recording = true;
            half.used = 0;
        }
        Ok(half)
    }

    /// Submits the current half's copies, if any, and moves to the other half.
    fn submit(&mut self) -> Result<()> {
        let d = &self.gpu.device;
        let half = &mut self.halves[self.current];
        if half.recording {
            half.recording = false;
            half.used = 0;
            let cmds = [half.cmd];
            unsafe {
                d.end_command_buffer(half.cmd)?;
                d.queue_submit(
                    *lock(&self.gpu.queue),
                    &[vk::SubmitInfo::default().command_buffers(&cmds)],
                    half.fence,
                )?;
            }
            half.submitted = true;
        }
        self.current = 1 - self.current;
        Ok(())
    }

    /// Submits what is left and waits until every image is filled.
    pub fn finish(mut self) -> Result<()> {
        self.submit()?;
        let fences: Vec<_> = self
            .halves
            .iter_mut()
            .filter(|h| h.submitted)
            .map(|h| {
                h.submitted = false;
                h.fence
            })
            .collect();
        if !fences.is_empty() {
            unsafe { self.gpu.device.wait_for_fences(&fences, true, u64::MAX)? };
        }
        Ok(())
    }
}

impl Drop for Uploader {
    fn drop(&mut self) {
        let d = &self.gpu.device;
        unsafe {
            // A half still copying must finish before its buffer and commands go.
            for half in &self.halves {
                if half.submitted {
                    let _ = d.wait_for_fences(&[half.fence], true, u64::MAX);
                }
                d.destroy_fence(half.fence, None);
            }
            // Frees the command buffers, recording or not.
            d.destroy_command_pool(self.pool, None);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `image`'s `layers` layers of `mips`, read back into bytes laid out as uploaded.
    fn read_back(gpu: &Arc<Gpu>, image: &Image, mips: &[Mip], layers: u32, size: u64) -> Vec<u8> {
        let layer_bytes = size / u64::from(layers);
        let readback = Buffer::new(
            gpu,
            vk::BufferUsageFlags::TRANSFER_DST,
            size,
            MemoryLocation::GpuToCpu,
        )
        .unwrap();
        let color = vk::ImageAspectFlags::COLOR;
        let regions: Vec<_> = (0..layers)
            .flat_map(|layer| (0u32..).zip(mips).map(move |(level, m)| (layer, level, m)))
            .map(|(layer, level, m)| {
                vk::BufferImageCopy::default()
                    .buffer_offset(m.offset + u64::from(layer) * layer_bytes)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: color,
                        mip_level: level,
                        base_array_layer: layer,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: m.width,
                        height: m.height,
                        depth: 1,
                    })
            })
            .collect();
        gpu.submit_and_wait(|cmd| unsafe {
            use vk::AccessFlags2 as A;
            use vk::ImageLayout as L;
            use vk::PipelineStageFlags2 as S;
            let to_copy = [barrier(
                image.image,
                color,
                (L::SHADER_READ_ONLY_OPTIMAL, S::NONE, A::NONE),
                (L::TRANSFER_SRC_OPTIMAL, S::COPY, A::TRANSFER_READ),
            )
            .subresource_range(vk::ImageSubresourceRange {
                level_count: mips.len() as u32,
                layer_count: layers,
                ..subresource(color)
            })];
            gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_copy),
            );
            gpu.device.cmd_copy_image_to_buffer(
                cmd,
                image.image,
                L::TRANSFER_SRC_OPTIMAL,
                readback.buffer,
                &regions,
            );
        })
        .unwrap();
        readback.mapped().unwrap().to_vec()
    }

    /// An RGBA texture's mips from `width`×`width` down to 1×1, and its size in bytes.
    fn rgba_mips(width: u32) -> (Vec<Mip>, u64) {
        let mut mips = Vec::new();
        let (mut w, mut offset) = (width, 0);
        while w > 0 {
            mips.push(Mip {
                offset,
                width: w,
                height: w,
            });
            offset += u64::from(w * w * 4);
            w /= 2;
        }
        (mips, offset)
    }

    #[test]
    fn the_uploader_fills_every_image_with_its_bytes() {
        let gpu = Gpu::new_headless().unwrap();
        let format = vk::Format::R8G8B8A8_UNORM;
        // Halves of 1 KiB: 8x8 textures (340 bytes with their mips) go three to a
        // half, so nine of them use both halves and wait to reuse the first; a 32x32
        // one is larger than a half and goes up on its own; a two-layer array fits.
        let mut uploader = Uploader::new(&gpu, 1024).unwrap();
        let mut wanted = Vec::new();
        let textures = std::iter::repeat_n(8, 9).chain([32, 8]);
        for (i, width) in textures.enumerate() {
            let (mips, size) = rgba_mips(width);
            let layers = if i == 10 { 2 } else { 1 };
            let data: Vec<u8> = (0..size * u64::from(layers))
                .map(|b| (b as u8).wrapping_mul(31).wrapping_add(i as u8 * 17))
                .collect();
            let image = if layers == 1 {
                uploader.sampled(format, &mips, &data).unwrap()
            } else {
                uploader
                    .sampled_array(format, &mips, (layers, size), &data)
                    .unwrap()
            };
            wanted.push((image, mips, layers, data));
        }
        uploader.finish().unwrap();
        for (i, (image, mips, layers, data)) in wanted.iter().enumerate() {
            let got = read_back(&gpu, image, mips, *layers, data.len() as u64);
            assert!(got == *data, "texture {i} came back different");
        }
    }

    #[test]
    fn an_uploader_dropped_unfinished_frees_its_copies() {
        let gpu = Gpu::new_headless().unwrap();
        let mut uploader = Uploader::new(&gpu, 1024).unwrap();
        let (mips, size) = rgba_mips(8);
        let data = vec![7; size as usize];
        let images: Vec<_> = (0..5)
            .map(|_| {
                uploader
                    .sampled(vk::Format::R8G8B8A8_UNORM, &mips, &data)
                    .unwrap()
            })
            .collect();
        // One half submitted, the other recording: dropping waits, then frees both.
        drop(uploader);
        drop(images);
    }
}
