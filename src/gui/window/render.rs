use std::{path::Path, sync::atomic::Ordering};

use egui::{ComboBox, ProgressBar};

use crate::video_render::{render_loop::start_render, RenderConfig};
use crate::{settings::WasabiSettings, state::WasabiState, utils};

use super::render_state::{ParseMode, RenderFrameRate, RenderResolution};
use super::GuiWasabiWindow;

/// File name for display, shortened to 30 characters
fn short_file_name(path: Option<&Path>) -> String {
    let Some(path) = path else {
        return "(None selected)".to_string();
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if name.chars().count() > 30 {
        format!("{}...", name.chars().take(27).collect::<String>())
    } else {
        name.into_owned()
    }
}

impl GuiWasabiWindow {
    pub fn show_render(
        &mut self,
        ctx: &egui::Context,
        settings: &mut WasabiSettings,
        state: &mut WasabiState,
    ) {
        if !state.show_render {
            return;
        }

        if state.render_state.ffmpeg_path.is_none() {
            if let Some(ref path) = settings.gui.ffmpeg_path {
                if path.exists() {
                    state.render_state.ffmpeg_path = Some(path.clone());
                }
            }
        }

        let mut frame = utils::create_window_frame(ctx);
        frame.shadow = egui::Shadow::NONE;

        let size = [500.0, 510.0];

        egui::Window::new("Render Video")
            .resizable(false)
            .collapsible(false)
            .title_bar(true)
            .scroll([false, true])
            .enabled(true)
            .frame(frame)
            .fixed_size(size)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .movable(false)
            .show(ctx, |ui| {
                ui.style_mut().interaction.selectable_labels = false;
                ui.add_space(10.0);

                let is_rendering = state.render_state.is_rendering;

                ui.add_enabled_ui(!is_rendering, |ui| {
                    self.render_settings_ui(ui, settings, state);
                });

                ui.add_space(15.0);
                ui.separator();

                if is_rendering {
                    ui.add_space(15.0);
                    self.render_progress_ui(ui, state);
                } else {
                    ui.add_space(15.0);
                    if let Some(error) = state.render_state.progress.error.lock().unwrap().as_ref()
                    {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!("Render failed: {error}"),
                        );
                        ui.add_space(10.0);
                    }
                    self.render_actions_ui(ui, settings, state);
                }
            });
    }

    fn render_settings_ui(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut WasabiSettings,
        state: &mut WasabiState,
    ) {
        ui.heading("Input Sources");
        ui.add_space(5.0);
        egui::Grid::new("render_input_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.label("MIDI File:");
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Browse...").clicked() {
                            let last_location = state.last_midi_location.clone();
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("MIDI", &["mid", "MID"])
                                .set_title("Select MIDI file")
                                .set_directory(last_location.parent().unwrap_or(Path::new("./")))
                                .pick_file()
                            {
                                let mut output = path.clone();
                                output.set_extension("mp4");
                                state.render_state.output_path = Some(output);
                                state.render_state.midi_path = Some(path);
                            }
                        }

                        let text = short_file_name(state.render_state.midi_path.as_deref());
                        ui.label(egui::RichText::new(text).strong());
                    });
                });
                ui.end_row();

                ui.label("FFmpeg:");
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Browse...").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Executable", &["exe"])
                                .set_title("Select ffmpeg.exe")
                                .pick_file()
                            {
                                state.render_state.ffmpeg_path = Some(path.clone());
                                settings.gui.ffmpeg_path = Some(path);
                                let _ = settings.save_to_file();
                            }
                        }
                        let text = short_file_name(state.render_state.ffmpeg_path.as_deref());
                        ui.label(egui::RichText::new(text).strong());
                    });
                });
                ui.end_row();

                ui.label("Parse Mode:");
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ComboBox::from_id_salt("parse_mode_combo")
                            .selected_text(state.render_state.parse_mode.label())
                            .width(100.0)
                            .show_ui(ui, |ui| {
                                for mode in [ParseMode::Live, ParseMode::Pie] {
                                    ui.selectable_value(
                                        &mut state.render_state.parse_mode,
                                        mode,
                                        mode.label(),
                                    );
                                }
                            });
                    });
                });
                ui.end_row();
            });

        ui.add_space(15.0);

        ui.heading("Output");
        ui.add_space(5.0);
        egui::Grid::new("render_output_grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.label("File:");
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Browse...").clicked() {
                            let mut dialog = rfd::FileDialog::new()
                                .add_filter("MP4 Video", &["mp4"])
                                .set_title("Save video as...");

                            if let Some(ref current_path) = state.render_state.output_path {
                                if let Some(parent) = current_path.parent() {
                                    dialog = dialog.set_directory(parent);
                                }
                                if let Some(filename) = current_path.file_name() {
                                    dialog = dialog.set_file_name(filename.to_string_lossy());
                                }
                            }

                            if let Some(path) = dialog.save_file() {
                                let path = if path.extension().is_none() {
                                    path.with_extension("mp4")
                                } else {
                                    path
                                };
                                state.render_state.output_path = Some(path);
                            }
                        }
                        let text = short_file_name(state.render_state.output_path.as_deref());
                        ui.label(egui::RichText::new(text).strong());
                    });
                });
                ui.end_row();
            });

        ui.add_space(15.0);

        ui.heading("Video Settings");
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.label("Resolution:");
            ComboBox::from_id_salt("resolution_combo")
                .selected_text(state.render_state.resolution.label())
                .width(100.0)
                .show_ui(ui, |ui| {
                    for resolution in [RenderResolution::HD1080, RenderResolution::UHD4K] {
                        ui.selectable_value(
                            &mut state.render_state.resolution,
                            resolution,
                            resolution.label(),
                        );
                    }
                });

            ui.add_space(8.0);

            ui.label("FPS:");
            ComboBox::from_id_salt("framerate_combo")
                .selected_text(state.render_state.frame_rate.label())
                .width(80.0)
                .show_ui(ui, |ui| {
                    for rate in [
                        RenderFrameRate::Fps30,
                        RenderFrameRate::Fps60,
                        RenderFrameRate::Fps120,
                    ] {
                        ui.selectable_value(&mut state.render_state.frame_rate, rate, rate.label());
                    }
                });

            ui.add_space(8.0);

            ui.label("Quality:");
            ui.add(
                egui::DragValue::new(&mut state.render_state.quality)
                    .range(1..=51)
                    .speed(0.1),
            );
        });

        ui.add_space(5.0);
        ui.label(
            egui::RichText::new(
                "Note: Other settings (colors, speed, range) use current app configuration.",
            )
            .weak()
            .small(),
        );
    }

    fn render_actions_ui(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut WasabiSettings,
        state: &mut WasabiState,
    ) {
        ui.horizontal(|ui| {
            let can_start = state.render_state.midi_path.is_some()
                && state.render_state.ffmpeg_path.is_some()
                && state.render_state.output_path.is_some();

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    state.show_render = false;
                }

                if ui
                    .add_enabled(can_start, egui::Button::new("🎬 Start Render"))
                    .clicked()
                {
                    let config = RenderConfig {
                        midi_path: state.render_state.midi_path.clone().unwrap(),
                        ffmpeg_path: state.render_state.ffmpeg_path.clone().unwrap(),
                        output_path: state.render_state.output_path.clone().unwrap(),
                        resolution: state.render_state.resolution,
                        frame_rate: state.render_state.frame_rate,
                        parse_mode: state.render_state.parse_mode,
                        quality: state.render_state.quality,
                        settings: settings.clone(),
                    };

                    state.render_state.progress.reset();
                    state.render_state.is_rendering = true;

                    start_render(config, state.render_state.progress.clone());
                }
            });
        });
    }

    fn render_progress_ui(&mut self, ui: &mut egui::Ui, state: &mut WasabiState) {
        ui.vertical_centered(|ui| {
            // Request continuous repaints while rendering to update progress
            ui.ctx().request_repaint();
            if state
                .render_state
                .progress
                .is_parsing
                .load(Ordering::Relaxed)
            {
                ui.heading("Parsing MIDI Info...");
            } else {
                ui.heading("Rendering in Progress...");
            }

            ui.add_space(15.0);

            let progress = state.render_state.progress.progress();

            ui.scope(|ui| {
                // Apply custom color ONLY to this scope
                ui.visuals_mut().selection.bg_fill = egui::Color32::from_rgb(0x66, 0x99, 0x00);
                let bar = ProgressBar::new(progress)
                    .desired_height(14.0)
                    .animate(false)
                    .corner_radius(egui::CornerRadius::ZERO);
                ui.add(bar);
            });

            ui.add_space(15.0);

            let current = state
                .render_state
                .progress
                .current_frame
                .load(Ordering::Relaxed);
            let total = state
                .render_state
                .progress
                .total_frames
                .load(Ordering::Relaxed);
            let stats = match state.render_state.progress.get_performance_stats() {
                Some((fps, eta)) => format!("{fps:.1} FPS | ETA: {:02}:{:02}", eta / 60, eta % 60),
                None => "--.- FPS | ETA: --:--".to_string(),
            };
            ui.monospace(format!("Frame: {current} / {total} | {stats}"));

            ui.add_space(15.0);

            if state
                .render_state
                .progress
                .is_complete
                .load(Ordering::Relaxed)
            {
                state.render_state.is_rendering = false;
            }

            if ui.button("Cancel Render").clicked() {
                state
                    .render_state
                    .progress
                    .is_cancelled
                    .store(true, Ordering::Relaxed);
                state.render_state.is_rendering = false;
            }
        });
    }
}
