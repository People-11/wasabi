use crate::gui::window::stats::GuiMidiStats;
use crate::midi::MIDIFile;
use crate::settings::{Statistics, WasabiSettings};
use crate::utils::convert_seconds_to_time_string;
use super::text_renderer;
use super::utils::{lerp_u8, draw_solid_rect_alpha};

/// Draw the overlay statistics
pub fn draw_overlay(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    midi_file: &mut impl MIDIFile,
    current_time: f64,
    stats: &GuiMidiStats,
    nps: u64,
    settings: &WasabiSettings,
) {
    let opacity = settings.scene.statistics.opacity.clamp(0.0, 1.0);
    let alpha = (255.0 * opacity).round() as u8;
    
    // Scale based on 720p reference height
    let scale = (height as f32 / 720.0).max(1.0);
    // Stats frame style
    let pad = if settings.scene.statistics.floating { (12.0 * scale) as i32 } else { 0 };
    let panel_height = 0;
    let x = pad;
    let y = panel_height + pad;
    
    // Collect lines to render
    let mut lines = Vec::new();
    
    let midi_len = midi_file.midi_length().unwrap_or(0.0);
    let time_passed = current_time.min(midi_len).max(0.0);
    let note_stats = midi_file.stats();
    
    // Filter order
    for (stat_type, enabled) in settings.scene.statistics.order.iter() {
        if !enabled { continue; }
        
        match stat_type {
            Statistics::Time => {
                lines.push(format!(
                    "Time: {} / {}",
                    convert_seconds_to_time_string(time_passed),
                    convert_seconds_to_time_string(midi_len)
                ));
            }
            Statistics::Fps | Statistics::VoiceCount => {
                // Skip in video render
            }
            Statistics::Rendered => {
                lines.push(format!("Rendered: {}", numfmt_format(stats.notes_on_screen)));
            }
            Statistics::NoteCount => {
                let passed = note_stats.passed_notes.unwrap_or(0);
                let total = note_stats.total_notes.unwrap_or(0);
                lines.push(format!("{} / {}", numfmt_format(passed), numfmt_format(total)));
            }
            Statistics::Nps => {
                lines.push(format!("NPS: {}", numfmt_format(nps)));
            }
            Statistics::Polyphony => {
                if let Some(poly) = stats.polyphony {
                    lines.push(format!("Polyphony: {}", numfmt_format(poly)));
                }
            }
        }
    }
    
    if lines.is_empty() {
        return;
    }
    
    // Calculate content height and width
    let font_size = 18.0 * scale;
    let line_height = font_size as i32 + (4.0 * scale) as i32;
    let spacing = (6.0 * scale) as i32;
    
    // Measure max width
    let mut max_text_width = 0;
    for line in &lines {
        let w = text_renderer::measure_text_width_ttf(line, font_size);
        if w > max_text_width {
            max_text_width = w;
        }
    }
    
    let min_digits_width = text_renderer::measure_text_width_ttf("00000000000000000000000000000", font_size);
    
    let window_width = (max_text_width.max(min_digits_width) + (32.0 * scale) as i32).max((200.0 * scale) as i32) as u32;
    let content_height = lines.len() as i32 * (line_height + spacing) - spacing + 2 * spacing;
    let window_height = content_height;
    
    let corner_radius = (8.0 * scale) as i32;
    // Draw background
    draw_rounded_rect(
        buffer, width, height, 
        x, y, window_width, window_height as u32, 
        (0, 0, 0), alpha, corner_radius
    );
    
    // Draw border if enabled
    if settings.scene.statistics.border {
        let thickness = (1.0 * scale).round() as i32;
        draw_rounded_rect_border(
            buffer, width, height,
            x, y, window_width, window_height as u32,
            (50, 50, 50), 255, corner_radius, thickness
        );
    }
    
    // Render text
    let mut text_y = y + spacing;
    let text_x_pad = (16.0 * scale) as i32;

    for line in lines {
        // Match egui's default text color (slight gray)
        let text_color = [210, 210, 210, 255];
        
        if let Some(idx) = line.find(':') {
            let label = &line[0..=idx];
            let value = &line[idx+1..];
            
            text_renderer::draw_text_ttf(
                buffer, width, height, 
                x + text_x_pad, text_y, 
                label, text_color, font_size
            );
            
            let value_width = text_renderer::measure_text_width_ttf(value, font_size);
            text_renderer::draw_text_ttf(
                buffer, width, height,
                x + window_width as i32 - text_x_pad - value_width, text_y,
                value, text_color, font_size
            );
        } else {
            // Center typical "X / Y" strings
            if line.contains('/') && !line.starts_with("Time") {
                 let text_width = text_renderer::measure_text_width_ttf(&line, font_size);
                 text_renderer::draw_text_ttf(
                    buffer, width, height,
                    x + (window_width as i32 - text_width) / 2, text_y,
                    &line, text_color, font_size
                );
            } else {
                text_renderer::draw_text_ttf(
                    buffer, width, height, 
                    x + text_x_pad, text_y, 
                    &line, text_color, font_size
                );
            }
        }
        
        text_y += line_height + spacing;
    }
}

