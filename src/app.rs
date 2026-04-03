use crate::{renderer::Renderer, settings::WasabiSettings, state::WasabiState, utils};
use egui_winit::winit::event::WindowEvent;
use winit::{
    application::ApplicationHandler,
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Icon, WindowAttributes, WindowId},
};

use std::time::{Duration, Instant};

const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_256.bitmap"));

pub struct WasabiApplication {
    settings: WasabiSettings,
    state: WasabiState,

    renderer: Option<Renderer>,
    current_vsync: bool,
    minimized: bool,

    render_timer: Option<Instant>,
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
            render_timer: None,
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

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        let Some(renderer) = self.renderer.as_mut() else { return };
        if self.minimized { return; }

        let is_rendering = self.state.render_state.is_rendering;
        let should_redraw = if is_rendering {
            matches!(
                cause,
                winit::event::StartCause::Init | winit::event::StartCause::ResumeTimeReached { .. }
            ) && self.render_timer.map_or(true, |t| Instant::now() >= t)
        } else {
            true
        };

        if should_redraw {
            renderer.window().request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(renderer) = self.renderer.as_mut() {
            if matches!(event, WindowEvent::RedrawRequested) {
                // Sync VSync: Disable during rendering to bypass OS limits, otherwise follow settings.
                let target_vsync = !self.state.render_state.is_rendering && self.settings.gui.vsync;
                if self.current_vsync != target_vsync {
                    renderer.set_vsync(target_vsync);
                    self.current_vsync = target_vsync;
                }

                renderer.render(&mut self.settings, &mut self.state);

                if self.state.render_state.is_rendering {
                    let next = Instant::now() + Duration::from_millis(16);
                    self.render_timer = Some(next);
                    event_loop.set_control_flow(ControlFlow::WaitUntil(next));
                } else {
                    self.render_timer = None;
                    event_loop.set_control_flow(if self.minimized { ControlFlow::Wait } else { ControlFlow::Poll });
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
                    self.minimized = size.width == 0 || size.height == 0;
                    if self.minimized {
                        event_loop.set_control_flow(ControlFlow::Wait);
                    } else {
                        event_loop.set_control_flow(ControlFlow::Poll);
                        renderer.resize(Some(size));
                    }
                }
                WindowEvent::ScaleFactorChanged { .. } => renderer.resize(None),
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::DroppedFile(ref path) => {
                    renderer.gui_window().load_midi(path.clone(), &mut self.settings, &self.state);
                }
                _ => (),
            }

            if self.state.fullscreen {
                if let Some(monitor) = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next()) {
                    if let Some(mode) = monitor.video_modes().next() {
                        renderer.set_fullscreen(mode);
                    }
                }
                self.state.fullscreen = false;
            }
        }
    }
}
