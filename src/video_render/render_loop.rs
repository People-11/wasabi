use std::{sync::atomic::Ordering, thread};

use time::Duration;

use super::{ffmpeg_encoder::FFmpegEncoder, offscreen_renderer::OffscreenRenderer, RenderConfig};
use crate::{
    audio_playback::WasabiAudioPlayer,
    gui::window::render_state::{ParseMode, RenderProgress},
    midi::{LiveLoadMIDIFile, MIDIFileBase, MIDIFileUnion, PieMIDIFile},
};

pub fn start_render(config: RenderConfig, progress: RenderProgress) {
    thread::spawn(move || {
        if let Err(e) = run_render_loop(config, &progress) {
            *progress.error.lock().unwrap() = Some(e);
        }
        progress.is_complete.store(true, Ordering::Relaxed);
    });
}

fn run_render_loop(config: RenderConfig, progress: &RenderProgress) -> Result<(), String> {
    let (w, h) = config.resolution.dimensions();
    let fps = config.frame_rate.value();
    let start = -config.settings.midi.start_delay;

    let mut renderer = OffscreenRenderer::new(&config).map_err(|e| format!("Init error: {e}"))?;
    let (player, midi_settings) = (WasabiAudioPlayer::empty(), &config.settings.midi);
    let mut midi = match config.parse_mode {
        ParseMode::Live => {
            LiveLoadMIDIFile::load_from_file(&config.midi_path, player, midi_settings)
                .map(MIDIFileUnion::Live)
        }
        ParseMode::Pie => PieMIDIFile::load_from_file(&config.midi_path, player, midi_settings)
            .map(MIDIFileUnion::Pie),
    }
    .map_err(|e| e.to_string())?;

    let midi_len = loop {
        if let Some(len) = midi.midi_length() {
            break len;
        }
        thread::sleep(std::time::Duration::from_millis(100));
    };
    progress.is_parsing.store(false, Ordering::Relaxed);

    let total_frames = ((midi_len + 2.0 - start) * fps as f64).ceil() as u64;
    progress.total_frames.store(total_frames, Ordering::Relaxed);
    let mut encoder = FFmpegEncoder::new(
        &config.ffmpeg_path,
        &config.output_path,
        w,
        h,
        fps,
        config.quality,
    )
    .map_err(|e| format!("FFmpeg error: {e}"))?;

    // Not encoded: the first draw uploads the note data and egui's font atlas
    let note_speed = config.settings.scene.note_speed;
    renderer.render_frame_into(&mut midi, note_speed, &config.settings, start, |_| Ok(()))?;

    for frame in 0..total_frames {
        if progress.is_cancelled.load(Ordering::Relaxed) {
            break;
        }
        let time = start + frame as f64 / fps as f64;
        midi.timer_mut().seek(Duration::seconds_f64(time));
        renderer.render_frame_into(&mut midi, note_speed, &config.settings, time, |pixels| {
            encoder
                .write_frame(pixels)
                .map_err(|e| format!("FFmpeg error: {e}"))
        })?;
        progress.current_frame.store(frame + 1, Ordering::Relaxed);
    }
    encoder.finish().map_err(|e| format!("FFmpeg error: {e}"))
}
