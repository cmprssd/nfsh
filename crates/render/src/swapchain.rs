//! A window's swapchain; [`crate::FrameLoop`] draws straight into its images.

use std::sync::Arc;

use ash::vk;

use crate::resource::subresource;
use crate::{Error, Gpu, Result};

pub struct Swapchain {
    gpu: Arc<Gpu>,
    surface_fn: ash::khr::surface::Instance,
    pub(crate) swapchain_fn: ash::khr::swapchain::Device,
    surface: vk::SurfaceKHR,
    pub(crate) swapchain: vk::SwapchainKHR,
    pub(crate) images: Vec<vk::Image>,
    pub(crate) views: Vec<vk::ImageView>,
    /// Per image: signalled when its frame is rendered; presenting waits for it.
    pub(crate) rendered: Vec<vk::Semaphore>,
    pub extent: vk::Extent2D,
    pub format: vk::Format,
    pub present_mode: vk::PresentModeKHR,
    vsync: bool,
}

/// The image format: sRGB, like [`crate::Target`], so the shaders work in linear
/// colour; else 8-bit UNORM (a little dark, but drawn). `None` if the surface offers
/// neither.
pub fn choose_format(available: &[vk::SurfaceFormatKHR]) -> Option<vk::SurfaceFormatKHR> {
    [
        vk::Format::B8G8R8A8_SRGB,
        vk::Format::R8G8B8A8_SRGB,
        vk::Format::B8G8R8A8_UNORM,
        vk::Format::R8G8B8A8_UNORM,
    ]
    .into_iter()
    .find_map(|f| {
        available
            .iter()
            .copied()
            .find(|a| a.format == f && a.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR)
    })
}

/// With `vsync`, FIFO (always available). Without, the fastest the surface has:
/// IMMEDIATE, else MAILBOX, else FIFO.
pub fn choose_present_mode(available: &[vk::PresentModeKHR], vsync: bool) -> vk::PresentModeKHR {
    let order: &[vk::PresentModeKHR] = if vsync {
        &[]
    } else {
        &[vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::MAILBOX]
    };
    order
        .iter()
        .copied()
        .find(|m| available.contains(m))
        .unwrap_or(vk::PresentModeKHR::FIFO)
}

impl Swapchain {
    /// Takes ownership of `surface` (from [`Gpu::new_for_window`]). `size` is the
    /// window's inner size in pixels.
    pub fn new(
        gpu: &Arc<Gpu>,
        surface: vk::SurfaceKHR,
        size: (u32, u32),
        vsync: bool,
    ) -> Result<Self> {
        let mut swapchain = Self {
            gpu: gpu.clone(),
            surface_fn: ash::khr::surface::Instance::new(&gpu.entry, &gpu.instance),
            swapchain_fn: ash::khr::swapchain::Device::new(&gpu.instance, &gpu.device),
            surface,
            swapchain: vk::SwapchainKHR::null(),
            images: Vec::new(),
            views: Vec::new(),
            rendered: Vec::new(),
            extent: vk::Extent2D::default(),
            format: vk::Format::UNDEFINED,
            present_mode: vk::PresentModeKHR::FIFO,
            vsync,
        };
        swapchain.resize(size)?;
        Ok(swapchain)
    }

