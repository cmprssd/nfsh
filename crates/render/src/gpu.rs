//! The Vulkan instance, device and memory allocator, shared by every GPU resource.

use std::ffi::CStr;
use std::sync::{Arc, Mutex, MutexGuard};

use ash::vk;
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<vk::Result> for Error {
    fn from(r: vk::Result) -> Self {
        Self(format!("Vulkan: {r:?}"))
    }
}

impl From<gpu_allocator::AllocationError> for Error {
    fn from(e: gpu_allocator::AllocationError) -> Self {
        Self(format!("GPU memory: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Locks `m` even if a thread panicked while holding it: what it guards (a queue, the
/// allocator) stays usable.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What kind of device was picked, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceKind {
    Discrete,
    Integrated,
    Virtual,
    Cpu,
    Other,
}

pub struct Gpu {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical: vk::PhysicalDevice,
    pub device: ash::Device,
    /// Locked for every submit and present: Vulkan requires one thread at a time on a
    /// queue.
    pub queue: Mutex<vk::Queue>,
    pub queue_family: u32,
    pub name: String,
    pub kind: DeviceKind,
    /// Nanoseconds per timestamp tick.
    pub timestamp_period: f32,
    /// The most anisotropic filtering samplers may ask for; `None` without it.
    pub max_anisotropy: Option<f32>,
    /// `None` only while the device is dropped.
    pub(crate) allocator: Mutex<Option<Allocator>>,
    /// For [`Gpu::submit_and_wait`], which holds it while recording.
    command_pool: Mutex<vk::CommandPool>,
}

impl Gpu {
    /// A device for rendering without a window (screenshots, tests). Requires Vulkan
    /// 1.3 with dynamic rendering and synchronization2. `NFSH_GPU` picks a device
    /// whose name contains the given text.
    pub fn new_headless() -> Result<Arc<Self>> {
        Ok(Self::create(None)?.0)
    }

    /// A device that can also present to a window, and the window's surface (for
    /// [`crate::Swapchain`], which destroys it). Only devices that can present to the
    /// surface from their graphics queue are considered.
    pub fn new_for_window(
        display: RawDisplayHandle,
        window: RawWindowHandle,
    ) -> Result<(Arc<Self>, vk::SurfaceKHR)> {
        let (gpu, surface) = Self::create(Some((display, window)))?;
        let surface = surface.ok_or_else(|| Error("no window surface".into()))?;
        Ok((gpu, surface))
    }

    fn create(
        window: Option<(RawDisplayHandle, RawWindowHandle)>,
    ) -> Result<(Arc<Self>, Option<vk::SurfaceKHR>)> {
        let entry =
            unsafe { ash::Entry::load() }.map_err(|e| Error(format!("no Vulkan loader: {e}")))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"nfsh")
            .engine_name(c"nfsh")
            .api_version(vk::API_VERSION_1_3);
        let extensions = match window {
            Some((display, _)) => ash_window::enumerate_required_extensions(display)?,
            None => &[],
        };
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app)
                    .enabled_extension_names(extensions),
                None,
            )?
        };
        let surface = match window {
            Some((display, window)) => Some(
                match unsafe {
                    ash_window::create_surface(&entry, &instance, display, window, None)
                } {
                    Ok(s) => s,
                    Err(e) => {
                        unsafe { instance.destroy_instance(None) };
                        return Err(e.into());
                    }
                },
            ),
            None => None,
        };
        let surface_fn = ash::khr::surface::Instance::new(&entry, &instance);
        let give_up = |message: String| {
            unsafe {
                if let Some(s) = surface {
                    surface_fn.destroy_surface(s, None);
                }
                instance.destroy_instance(None);
            }
            Error(message)
        };

