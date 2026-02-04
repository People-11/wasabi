use crate::{renderer::Renderer, settings::WasabiSettings, state::WasabiState, utils};
use egui_winit::winit::event::WindowEvent;
use winit::{
    application::ApplicationHandler,
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Icon, WindowAttributes, WindowId},
};

const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_256.bitmap"));

pub struct WasabiApplication {
    settings: WasabiSettings,
    state: WasabiState,

    renderer: Option<Renderer>,
    current_vsync: bool,
    minimized: bool,
}

impl WasabiApplication {
    pub fn new() -> Self {
        // Load the settings values
        let state = WasabiState::new();
        let settings = WasabiSettings::new_or_load().unwrap_or_else(|e| {
            state.errors.error(&e);
            WasabiSettings::default()
        });
        settings
            .save_to_file()
            .unwrap_or_else(|e| state.errors.error(&e));

        if settings.gui.check_for_updates {
            utils::check_for_updates(&state);
        }

        let current_vsync = !settings.gui.vsync;
        Self {
            settings,
            state,
            renderer: None,
            current_vsync,
            minimized: false,
        }
    }
}

impl ApplicationHandler for WasabiApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            let win_attr = WindowAttributes::default()
                .with_window_icon(Some(Icon::from_rgba(ICON.to_vec(), 256, 256).unwrap()))
                .with_inner_size(crate::WINDOW_SIZE)
                .with_title("Wasabi");
            let window = event_loop.create_window(win_attr).unwrap();
            self.renderer = Some(Renderer::new(
                event_loop,
                window,
                &mut self.settings,
                &self.state,
            ))
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        if let Some(renderer) = self.renderer.as_mut() {
            if !self.minimized {
                renderer.window().request_redraw();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(renderer) = self.renderer.as_mut() {
            // First process the redraw request
            if matches!(event, WindowEvent::RedrawRequested) {
                renderer.render(&mut self.settings, &mut self.state);
                // Use on-demand repainting during video rendering to save CPU
                if self.state.render_state.is_rendering {
                    if renderer.gui().context().has_requested_repaint() {
                        event_loop.set_control_flow(ControlFlow::Poll);
                        renderer.window().request_redraw();
                    } else {
                        event_loop.set_control_flow(ControlFlow::Wait);
                    }
                }
                // Update VSYNC if changed during render
                if self.settings.gui.vsync != self.current_vsync {
                    renderer.set_vsync(self.settings.gui.vsync);
                    self.current_vsync = self.settings.gui.vsync;
                }
                return;
            }

            if matches!(event, WindowEvent::CursorMoved { .. }) {
                let _ = renderer.gui().update(&event);
                return;
            }

            let _pass_events_to_game = !renderer.gui().update(&event);
            match event {
                WindowEvent::Resized(size) => {
                    if size.width == 0 || size.height == 0 {
                        self.minimized = true;
                        event_loop.set_control_flow(ControlFlow::Wait);
                    } else {
                        self.minimized = false;
                        event_loop.set_control_flow(ControlFlow::Poll);
                        renderer.resize(Some(size));
                    }
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    renderer.resize(None);
                }
                WindowEvent::CloseRequested => {
                    event_loop.exit();
                }
                WindowEvent::DroppedFile(ref path) => {
                    renderer
                        .gui_window()
                        .load_midi(path.clone(), &mut self.settings, &self.state);
                }
                _ => (),
            }

            if self.state.fullscreen {
                let mode = event_loop
                    .available_monitors()
                    .next()
                    .unwrap()
                    .video_modes()
                    .next()
                    .unwrap();

                renderer.set_fullscreen(mode);
                self.state.fullscreen = false;
            }

            if self.settings.gui.vsync != self.current_vsync {
                renderer.set_vsync(self.settings.gui.vsync);
                self.current_vsync = self.settings.gui.vsync;
            }
        }
    }
}