// Helper for number formatting (thousands separator)
fn numfmt_format(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let mut count = 0;
    for c in s.chars().rev() {
        if count > 0 && count % 3 == 0 {
            result.push(',');
        }
        result.push(c);
        count += 1;
    }
    result.chars().rev().collect()
}

/// Draw a rounded rectangle with alpha blending
fn draw_rounded_rect(
    buffer: &mut [u8], width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32,
    color: (u8, u8, u8), alpha: u8, radius: i32,
) {
    let right = x + w as i32;
    let bottom = y + h as i32;
    let r2 = radius * radius;
    
    let min_x = x.max(0);
    let max_x = right.min(width as i32);
    let min_y = y.max(0);
    let max_y = bottom.min(height as i32);

    for py in min_y..max_y {
        for px in min_x..max_x {
            let mut inside = true;
            
            // Top-left
            if px < x + radius && py < y + radius {
                let dx = x + radius - px - 1;
                let dy = y + radius - py - 1;
                if dx*dx + dy*dy > r2 { inside = false; }
            }
            // Top-right
            else if px >= right - radius && py < y + radius {
                let dx = px - (right - radius);
                let dy = y + radius - py - 1;
                if dx*dx + dy*dy > r2 { inside = false; }
            }
            // Bottom-left
            else if px < x + radius && py >= bottom - radius {
                let dx = x + radius - px - 1;
                let dy = py - (bottom - radius);
                if dx*dx + dy*dy > r2 { inside = false; }
            }
            // Bottom-right
            else if px >= right - radius && py >= bottom - radius {
                let dx = px - (right - radius);
                let dy = py - (bottom - radius);
                if dx*dx + dy*dy > r2 { inside = false; }
            }
            
            if inside {
                let idx = ((py as u32 * width + px as u32) * 4) as usize;
                if idx + 3 < buffer.len() {
                    let a = alpha as f32 / 255.0;
                    buffer[idx] = lerp_u8(buffer[idx], color.2, a);
                    buffer[idx + 1] = lerp_u8(buffer[idx + 1], color.1, a);
                    buffer[idx + 2] = lerp_u8(buffer[idx + 2], color.0, a);
                }
            }
        }
    }
}

fn draw_rounded_rect_border(
    buffer: &mut [u8], width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32,
    color: (u8, u8, u8), alpha: u8, radius: i32, thickness: i32,
) {
    // Top
    draw_solid_rect_alpha(buffer, width, height, x + radius, x + w as i32 - radius, y, y + thickness, color, alpha);
    // Bottom
    draw_solid_rect_alpha(buffer, width, height, x + radius, x + w as i32 - radius, y + h as i32 - thickness, y + h as i32, color, alpha);
    // Left
    draw_solid_rect_alpha(buffer, width, height, x, x + thickness, y + radius, y + h as i32 - radius, color, alpha);
    // Right
    draw_solid_rect_alpha(buffer, width, height, x + w as i32 - thickness, x + w as i32, y + radius, y + h as i32 - radius, color, alpha);
    // Draw four corners
    let right = x + w as i32;
    let bottom = y + h as i32;
    let inner_radius = radius - thickness;
    let r2_outer = radius * radius;
    let r2_inner = inner_radius * inner_radius;

    let corners = [
        (x + radius, y + radius, -1, -1), // Top-left
        (right - radius, y + radius, 1, -1), // Top-right
        (x + radius, bottom - radius, -1, 1), // Bottom-left
        (right - radius, bottom - radius, 1, 1), // Bottom-right
    ];

    let a_f32 = alpha as f32 / 255.0;

    for (cx, cy, _, _) in corners {
        let min_px = (cx - radius).max(0);
        let max_px = (cx + radius).min(width as i32);
        let min_py = (cy - radius).max(0);
        let max_py = (cy + radius).min(height as i32);

        for py in min_py..max_py {
            for px in min_px..max_px {
                let dx = px - cx;
                let dy = py - cy;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= r2_outer && dist_sq > r2_inner {
                    // Only draw in the correct quadrant for each corner
                    let is_in_quadrant = if cx <= x + radius { px <= cx } else { px >= cx }
                                      && if cy <= y + radius { py <= cy } else { py >= cy };

                    if is_in_quadrant {
                        let idx = ((py as u32 * width + px as u32) * 4) as usize;
                        if idx + 3 < buffer.len() {
                            buffer[idx] = lerp_u8(buffer[idx], color.2, a_f32);
                            buffer[idx + 1] = lerp_u8(buffer[idx + 1], color.1, a_f32);
                            buffer[idx + 2] = lerp_u8(buffer[idx + 2], color.0, a_f32);
                        }
                    }
                }
            }
        }
    }
}
