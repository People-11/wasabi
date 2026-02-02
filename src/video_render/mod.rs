//! Video rendering module for offline video generation
//!
//! This module provides the core functionality for rendering MIDI playback
//! to video files using FFmpeg.

pub mod ffmpeg_encoder;
pub mod keyboard_renderer;
pub mod offscreen_renderer;
pub mod overlay_renderer;
pub mod render_loop;
pub mod text_renderer;
pub mod utils;

use std::path::PathBuf;

use crate::gui::window::render_state::{RenderFrameRate, RenderResolution};
use crate::settings::WasabiSettings;

#[derive(Clone)]
pub struct RenderConfig {
    pub midi_path: PathBuf,
    pub ffmpeg_path: PathBuf,
    pub output_path: PathBuf,
    pub resolution: RenderResolution,
    pub frame_rate: RenderFrameRate,
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
        quality: u8,
        settings: WasabiSettings,
    ) -> Self {
        Self {
            midi_path,
            ffmpeg_path,
            output_path,
            resolution,
            frame_rate,
            quality,
            start_delay: settings.midi.start_delay,
            settings,
        }
    }
}
