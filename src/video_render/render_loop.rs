//! Offline render loop for video generation
//!
//! This module handles the main rendering loop that generates video frames
//! from MIDI playback without real-time constraints.

use std::sync::atomic::Ordering;
use std::thread;
use time::Duration;

use crate::audio_playback::WasabiAudioPlayer;
use crate::gui::window::render_state::RenderProgress;
use crate::midi::{LiveLoadMIDIFile, MIDIFileBase};

use super::ffmpeg_encoder::FFmpegEncoder;
use super::offscreen_renderer::OffscreenRenderer;
use super::RenderConfig;

/// Start the render loop in a background thread
pub fn start_render(config: RenderConfig, progress: RenderProgress) {
    thread::spawn(move || {
        if let Err(e) = run_render_loop(config, progress) {
            eprintln!("[RenderLoop] Error: {}", e);
        }
    });
}

/// Main render loop function
fn run_render_loop(config: RenderConfig, progress: RenderProgress) -> Result<(), String> {
    let (width, height) = config.resolution.dimensions();
    let fps = config.frame_rate.value();
    let frame_duration_secs = 1.0 / fps as f64;

    println!(
        "[RenderLoop] Starting GPU rendering: {}x{} @ {} FPS",
        width, height, fps
    );

    // Initialize offscreen renderer
    let mut renderer = OffscreenRenderer::new(width, height)
        .map_err(|e| format!("Failed to create offscreen renderer: {}", e))?;

    println!("[RenderLoop] Offscreen renderer initialized");

    // Create a silent audio player
    let silent_player = WasabiAudioPlayer::empty();

    // Load MIDI file
    let mut midi_file =
        LiveLoadMIDIFile::load_from_file(&config.midi_path, silent_player, &config.settings.midi)
            .map_err(|e| format!("Failed to load MIDI: {:?}", e))?;

    println!("[RenderLoop] MIDI file loaded");

    // Wait for MIDI length to be parsed (can take a long time for huge MIDIs)
    let mut wait_count = 0;
    let midi_length = loop {
        if let Some(len) = midi_file.midi_length() {
            break len;
        }
        thread::sleep(std::time::Duration::from_millis(100));
        wait_count += 1;
        // Log every 5 seconds
        if wait_count % 50 == 0 {
            println!(
                "[RenderLoop] Waiting for MIDI statistics... ({}s)",
                wait_count / 10
            );
        }
    };
    println!("[RenderLoop] MIDI length: {:.2} seconds", midi_length);
    progress.is_parsing.store(false, Ordering::Relaxed);

    // Calculate total frames
    let total_time = midi_length + config.start_delay + 2.0;
    let total_frames = (total_time * fps as f64).ceil() as u64;
    progress.total_frames.store(total_frames, Ordering::Relaxed);
    println!("[RenderLoop] Total frames to render: {}", total_frames);

    // Initialize FFmpeg encoder
    let mut encoder =
        FFmpegEncoder::new(&config.ffmpeg_path, &config.output_path, width, height, fps)
            .map_err(|e| format!("Failed to start FFmpeg: {}", e))?;

    println!("[RenderLoop] FFmpeg encoder started");

    // Get view range from settings
    let view_range = config.settings.scene.note_speed;

    // Render loop
    let mut current_time = -config.start_delay;
    let mut frame_num = 0u64;

    while current_time < midi_length + 2.0 {
        // Check for cancellation
        if progress.is_cancelled.load(Ordering::Relaxed) {
            encoder.cancel().ok();
            return Err("Rendering cancelled by user".to_string());
        }

        // Seek to current time
        midi_file
            .timer_mut()
            .seek(Duration::seconds_f64(current_time));

        // Get a recycled buffer (or create new one) from the encoder
        let mut frame_buffer = encoder.get_buffer();

        // Render frame into the buffer
        renderer
            .render_frame_into(
                &mut frame_buffer,
                &mut midi_file,
                view_range,
                &config.settings,
                current_time,
            )
            .map_err(|e| format!("Failed to render frame {}: {}", frame_num, e))?;

        // Write frame to FFmpeg (passes ownership of buffer)
        encoder
            .write_frame(frame_buffer)
            .map_err(|e| format!("Failed to write frame: {}", e))?;

        // Update progress
        frame_num += 1;
        progress.current_frame.store(frame_num, Ordering::Relaxed);

        // Log progress periodically
        if frame_num % 100 == 0 {
            let percent = (frame_num as f64 / total_frames as f64) * 100.0;
            println!(
                "[RenderLoop] Progress: {:.1}% ({}/{})",
                percent, frame_num, total_frames
            );
        }

        // Advance time
        current_time += frame_duration_secs;
    }

    // Finish encoding
    encoder
        .finish()
        .map_err(|e| format!("Failed to finish encoding: {}", e))?;

    progress.is_complete.store(true, Ordering::Relaxed);
    println!(
        "[RenderLoop] Rendering complete! Output: {:?}",
        config.output_path
    );
    Ok(())
}
