use std::sync::Arc;

use egui_winit_vulkano::Gui;
use vulkano::{
    device::{Device, Queue},
    sync::GpuFuture,
};

use crate::renderer::swapchain::SwapchainFrame;

pub mod icons;
pub mod window;

pub struct GuiState<'a> {
    pub renderer: &'a mut GuiRenderer<'a>,

    pub frame: &'a SwapchainFrame<'a>,

    /// GPU work of this frame so far; scene renderers chain onto it instead of blocking
    pub frame_future: &'a mut Option<Box<dyn GpuFuture>>,
}

pub struct GuiRenderer<'a> {
    pub gui: &'a mut Gui,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub format: vulkano::format::Format,
}