    /// Recreates the swapchain for the window's new size. No frame may be using it
    /// (the caller waits for the GPU first).
    pub fn resize(&mut self, size: (u32, u32)) -> Result<()> {
        let gpu = self.gpu.clone();
        let s = &self.surface_fn;
        let (caps, formats, modes) = unsafe {
            (
                s.get_physical_device_surface_capabilities(gpu.physical, self.surface)?,
                s.get_physical_device_surface_formats(gpu.physical, self.surface)?,
                s.get_physical_device_surface_present_modes(gpu.physical, self.surface)?,
            )
        };
        let format = choose_format(&formats).ok_or_else(|| {
            Error(format!(
                "the window offers no 8-bit sRGB or UNORM format (only {:?})",
                formats.iter().map(|f| f.format).collect::<Vec<_>>()
            ))
        })?;
        // The format can't change while the window lives: the pipeline is made for it.
        if self.format != vk::Format::UNDEFINED && format.format != self.format {
            return Err(Error(format!(
                "the window's format changed from {:?} to {:?}",
                self.format, format.format
            )));
        }
        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: size
                    .0
                    .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: size
                    .1
                    .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };
        let mut count = caps.min_image_count + 1;
        if caps.max_image_count != 0 {
            count = count.min(caps.max_image_count);
        }
        let present_mode = choose_present_mode(&modes, self.vsync);
        let old = self.swapchain;
        self.swapchain = unsafe {
            self.swapchain_fn.create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(self.surface)
                    .min_image_count(count)
                    .image_format(format.format)
                    .image_color_space(format.color_space)
                    .image_extent(extent)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                    .present_mode(present_mode)
                    .clipped(true)
                    .old_swapchain(old),
                None,
            )?
        };
        self.destroy_images();
        unsafe { self.swapchain_fn.destroy_swapchain(old, None) };
        self.images = unsafe { self.swapchain_fn.get_swapchain_images(self.swapchain)? };
        for &image in &self.images {
            let view = unsafe {
                gpu.device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format.format)
                        .subresource_range(subresource(vk::ImageAspectFlags::COLOR)),
                    None,
                )?
            };
            self.views.push(view);
            let sem = unsafe {
                gpu.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?
            };
            self.rendered.push(sem);
        }
        self.extent = extent;
        self.format = format.format;
        self.present_mode = present_mode;
        Ok(())
    }

    fn destroy_images(&mut self) {
        let d = &self.gpu.device;
        unsafe {
            for view in self.views.drain(..) {
                d.destroy_image_view(view, None);
            }
            for sem in self.rendered.drain(..) {
                d.destroy_semaphore(sem, None);
            }
        }
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        self.gpu.wait_idle();
        self.destroy_images();
        unsafe {
            self.swapchain_fn.destroy_swapchain(self.swapchain, None);
            self.surface_fn.destroy_surface(self.surface, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(format: vk::Format) -> vk::SurfaceFormatKHR {
        vk::SurfaceFormatKHR {
            format,
            color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        }
    }

    #[test]
    fn formats_prefer_srgb() {
        let available = [
            fmt(vk::Format::R8G8B8A8_UNORM),
            fmt(vk::Format::B8G8R8A8_SRGB),
        ];
        assert_eq!(
            choose_format(&available),
            Some(fmt(vk::Format::B8G8R8A8_SRGB))
        );
        assert_eq!(
            choose_format(&[fmt(vk::Format::R8G8B8A8_UNORM)]),
            Some(fmt(vk::Format::R8G8B8A8_UNORM))
        );
        assert_eq!(choose_format(&[fmt(vk::Format::R16G16B16A16_SFLOAT)]), None);
        // HDR colour spaces are not ours to draw into.
        let hdr = vk::SurfaceFormatKHR {
            format: vk::Format::B8G8R8A8_SRGB,
            color_space: vk::ColorSpaceKHR::EXTENDED_SRGB_LINEAR_EXT,
        };
        assert_eq!(choose_format(&[hdr]), None);
    }

    #[test]
    fn present_modes() {
        use vk::PresentModeKHR as M;
        let all = [M::FIFO, M::MAILBOX, M::IMMEDIATE];
        assert_eq!(choose_present_mode(&all, false), M::IMMEDIATE);
        assert_eq!(choose_present_mode(&all, true), M::FIFO);
        assert_eq!(
            choose_present_mode(&[M::FIFO, M::MAILBOX], false),
            M::MAILBOX
        );
        assert_eq!(choose_present_mode(&[M::FIFO], false), M::FIFO);
    }
}
