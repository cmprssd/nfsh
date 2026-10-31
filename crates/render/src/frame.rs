//! Presenting to a window: up to [`FRAMES_IN_FLIGHT`] frames recorded ahead, each
//! drawn straight into a swapchain image with a shared depth buffer, and timed on the
//! GPU.

use std::sync::Arc;

use ash::vk;

use crate::gpu::{Gpu, Result, lock};
use crate::resource::{Image, barrier};
use crate::swapchain::Swapchain;

pub const FRAMES_IN_FLIGHT: usize = 2;

/// Begins rendering into `color` (cleared to black) and `depth` (cleared to 0, the
/// far end of reversed depth), both already in attachment layouts.
pub fn begin_rendering(
    d: &ash::Device,
    cmd: vk::CommandBuffer,
    color: vk::ImageView,
    depth: vk::ImageView,
    extent: vk::Extent2D,
) {
    let colors = [vk::RenderingAttachmentInfo::default()
        .image_view(color)
        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .clear_value(vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        })];
    let depth = vk::RenderingAttachmentInfo::default()
        .image_view(depth)
        .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::DONT_CARE)
        .clear_value(vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 0.0,
                stencil: 0,
            },
        });
    unsafe {
        d.cmd_begin_rendering(
            cmd,
            &vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                })
                .layer_count(1)
                .color_attachments(&colors)
                .depth_attachment(&depth),
        );
    }
}

/// What presenting a frame came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Presented {
    /// Drawn and queued; with how long the GPU took on the last frame drawn in this
    /// slot, in ms, when the device has timestamps.
    Drawn { gpu_ms: Option<f32> },
    /// The swapchain no longer fits the window: resize it (the frame may or may not
    /// have been shown).
    Resize,
}

#[derive(Clone, Copy)]
struct Slot {
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    acquired: vk::Semaphore,
    /// Whether this slot's timestamps hold a finished frame.
    timed: bool,
}

pub struct FrameLoop {
    gpu: Arc<Gpu>,
    pool: vk::CommandPool,
    slots: Vec<Slot>,
    next: usize,
    /// Two timestamps per slot; `None` when the queue has no timestamps.
    queries: Option<vk::QueryPool>,
    depth: Option<Image>,
}

