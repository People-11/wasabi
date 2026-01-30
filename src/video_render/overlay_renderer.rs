use crate::gui::window::stats::GuiMidiStats;
use crate::midi::MIDIFile;
use crate::settings::{Statistics, WasabiSettings};
use crate::utils::convert_seconds_to_time_string;
use super::text_renderer;

/// Draw the overlay statistics
pub fn draw_overlay(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    midi_file: &mut impl MIDIFile,
    current_time: f64,
    stats: &GuiMidiStats, // We use this for rendered notes count / polyphony passed from renderer
    nps: u64,
    settings: &WasabiSettings,
) {
    // There is no visible flag in StatisticsSettings, we assume visible if opacity > 0
    // println!("[Overlay] Opacity: {}, Lines to draw setting: {:?}", settings.scene.statistics.opacity, settings.scene.statistics.order);
    
    // We render even if opacity is 0, because opacity controls background transparency,
    // while text usually remains visible (or user wants transparent background).
    
    // Calculate window position and size
    let opacity = settings.scene.statistics.opacity.clamp(0.0, 1.0);
    let alpha = (255.0 * opacity).round() as u8;
    
    // Stats frame style
    let pad = if settings.scene.statistics.floating { 12 } else { 0 };
    let panel_height = 0; // No playback panel in video render usually
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
            Statistics::Fps => {
                // Skip FPS in video
            }
            Statistics::VoiceCount => {
                // Skip Voice Count in video
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
    let font_size = 24.0;
    let line_height = font_size as i32 + 4; // Add some leading
    let spacing = 8;
    
    // Measure max width
    let mut max_text_width = 0;
    for line in &lines {
        let w = text_renderer::measure_text_width_ttf(line, font_size);
        if w > max_text_width {
            max_text_width = w;
        }
    }
    
    // Ensure width is enough for 25 digits (e.g. "00000000000000000000000000000")
    // Using a sample string of 25 '0's to calculate minimum content width.
    let min_digits_width = text_renderer::measure_text_width_ttf("00000000000000000000000000000", font_size);
    
    let window_width = (max_text_width.max(min_digits_width) + 32).max(200) as u32; // Min 200 or 25 digits, padding 16*2
    let content_height = lines.len() as i32 * (line_height + spacing) - spacing + 2 * spacing; // padding
    let window_height = content_height;
    
    // Draw background
    let bg_color = (0, 0, 0); // Black
    draw_rounded_rect(
        buffer, width, height, 
        x, y, window_width, window_height as u32, 
        bg_color, alpha, 8
    );
    
    // Draw border if enabled
    if settings.scene.statistics.border {
        draw_rounded_rect_border(
            buffer, width, height,
            x, y, window_width, window_height as u32,
            (50, 50, 50), 255, 8
        );
    }
    
    // Render text
    let mut text_y = y + spacing;
    for line in lines {
        // Simple layout: "Label: Value"
        let text_color = [255, 255, 255, 255]; // White
        
        if let Some(idx) = line.find(':') {
            let label = &line[0..=idx];
            let value = &line[idx+1..];
            
            text_renderer::draw_text_ttf(
                buffer, width, height, 
                x + 16, text_y, 
                label, text_color, font_size
            );
            
            let value_width = text_renderer::measure_text_width_ttf(value, font_size);
            text_renderer::draw_text_ttf(
                buffer, width, height,
                x + window_width as i32 - 16 - value_width, text_y,
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
                    x + 16, text_y, 
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
    
    // Bounds check
    let min_x = x.max(0);
    let max_x = right.min(width as i32);
    let min_y = y.max(0);
    let max_y = bottom.min(height as i32);

    for py in min_y..max_y {
        for px in min_x..max_x {
            // Check rounded corners
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
                    let bg_b = buffer[idx];
                    let bg_g = buffer[idx + 1];
                    let bg_r = buffer[idx + 2];
                    
                    let a = alpha as f32 / 255.0;
                    
                    buffer[idx] = lerp_u8(bg_b, color.2, a);
                    buffer[idx + 1] = lerp_u8(bg_g, color.1, a);
                    buffer[idx + 2] = lerp_u8(bg_r, color.0, a);
                }
            }
        }
    }
}

fn draw_rounded_rect_border(
    buffer: &mut [u8], width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32,
    color: (u8, u8, u8), alpha: u8, radius: i32,
) {
    // Simple implementation: draw larger rect then clear inner? No, expensive.
    // Just draw stroke.
    // Simplifying: just draw 1px outline for now skipping complex rounded corner stroke math
    // Top
    draw_solid_rect(buffer, width, height, x + radius, x + w as i32 - radius, y, y + 1, color, alpha);
    // Bottom
    draw_solid_rect(buffer, width, height, x + radius, x + w as i32 - radius, y + h as i32 - 1, y + h as i32, color, alpha);
    // Left
    draw_solid_rect(buffer, width, height, x, x + 1, y + radius, y + h as i32 - radius, color, alpha);
    // Right
    draw_solid_rect(buffer, width, height, x + w as i32 - 1, x + w as i32, y + radius, y + h as i32 - radius, color, alpha);
    
    // Corners (pixels) - naive
}

fn draw_solid_rect(
    buffer: &mut [u8], width: u32, height: u32,
    left: i32, right: i32, top: i32, bottom: i32,
    color: (u8, u8, u8), alpha: u8,
) {
    for py in top.max(0)..bottom.min(height as i32) {
        for px in left.max(0)..right.min(width as i32) {
             let idx = ((py as u32 * width + px as u32) * 4) as usize;
             if idx + 3 < buffer.len() {
                 let bg_b = buffer[idx];
                 let bg_g = buffer[idx + 1];
                 let bg_r = buffer[idx + 2];
                 
                 let a = alpha as f32 / 255.0;
                 
                 buffer[idx] = lerp_u8(bg_b, color.2, a);
                 buffer[idx + 1] = lerp_u8(bg_g, color.1, a);
                 buffer[idx + 2] = lerp_u8(bg_r, color.0, a);
             }
        }
    }
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 * (1.0 - t) + b as f32 * t) as u8
}
