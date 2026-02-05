//! Video rendering module for offline video generation
//!
//! This module provides the core functionality for rendering MIDI playback
//! to video files using FFmpeg.

pub mod egui_render_pass;
pub mod ffmpeg_encoder;
pub mod offscreen_renderer;
pub mod render_loop;

use std::path::PathBuf;

use crate::gui::window::render_state::{ParseMode, RenderFrameRate, RenderResolution};
use crate::settings::WasabiSettings;

#[derive(Clone)]
pub struct RenderConfig {
    pub midi_path: PathBuf,
    pub ffmpeg_path: PathBuf,
    pub output_path: PathBuf,
    pub resolution: RenderResolution,
    pub frame_rate: RenderFrameRate,
    pub parse_mode: ParseMode,
    pub quality: u8,
    pub start_delay: f64,
    pub settings: WasabiSettings,
}

impl RenderConfig {
    pub fn new(
        midi_path: PathBuf,
        ffmpeg_path: PathBuf,
        output_path: PathBuf,
        resolution: RenderResolution,
        frame_rate: RenderFrameRate,
        parse_mode: ParseMode,
        quality: u8,
        settings: WasabiSettings,
    ) -> Self {
        Self {
            midi_path,
            ffmpeg_path,
            output_path,
            resolution,
            frame_rate,
            parse_mode,
            quality,
            start_delay: settings.midi.start_delay,
            settings,
        }
    }
}
