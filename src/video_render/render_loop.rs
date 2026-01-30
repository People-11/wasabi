//! Offline render loop for video generation
//!
//! This module handles the main rendering loop that generates video frames
//! from MIDI playback without real-time constraints.

use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};
use time::Duration;

use crate::audio_playback::WasabiAudioPlayer;
use crate::gui::window::render_state::RenderProgress;
use crate::midi::{LiveLoadMIDIFile, MIDIFileBase};

use super::ffmpeg_encoder::FFmpegEncoder;
use super::offscreen_renderer::OffscreenRenderer;
use super::RenderConfig;

/// Manages the offline video rendering process
pub struct RenderLoop {
    #[allow(dead_code)]
    handle: Option<JoinHandle<Result<(), String>>>,
}

impl RenderLoop {
    /// Start the render loop in a background thread
    pub fn start(config: RenderConfig, progress: RenderProgress) -> Self {
        let handle = thread::spawn(move || run_render_loop(config, progress));

        Self {
            handle: Some(handle),
        }
    }

    /// Check if the render loop is still running
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    /// Wait for the render loop to complete and get the result
    #[allow(dead_code)]
    pub fn join(mut self) -> Result<(), String> {
        if let Some(handle) = self.handle.take() {
            handle.join().map_err(|_| "Render thread panicked".to_string())?
        } else {
            Ok(())
        }
    }
}

/// Main render loop function
fn run_render_loop(config: RenderConfig, progress: RenderProgress) -> Result<(), String> {
    let (width, height) = config.resolution.dimensions();
    let fps = config.frame_rate.value();
    let frame_duration_secs = 1.0 / fps as f64;

    println!("[RenderLoop] Starting GPU rendering: {}x{} @ {} FPS", width, height, fps);

    // Initialize offscreen renderer
    let mut renderer = OffscreenRenderer::new(width, height, &config.settings)
        .map_err(|e| format!("Failed to create offscreen renderer: {}", e))?;

    println!("[RenderLoop] Offscreen renderer initialized");

    // Create a silent audio player (no actual audio output during rendering)
    let silent_player = WasabiAudioPlayer::empty();

    // Load MIDI file using Live mode
    let mut midi_file = LiveLoadMIDIFile::load_from_file(
        &config.midi_path,
        silent_player,
        &config.settings.midi,
    )
    .map_err(|e| format!("Failed to load MIDI: {:?}", e))?;

    println!("[RenderLoop] MIDI file loaded");

    // Wait for MIDI length to be parsed
    let mut midi_length = None;
    for _ in 0..100 {
        // Try for up to 10 seconds
        if let Some(len) = midi_file.midi_length() {
            midi_length = Some(len);
            break;
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }

    let midi_length = midi_length.ok_or("Failed to get MIDI length")?;
    println!("[RenderLoop] MIDI length: {:.2} seconds", midi_length);

    // Calculate total frames
    let total_time = midi_length + config.start_delay;
    let total_frames = (total_time * fps as f64).ceil() as u64;
    progress.total_frames.store(total_frames, Ordering::Relaxed);
    println!("[RenderLoop] Total frames to render: {}", total_frames);

    // Initialize FFmpeg encoder
    let mut encoder = FFmpegEncoder::new(
        &config.ffmpeg_path,
        &config.output_path,
        width,
        height,
        fps,
    )
    .map_err(|e| format!("Failed to start FFmpeg: {}", e))?;

    println!("[RenderLoop] FFmpeg encoder started");

    // Get view range from settings
    let view_range = config.settings.scene.note_speed;

    // Render loop
    let mut current_time = -config.start_delay;
    let mut frame_num = 0u64;

    while current_time < midi_length {
        // Check for cancellation
        if progress.is_cancelled.load(Ordering::Relaxed) {
            encoder.cancel().ok();
            return Err("Rendering cancelled by user".to_string());
        }

        // Seek to current time
        midi_file.timer_mut().seek(Duration::seconds_f64(current_time));

        // Render frame using GPU
        let frame_buffer = renderer.render_frame(&mut midi_file, view_range, &config.settings, current_time)
            .map_err(|e| format!("Failed to render frame {}: {}", frame_num, e))?;

        // Write frame to FFmpeg
        encoder
            .write_frame(&frame_buffer)
            .map_err(|e| format!("Failed to write frame: {}", e))?;

        // Update progress
        frame_num += 1;
        progress.current_frame.store(frame_num, Ordering::Relaxed);

        // Log progress periodically
        if frame_num % 100 == 0 {
            let percent = (frame_num as f64 / total_frames as f64) * 100.0;
            println!("[RenderLoop] Progress: {:.1}% ({}/{})", percent, frame_num, total_frames);
        }

        // Advance time
        current_time += frame_duration_secs;
    }

    // Finish encoding
    encoder
        .finish()
        .map_err(|e| format!("Failed to finish encoding: {}", e))?;

    progress.is_complete.store(true, Ordering::Relaxed);
    println!("[RenderLoop] Rendering complete! Output: {:?}", config.output_path);
    Ok(())
}

