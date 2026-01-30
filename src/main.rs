#![feature(type_alias_impl_trait)]
#![feature(coroutines)]
#![feature(impl_trait_in_assoc_type)]

#![windows_subsystem = "windows"]
mod app;
mod audio_playback;
mod gui;
mod midi;
mod renderer;
mod scenes;
mod settings;
mod state;
mod utils;
mod video_render;

use app::WasabiApplication;
use vulkano::swapchain::PresentMode;

use egui_winit::winit::{
    dpi::{LogicalSize, Size},
    event_loop::EventLoop,
};
use winit::event_loop::ControlFlow;

pub const WINDOW_SIZE: Size = Size::Logical(LogicalSize {
    width: 1280.0,
    height: 720.0,
});

pub const PRESENT_MODE: PresentMode = PresentMode::Immediate;
pub const WAYLAND_PRESENT_MODE: PresentMode = PresentMode::Mailbox;
pub const VSYNC_PRESENT_MODE: PresentMode = PresentMode::Fifo;

pub fn main() {
    // Attach to parent console if available (e.g. running from CMD)
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(dwProcessId: u32) -> i32;
        }
        unsafe {
            AttachConsole(u32::MAX);
        }
    }

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = WasabiApplication::new();
    event_loop.run_app(&mut app).unwrap();
}
