use std::sync::Arc;

use egui::{ComboBox, ProgressBar};

use crate::video_render::{render_loop::start_render, RenderConfig};
use crate::{settings::WasabiSettings, state::WasabiState, utils};

use super::render_state::{RenderFrameRate, RenderResolution};
use super::GuiWasabiWindow;

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

        // Initialize FFmpeg path from settings if not set
        if state.render_state.ffmpeg_path.is_none() {
            if let Some(ref path) = settings.gui.ffmpeg_path {
                if path.exists() {
                    state.render_state.ffmpeg_path = Some(path.clone());
                }
            }
        }

        // Remove shadow and customize frame
        let mut frame = utils::create_window_frame(ctx);
        frame.shadow = egui::Shadow::NONE;

        // Slightly shorter since we are reducing padding
        let size = [500.0, 480.0];

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
                // Disable text selection in this window
                ui.style_mut().interaction.selectable_labels = false;
                ui.add_space(10.0);

                // Determine if interaction is allowed
                let is_rendering = state.render_state.is_rendering;

                ui.add_enabled_ui(!is_rendering, |ui| {
                    self.render_settings_ui(ui, settings, state);
                });

                ui.add_space(15.0);
                ui.separator();

                // Conditional spacing based on state to match desired layout
                // But user wants spacing inside render_progress_ui to be consistent

                if is_rendering {
                    ui.add_space(15.0);
                    self.render_progress_ui(ui, settings, state);
                } else {
                    ui.add_space(15.0);
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
                                .set_directory(
                                    last_location.parent().unwrap_or(std::path::Path::new("./")),
                                )
                                .pick_file()
                            {
                                // Set output path to same name but .mp4
                                let mut output = path.clone();
                                output.set_extension("mp4");
                                state.render_state.output_path = Some(output);
                                state.render_state.midi_path = Some(path);
                            }
                        }

                        let text = if let Some(path) = &state.render_state.midi_path {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            if name.len() > 30 {
                                format!("{}...", &name[..27])
                            } else {
                                name
                            }
                        } else {
                            "(None selected)".to_string()
                        };
                        ui.label(egui::RichText::new(text).strong());
                    });
                });
                ui.end_row();

                // FFmpeg
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
                                // Save to settings
                                settings.gui.ffmpeg_path = Some(path);
                                let _ = settings.save_to_file();
                            }
                        }
                        let text = if let Some(path) = &state.render_state.ffmpeg_path {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            if name.len() > 30 {
                                format!("{}...", &name[..27])
                            } else {
                                name
                            }
                        } else {
                            "(None selected)".to_string()
                        };
                        ui.label(egui::RichText::new(text).strong());
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

                            // Pre-fill directory and filename if available
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
                        let text = if let Some(path) = &state.render_state.output_path {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            if name.len() > 30 {
                                format!("{}...", &name[..27])
                            } else {
                                name
                            }
                        } else {
                            "(None selected)".to_string()
                        };
                        ui.label(egui::RichText::new(text).strong());
                    });
                });
                ui.end_row();
            });

        ui.add_space(15.0);

        ui.heading("Video Settings");
        ui.add_space(5.0);
        
        ui.horizontal(|ui| {
            // Resolution
            ui.label("Resolution:");
            ComboBox::from_id_salt("resolution_combo")
                .selected_text(state.render_state.resolution.label())
                .width(100.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.render_state.resolution,
                        RenderResolution::HD1080,
                        RenderResolution::HD1080.label(),
                    );
                    ui.selectable_value(
                        &mut state.render_state.resolution,
                        RenderResolution::UHD4K,
                        RenderResolution::UHD4K.label(),
                    );
                });

            ui.add_space(8.0);

            // Frame Rate
            ui.label("FPS:");
            ComboBox::from_id_salt("framerate_combo")
                .selected_text(state.render_state.frame_rate.label())
                .width(80.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.render_state.frame_rate,
                        RenderFrameRate::Fps30,
                        RenderFrameRate::Fps30.label(),
                    );
                    ui.selectable_value(
                        &mut state.render_state.frame_rate,
                        RenderFrameRate::Fps60,
                        RenderFrameRate::Fps60.label(),
                    );
                    ui.selectable_value(
                        &mut state.render_state.frame_rate,
                        RenderFrameRate::Fps120,
                        RenderFrameRate::Fps120.label(),
                    );
                });

            ui.add_space(8.0);

            // Quality
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

            // Centering buttons roughly
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    state.show_render = false;
                }

                if ui
                    .add_enabled(can_start, egui::Button::new("🎬 Start Render"))
                    .clicked()
                {
                    // Start render logic
                    let config = RenderConfig::new(
                        state.render_state.midi_path.clone().unwrap(),
                        state.render_state.ffmpeg_path.clone().unwrap(),
                        state.render_state.output_path.clone().unwrap(),
                        state.render_state.resolution,
                        state.render_state.frame_rate,
                        state.render_state.quality,
                        settings.clone(),
                    );

                    state.render_state.progress.reset();
                    state.render_state.is_rendering = true;

                    let progress = super::render_state::RenderProgress {
                        current_frame: Arc::clone(&state.render_state.progress.current_frame),
                        total_frames: Arc::clone(&state.render_state.progress.total_frames),
                        is_cancelled: Arc::clone(&state.render_state.progress.is_cancelled),
                        is_complete: Arc::clone(&state.render_state.progress.is_complete),
                        is_parsing: Arc::clone(&state.render_state.progress.is_parsing),
                        fps_history: Arc::clone(&state.render_state.progress.fps_history),
                    };

                    start_render(config, progress);
                }
            });
        });
    }

    fn render_progress_ui(
        &mut self,
        ui: &mut egui::Ui,
        _settings: &mut WasabiSettings,
        state: &mut WasabiState,
    ) {
        ui.vertical_centered(|ui| {
            // Request continuous repaints while rendering to update progress
            ui.ctx().request_repaint();
            // Heading
            if state
                .render_state
                .progress
                .is_parsing
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                ui.heading("Parsing MIDI Info...");
            } else {
                ui.heading("Rendering in Progress...");
            }

            // Consistent Spacing 1
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

            // Consistent Spacing 2
            ui.add_space(15.0);

            let current = state
                .render_state
                .progress
                .current_frame
                .load(std::sync::atomic::Ordering::Relaxed);
            let total = state
                .render_state
                .progress
                .total_frames
                .load(std::sync::atomic::Ordering::Relaxed);
            let mut info_text = format!("Frame: {} / {}", current, total);
            if let Some((fps, eta)) = state.render_state.progress.get_performance_stats() {
                info_text.push_str(&format!(
                    " | {:.1} FPS | ETA: {:02}:{:02}",
                    fps,
                    eta / 60,
                    eta % 60
                ));
            }
            ui.monospace(info_text);

            // Consistent Spacing 3
            ui.add_space(15.0);

            if state
                .render_state
                .progress
                .is_complete
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                state.render_state.is_rendering = false;
            }

            if ui.button("Cancel Render").clicked() {
                state
                    .render_state
                    .progress
                    .is_cancelled
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                state.render_state.is_rendering = false;
            }
        });
    }
}
