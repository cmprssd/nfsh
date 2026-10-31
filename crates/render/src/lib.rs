//! nfsh's Vulkan renderer: device bring-up, a window's swapchain and frame loop, an
//! offscreen target for screenshots, and a zone drawn untextured.

pub mod camera;
pub mod frame;
pub mod gpu;
pub mod resource;
pub mod scene;
pub mod swapchain;
pub mod zone;

pub use ash::vk;
pub use camera::Camera;
pub use frame::{FRAMES_IN_FLIGHT, FrameLoop, Presented};
pub use gpu::{DeviceKind, Error, Gpu, Result};
pub use resource::{Buffer, Image, Target};
pub use scene::Scene;
pub use swapchain::Swapchain;
pub use zone::ZoneRenderer;