impl FrameLoop {
    pub fn new(gpu: &Arc<Gpu>) -> Result<Self> {
        let d = &gpu.device;
        let pool = unsafe {
            d.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(gpu.queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?
        };
        // Built as it goes, so a failure part way drops what was made.
        let mut frames = Self {
            gpu: gpu.clone(),
            pool,
            slots: Vec::new(),
            next: 0,
            queries: None,
            depth: None,
        };
        let cmds = unsafe {
            d.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .command_buffer_count(FRAMES_IN_FLIGHT as u32),
            )?
        };
        for cmd in cmds {
            let fence = unsafe {
                d.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )?
            };
            let acquired = unsafe { d.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) };
            let acquired = match acquired {
                Ok(s) => s,
                Err(e) => {
                    unsafe { d.destroy_fence(fence, None) };
                    return Err(e.into());
                }
            };
            frames.slots.push(Slot {
                cmd,
                fence,
                acquired,
                timed: false,
            });
        }
        let families = unsafe {
            gpu.instance
                .get_physical_device_queue_family_properties(gpu.physical)
        };
        let has_timestamps = families
            .get(gpu.queue_family as usize)
            .is_some_and(|f| f.timestamp_valid_bits > 0);
        if has_timestamps {
            frames.queries = Some(unsafe {
                d.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(2 * FRAMES_IN_FLIGHT as u32),
                    None,
                )?
            });
        }
        Ok(frames)
    }

    /// Draws one frame with `record` (inside rendering begun at the swapchain's size)
    /// and presents it.
    pub fn present(
        &mut self,
        swapchain: &Swapchain,
        record: impl FnOnce(vk::CommandBuffer, vk::Extent2D),
    ) -> Result<Presented> {
        let gpu = self.gpu.clone();
        let d = &gpu.device;
        let index = self.next;
        let slot = self.slots[index];
        unsafe { d.wait_for_fences(&[slot.fence], true, u64::MAX)? };
        let gpu_ms = match (self.queries, slot.timed) {
            (Some(pool), true) => {
                let mut t = [0u64; 2];
                unsafe {
                    d.get_query_pool_results(
                        pool,
                        2 * index as u32,
                        &mut t,
                        vk::QueryResultFlags::TYPE_64,
                    )
                }
                .ok()
                .map(|()| t[1].wrapping_sub(t[0]) as f32 * gpu.timestamp_period / 1e6)
            }
            _ => None,
        };
        let acquired = unsafe {
            swapchain.swapchain_fn.acquire_next_image(
                swapchain.swapchain,
                u64::MAX,
                slot.acquired,
                vk::Fence::null(),
            )
        };
        let (image, suboptimal) = match acquired {
            Ok(a) => a,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(Presented::Resize),
            Err(e) => return Err(e.into()),
        };
        let extent = swapchain.extent;
        if self.depth.as_ref().is_none_or(|i| i.extent != extent) {
            self.depth = None;
            self.depth = Some(Image::depth(&gpu, extent)?);
        }
        let Some(depth) = &self.depth else {
            return Ok(Presented::Resize);
        };
        let i = image as usize;
        let (color_image, color_view, rendered) = (
            swapchain.images[i],
            swapchain.views[i],
            swapchain.rendered[i],
        );

        let cmd = slot.cmd;
        unsafe {
            d.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
            d.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            if let Some(pool) = self.queries {
                d.cmd_reset_query_pool(cmd, pool, 2 * index as u32, 2);
                d.cmd_write_timestamp2(
                    cmd,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    pool,
                    2 * index as u32,
                );
            }
            use vk::AccessFlags2 as A;
            use vk::ImageLayout as L;
            use vk::PipelineStageFlags2 as S;
            let depth_stages = S::EARLY_FRAGMENT_TESTS | S::LATE_FRAGMENT_TESTS;
            let depth_access = A::DEPTH_STENCIL_ATTACHMENT_WRITE | A::DEPTH_STENCIL_ATTACHMENT_READ;
            let to_draw = [
                // Waits for the acquire semaphore, which is waited at this stage.
                barrier(
                    color_image,
                    vk::ImageAspectFlags::COLOR,
                    (L::UNDEFINED, S::COLOR_ATTACHMENT_OUTPUT, A::NONE),
                    (
                        L::COLOR_ATTACHMENT_OPTIMAL,
                        S::COLOR_ATTACHMENT_OUTPUT,
                        A::COLOR_ATTACHMENT_WRITE,
                    ),
                ),
                // The depth buffer is shared: wait for the previous frame's use.
                barrier(
                    depth.image,
                    vk::ImageAspectFlags::DEPTH,
                    (L::UNDEFINED, depth_stages, depth_access),
                    (L::DEPTH_ATTACHMENT_OPTIMAL, depth_stages, depth_access),
                ),
            ];
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_draw),
            );
            begin_rendering(d, cmd, color_view, depth.view, extent);
            record(cmd, extent);
            d.cmd_end_rendering(cmd);
            let to_present = [barrier(
                color_image,
                vk::ImageAspectFlags::COLOR,
                (
                    L::COLOR_ATTACHMENT_OPTIMAL,
                    S::COLOR_ATTACHMENT_OUTPUT,
                    A::COLOR_ATTACHMENT_WRITE,
                ),
                (L::PRESENT_SRC_KHR, S::NONE, A::NONE),
            )];
            d.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_present),
            );
            if let Some(pool) = self.queries {
                d.cmd_write_timestamp2(
                    cmd,
                    vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
                    pool,
                    2 * index as u32 + 1,
                );
            }
            d.end_command_buffer(cmd)?;
        }

        let waits = [vk::SemaphoreSubmitInfo::default()
            .semaphore(slot.acquired)
            .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
        let signals = [vk::SemaphoreSubmitInfo::default()
            .semaphore(rendered)
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
        let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
        let swapchains = [swapchain.swapchain];
        let images = [image];
        let wait_rendered = [rendered];
        let queue = lock(&gpu.queue);
        let presented = unsafe {
            d.reset_fences(&[slot.fence])?;
            d.queue_submit2(
                *queue,
                &[vk::SubmitInfo2::default()
                    .wait_semaphore_infos(&waits)
                    .command_buffer_infos(&cmds)
                    .signal_semaphore_infos(&signals)],
                slot.fence,
            )?;
            swapchain.swapchain_fn.queue_present(
                *queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&wait_rendered)
                    .swapchains(&swapchains)
                    .image_indices(&images),
            )
        };
        drop(queue);
        self.slots[index].timed = true;
        self.next = (index + 1) % FRAMES_IN_FLIGHT;
        match presented {
            Ok(false) if !suboptimal => Ok(Presented::Drawn { gpu_ms }),
            Ok(_) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(Presented::Resize),
            Err(e) => Err(e.into()),
        }
    }

    /// Waits until no frame is in flight (before resizing or closing).
    pub fn finish(&self) {
        self.gpu.wait_idle();
    }
}

impl Drop for FrameLoop {
    fn drop(&mut self) {
        let d = &self.gpu.device;
        self.gpu.wait_idle();
        unsafe {
            for s in &self.slots {
                d.destroy_fence(s.fence, None);
                d.destroy_semaphore(s.acquired, None);
            }
            if let Some(pool) = self.queries {
                d.destroy_query_pool(pool, None);
            }
            d.destroy_command_pool(self.pool, None);
        }
    }
}
