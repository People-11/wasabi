use crate::{renderer::Renderer, settings::WasabiSettings, state::WasabiState};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use egui_winit::winit::event::WindowEvent;
use winit::{
    application::ApplicationHandler,
    event_loop::{ActiveEventLoop, EventLoopProxy},
    window::{Icon, Window, WindowAttributes, WindowId},
};

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon.bitmap"));

/// The main thread only pumps winit messages and forwards window events to the
/// render thread, so modal loops (dragging/resizing) and message floods can't stall frames.
pub struct WasabiApplication {
    proxy: EventLoopProxy<()>,
    init: Option<(WasabiSettings, WasabiState)>,
    events: Option<Sender<WindowEvent>>,
    // Keeps the window owned by the main thread until the render thread is gone
    window: Option<Arc<Window>>,
}

impl WasabiApplication {
    pub fn new(proxy: EventLoopProxy<()>) -> Self {
        let state = WasabiState::new();
        let settings = WasabiSettings::new_or_load().unwrap_or_else(|e| {
            state.errors.error(&e);
            WasabiSettings::default()
        });
        settings
            .save_to_file()
            .unwrap_or_else(|e| state.errors.error(&e));

        Self {
            proxy,
            init: Some((settings, state)),
            events: None,
            window: None,
        }
    }
}

impl ApplicationHandler for WasabiApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some((mut settings, state)) = self.init.take() else { return };

        let win_attr = WindowAttributes::default()
            .with_window_icon(Some(Icon::from_rgba(ICON.to_vec(), 16, 16).unwrap()))
            .with_inner_size(crate::WINDOW_SIZE)
            .with_title("Wasabi");
        let window = event_loop.create_window(win_attr).unwrap();

        // egui_winit_vulkano::Gui needs the ActiveEventLoop, so build here and hand it over
        let renderer = Renderer::new(event_loop, window, &mut settings, &state);
        self.window = Some(renderer.window());
        let renderer = SendRenderer(renderer);

        let (tx, rx) = crossbeam_channel::unbounded();
        self.events = Some(tx);
        let proxy = self.proxy.clone();
        std::thread::Builder::new()
            .name("wasabi-render".into())
            .spawn(move || {
                let _exit = ExitOnDrop(proxy);
                RenderThread::new(renderer.into_inner(), settings, state).run(rx);
            })
            .unwrap();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // The render thread draws continuously; OS paint requests are irrelevant
        if matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        // CloseRequested is forwarded too: the render thread cleans up and then wakes us to exit.
        // Exiting here directly could deadlock if the render thread is mid-call into the window.
        if self.events.as_ref().is_none_or(|tx| tx.send(event).is_err()) {
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
        // Render thread has finished (normally or by panic)
        event_loop.exit();
    }
}

// SAFETY: the only !Send fields of Renderer are in ManagedSwapchain: previous_frame_end,
// which right after construction holds a plain `sync::now` future, and the still-empty
// frames_in_flight queue.
// The Renderer is moved exactly once and the main thread keeps no references into it.
struct SendRenderer(Renderer);
unsafe impl Send for SendRenderer {}

impl SendRenderer {
    // Method (not `.0`) so the closure captures the whole wrapper, not the !Send field
    fn into_inner(self) -> Renderer {
        self.0
    }
}

struct ExitOnDrop(EventLoopProxy<()>);

impl Drop for ExitOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send_event(());
    }
}

struct RenderThread {
    renderer: Renderer,
    settings: WasabiSettings,
    state: WasabiState,
    current_vsync: bool,
    minimized: bool,
}

impl RenderThread {
    fn new(renderer: Renderer, settings: WasabiSettings, state: WasabiState) -> Self {
        let current_vsync = !settings.gui.vsync;
        Self {
            renderer,
            settings,
            state,
            current_vsync,
            minimized: false,
        }
    }

    fn run(mut self, rx: Receiver<WindowEvent>) {
        let mut next_frame = Instant::now();
        loop {
            // Sleep while minimized, or until the next frame is due (throttled only while
            // exporting video); otherwise just pick up whatever arrived.
            let first = if self.minimized {
                rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
            } else {
                rx.recv_deadline(next_frame)
            };
            let first = match first {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            for event in first.into_iter().chain(rx.try_iter()) {
                if !self.handle_event(event) {
                    return;
                }
            }
            if self.minimized || Instant::now() < next_frame {
                continue;
            }

            self.draw();
            next_frame = Instant::now();
            if self.state.render_state.is_rendering {
                next_frame += Duration::from_millis(16);
            }
        }
    }

    /// Returns false when the application should exit
    fn handle_event(&mut self, event: WindowEvent) -> bool {
        self.renderer.gui().update(&event);
        match event {
            WindowEvent::Resized(size) => {
                self.minimized = size.width == 0 || size.height == 0;
                if !self.minimized {
                    self.renderer.resize(Some(size));
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => self.renderer.resize(None),
            WindowEvent::CloseRequested => return false,
            WindowEvent::DroppedFile(path) => {
                self.renderer
                    .gui_window()
                    .load_midi(path, &mut self.settings, &self.state);
            }
            _ => (),
        }
        true
    }

    fn draw(&mut self) {
        let target_vsync = self.settings.gui.vsync;
        if self.current_vsync != target_vsync {
            self.renderer.set_vsync(target_vsync);
            self.current_vsync = target_vsync;
        }

        self.renderer.render(&mut self.settings, &mut self.state);
        #[cfg(windows)]
        if target_vsync {
            wait_for_compositor();
        }

        if self.state.fullscreen {
            let window = self.renderer.window();
            if let Some(monitor) = window
                .primary_monitor()
                .or_else(|| window.available_monitors().next())
            {
                if let Some(mode) = monitor.video_modes().next() {
                    self.renderer.set_fullscreen(mode);
                }
            }
            self.state.fullscreen = false;
        }
    }
}

/// Blocks until the desktop compositor's next frame. Fifo presentation through DWM (while
/// the window is dragged or captured, and on hybrid-GPU laptops) only gets an image back
/// every other refresh, halving the frame rate; pacing on DWM directly doesn't.
#[cfg(windows)]
fn wait_for_compositor() {
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmFlush() -> i32;
    }
    unsafe { DwmFlush() };
}
