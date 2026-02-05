use std::{sync::atomic::Ordering, thread};
use time::Duration;
use crate::{audio_playback::WasabiAudioPlayer, gui::window::render_state::RenderProgress, midi::{LiveLoadMIDIFile, MIDIFileBase}};
use super::{ffmpeg_encoder::FFmpegEncoder, offscreen_renderer::OffscreenRenderer, RenderConfig};

pub fn start_render(config: RenderConfig, progress: RenderProgress) {
    thread::spawn(move || { if let Err(e) = run_render_loop(config, progress) { eprintln!("[RenderLoop] Error: {e}"); } });
}

fn run_render_loop(config: RenderConfig, progress: RenderProgress) -> Result<(), String> {
    let (w, h) = config.resolution.dimensions();
    let fps = config.frame_rate.value();
    let frame_dur = 1.0 / fps as f64;

    let mut renderer = OffscreenRenderer::new(&config).map_err(|e| format!("Init error: {e}"))?;
    let mut midi = LiveLoadMIDIFile::load_from_file(&config.midi_path, WasabiAudioPlayer::empty(), &config.settings.midi).map_err(|e| format!("MIDI error: {e:?}"))?;
    
    let midi_len = loop { if let Some(l) = midi.midi_length() { break l; } thread::sleep(std::time::Duration::from_millis(100)); };
    progress.is_parsing.store(false, Ordering::Relaxed);

    let total_frames = ((midi_len + config.start_delay + 2.0) * fps as f64).ceil() as u64;
    progress.total_frames.store(total_frames, Ordering::Relaxed);
    let mut encoder = FFmpegEncoder::new(&config.ffmpeg_path, &config.output_path, w, h, fps, config.quality).map_err(|e| format!("FFmpeg error: {e}"))?;

    let range = config.settings.scene.note_speed as f32;
    renderer.render_frame_into(&mut vec![0u8; (w * h * 4) as usize], &mut midi, range, &config.settings, -config.start_delay).ok();

    let (mut time, mut frame_num) = (-config.start_delay, 0u64);
    while time < midi_len + 2.0 {
        if progress.is_cancelled.load(Ordering::Relaxed) { return encoder.cancel().map_err(|e| e.to_string()); }
        midi.timer_mut().seek(Duration::seconds_f64(time));
        let mut buf = encoder.get_buffer();
        renderer.render_frame_into(&mut buf, &mut midi, range, &config.settings, time)?;
        encoder.write_frame(buf).map_err(|e| format!("Write error: {e}"))?;
        frame_num += 1;
        progress.current_frame.store(frame_num, Ordering::Relaxed);
        if frame_num % 100 == 0 { println!("[RenderLoop] Progress: {:.1}% ({frame_num}/{total_frames})", (frame_num as f64 / total_frames as f64) * 100.0); }
        time += frame_dur;
    }
    encoder.finish().map_err(|e| format!("Finish error: {e}"))?;
    progress.is_complete.store(true, Ordering::Relaxed);
    Ok(())
}
