use std::{collections::VecDeque, sync::OnceLock, time::Instant};

use egui::{Context, Frame, Pos2};
use numfmt::{Formatter, Precision};

use crate::{
    gui::window::GuiWasabiWindow,
    midi::{MIDIFileBase, MIDIFileStats},
    settings::{Statistics, WasabiSettings},
    utils::convert_seconds_to_time_string,
};

#[derive(Clone, Default)]
pub struct GuiMidiStats {
    pub time_passed: f64,
    pub time_total: f64,
    pub notes_on_screen: u64,
    pub polyphony: Option<u64>,
    pub voice_count: Option<u64>,
    pub fps: u32,
    pub nps: u64,
    pub note_stats: MIDIFileStats,
}

pub fn draw_stats_panel(
    ctx: &Context,
    pos: Pos2,
    stats: &GuiMidiStats,
    settings: &WasabiSettings,
    is_video_render: bool,
) {
    let opacity = settings.scene.statistics.opacity.clamp(0.0, 1.0);
    let alpha = (u8::MAX as f32 * opacity).round() as u8;

    let round = 8;

    let mut stats_frame = Frame::default()
        .inner_margin(egui::Margin::same(7))
        .fill(egui::Color32::from_black_alpha(alpha));

    if settings.scene.statistics.floating {
        stats_frame = stats_frame.corner_radius(egui::CornerRadius::same(round));
    } else {
        stats_frame = stats_frame.corner_radius(egui::CornerRadius {
            ne: 0,
            nw: 0,
            sw: 0,
            se: round,
        });
    }

    if settings.scene.statistics.border {
        stats_frame =
            stats_frame.stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 50, 50)));
    }

    egui::Window::new("Stats")
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .scroll([false, false])
        .interactable(false)
        .frame(stats_frame)
        .fixed_pos(pos)
        .fixed_size(egui::Vec2::new(200.0, 128.0))
        .show(ctx, |ui| {
            ui.spacing_mut().interact_size.y = 16.0;

            let mut f = Formatter::new()
                .separator(',')
                .unwrap()
                .precision(Precision::Decimals(0));
            let mut num = |n: u64| f.fmt2(n).to_string();

            for (stat, _) in settings.scene.statistics.order.iter().filter(|i| i.1) {
                match stat {
                    Statistics::Time => stat_row(
                        ui,
                        "Time:",
                        format!(
                            "{} / {}",
                            convert_seconds_to_time_string(stats.time_passed),
                            convert_seconds_to_time_string(stats.time_total)
                        ),
                    ),
                    // FPS and voice count mean nothing in a rendered video
                    Statistics::Fps if !is_video_render => {
                        stat_row(ui, "FPS:", num(stats.fps as u64))
                    }
                    Statistics::VoiceCount if !is_video_render => {
                        if let Some(voice_count) = stats.voice_count {
                            stat_row(ui, "Voice Count:", num(voice_count));
                        }
                    }
                    Statistics::Rendered => stat_row(ui, "Rendered:", num(stats.notes_on_screen)),
                    Statistics::NoteCount => {
                        let passed = stats.note_stats.passed_notes.map_or("-".into(), &mut num);
                        let total = stats.note_stats.total_notes.map_or("-".into(), &mut num);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            let text = format!("{passed} / {total}");
                            let rect = ui.monospace(&text).rect;
                            pad_quads(ui, rect, &text);
                        });
                    }
                    Statistics::Nps => stat_row(ui, "NPS:", num(stats.nps)),
                    Statistics::Polyphony => {
                        if let Some(poly) = stats.polyphony {
                            stat_row(ui, "Polyphony:", num(poly));
                        }
                    }
                    Statistics::Fps | Statistics::VoiceCount => {}
                }
            }
        });
}

/// A "label ... value" line of the statistics panel
fn stat_row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.monospace(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let rect = ui.monospace(&value).rect;
            pad_quads(ui, rect, &value);
        });
    });
}

/// Glyph quads every stat value is padded to (longest: "999,999,999 / 999,999,999")
const VALUE_QUADS: usize = 32;

/// egui uploads a frame's vertices as one sub-allocation, and when its size changes from
/// frame to frame vulkano's buffer range tracking fragments without bound (see the keyboard).
/// Stat values change length all the time (9 -> 12 notes), so each one is topped up with
/// invisible zero-area quads to a fixed count. Whitespace has no glyph quad.
fn pad_quads(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    let glyphs = text.chars().filter(|c| !c.is_whitespace()).count();
    let missing = VALUE_QUADS.saturating_sub(glyphs);
    if missing == 0 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    for _ in 0..missing {
        let i = mesh.vertices.len() as u32;
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 2, i + 1, i + 3);
        for _ in 0..4 {
            // Inside the clip rect, or the tessellator culls the mesh
            mesh.colored_vertex(rect.center(), egui::Color32::TRANSPARENT);
        }
    }
    ui.painter().add(mesh);
}

impl GuiWasabiWindow {
    pub fn draw_stats(
        &mut self,
        ctx: &Context,
        pos: Pos2,
        mut stats: GuiMidiStats,
        settings: &WasabiSettings,
        is_video_render: bool,
    ) {
        if let Some(midi_file) = self.midi_file.as_mut() {
            stats.time_total = midi_file.midi_length().unwrap_or(0.0);
            let time = midi_file.timer().get_time().as_seconds_f64();

            if time > stats.time_total {
                stats.time_passed = stats.time_total;
            } else {
                stats.time_passed = time;
            }

            stats.note_stats = midi_file.stats();
        }

        self.nps.tick(stats.note_stats.passed_notes.unwrap_or(0) as i64);
        stats.nps = self.nps.read();

        stats.fps = self.fps.get_fps() as u32;

        draw_stats_panel(ctx, pos, &stats, settings, is_video_render);
    }
}

#[derive(Default)]
pub struct NpsCounter {
    /// (time in seconds, notes passed)
    ticks: VecDeque<(f64, i64)>,
}

impl NpsCounter {
    const NPS_WINDOW: f64 = 0.5;

    /// Samples at wall-clock time, for live playback
    pub fn tick(&mut self, passed: i64) {
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        let now = EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64();
        self.tick_at(now, passed);
    }

    /// Samples at an explicit time. Video export passes the playback time, since its
    /// frames are rendered at whatever speed the machine manages, not in real time.
    pub fn tick_at(&mut self, time: f64, passed: i64) {
        self.ticks.push_back((time, passed));
        while let Some(&(front_time, _)) = self.ticks.front() {
            if time - front_time > Self::NPS_WINDOW {
                self.ticks.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn read(&self) -> u64 {
        let old = self.ticks.front().map_or(0.0, |(_, passed)| *passed as f64);
        let last = self.ticks.back().map_or(0.0, |(_, passed)| *passed as f64);

        ((last - old).max(0.0) / Self::NPS_WINDOW).round() as u64
    }
}