        let wanted = std::env::var("NFSH_GPU")
            .ok()
            .map(|s| s.to_ascii_lowercase());
        let physicals = match unsafe { instance.enumerate_physical_devices() } {
            Ok(p) => p,
            Err(e) => return Err(give_up(format!("Vulkan: {e:?}"))),
        };
        let mut candidates = Vec::new();
        for physical in physicals {
            let props = unsafe { instance.get_physical_device_properties(physical) };
            let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            if props.api_version < vk::API_VERSION_1_3
                || wanted
                    .as_ref()
                    .is_some_and(|w| !name.to_ascii_lowercase().contains(w))
            {
                continue;
            }
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(physical) };
            let Some(family) = families.iter().enumerate().position(|(i, f)| {
                f.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                    && surface.is_none_or(|s| unsafe {
                        surface_fn
                            .get_physical_device_surface_support(physical, i as u32, s)
                            .unwrap_or(false)
                    })
            }) else {
                continue;
            };
            let kind = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => DeviceKind::Discrete,
                vk::PhysicalDeviceType::INTEGRATED_GPU => DeviceKind::Integrated,
                vk::PhysicalDeviceType::VIRTUAL_GPU => DeviceKind::Virtual,
                vk::PhysicalDeviceType::CPU => DeviceKind::Cpu,
                _ => DeviceKind::Other,
            };
            let period = props.limits.timestamp_period;
            let anisotropy = props.limits.max_sampler_anisotropy;
            candidates.push((kind, physical, family as u32, name, period, anisotropy));
        }
        candidates.sort_by_key(|c| c.0);
        let Some((kind, physical, queue_family, name, timestamp_period, anisotropy)) =
            candidates.into_iter().next()
        else {
            return Err(give_up(format!(
                "no Vulkan 1.3 device with a graphics queue{}",
                if surface.is_some() {
                    " that can present to the window"
                } else {
                    ""
                }
            )));
        };

        let priorities = [1.0];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities)];
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true);
        // Block-compressed textures and anisotropic filtering where the device has
        // them: a texture whose format can't be sampled is drawn grey instead.
        let supported = unsafe { instance.get_physical_device_features(physical) };
        let features = vk::PhysicalDeviceFeatures::default()
            .texture_compression_bc(supported.texture_compression_bc == vk::TRUE)
            .sampler_anisotropy(supported.sampler_anisotropy == vk::TRUE);
        let max_anisotropy = (supported.sampler_anisotropy == vk::TRUE).then_some(anisotropy);
        let device_extensions = match surface {
            Some(_) => &[ash::khr::swapchain::NAME.as_ptr()][..],
            None => &[],
        };
        let device = match unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queue_info)
                    .enabled_extension_names(device_extensions)
                    .enabled_features(&features)
                    .push_next(&mut features13),
                None,
            )
        } {
            Ok(d) => d,
            Err(e) => return Err(give_up(format!("{name}: Vulkan: {e:?}"))),
        };
        let queue = unsafe { device.get_device_queue(queue_family, 0) };
        let made = Allocator::new(&AllocatorCreateDesc {
            instance: instance.clone(),
            device: device.clone(),
            physical_device: physical,
            debug_settings: Default::default(),
            buffer_device_address: false,
            allocation_sizes: Default::default(),
        })
        .map_err(Error::from)
        .and_then(|allocator| {
            let pool = unsafe {
                device.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(queue_family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
            }?;
            Ok((allocator, pool))
        });
        let (allocator, command_pool) = match made {
            Ok(m) => m,
            Err(e) => {
                unsafe { device.destroy_device(None) };
                return Err(give_up(e.0));
            }
        };
        let gpu = Arc::new(Self {
            entry,
            instance,
            physical,
            device,
            queue: Mutex::new(queue),
            queue_family,
            name,
            kind,
            timestamp_period,
            max_anisotropy,
            allocator: Mutex::new(Some(allocator)),
            command_pool: Mutex::new(command_pool),
        });
        Ok((gpu, surface))
    }

    /// Waits until the GPU is idle (`vkDeviceWaitIdle`, which needs the queue to
    /// itself).
    pub fn wait_idle(&self) {
        let _queue = lock(&self.queue);
        let _ = unsafe { self.device.device_wait_idle() };
    }

    /// Records commands with `record`, submits them and waits until the GPU is done.
    pub fn submit_and_wait(&self, record: impl FnOnce(vk::CommandBuffer)) -> Result<()> {
        let d = &self.device;
        let pool = lock(&self.command_pool);
        unsafe {
            let cmd = d.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(*pool)
                    .command_buffer_count(1),
            )?[0];
            let cmds = [cmd];
            let run = || -> Result<()> {
                d.begin_command_buffer(
                    cmd,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )?;
                record(cmd);
                d.end_command_buffer(cmd)?;
                let fence = d.create_fence(&vk::FenceCreateInfo::default(), None)?;
                let submitted = d.queue_submit(
                    *lock(&self.queue),
                    &[vk::SubmitInfo::default().command_buffers(&cmds)],
                    fence,
                );
                let waited = submitted.and_then(|()| d.wait_for_fences(&[fence], true, u64::MAX));
                d.destroy_fence(fence, None);
                Ok(waited?)
            };
            let result = run();
            d.free_command_buffers(*pool, &cmds);
            result
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            // The allocator must go before the device it allocates from.
            lock(&self.allocator).take();
            let pool = *self
                .command_pool
                .get_mut()
                .unwrap_or_else(|e| e.into_inner());
            self.device.destroy_command_pool(pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
