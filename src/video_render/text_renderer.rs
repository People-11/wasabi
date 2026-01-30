//! TrueType text renderer (Software)
//!
//! Uses rusttype to render TTF fonts to a pixel buffer.

use rusttype::{Font, Scale, Point};
use std::sync::OnceLock;

static FONT: OnceLock<Font<'static>> = OnceLock::new();

pub fn get_font() -> &'static Font<'static> {
    FONT.get_or_init(|| {
        let font_data = include_bytes!("../../assets/UbuntuSansMono-Medium.ttf");
        Font::try_from_bytes(font_data as &[u8]).expect("Error constructing Font")
    })
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 * (1.0 - t) + b as f32 * t) as u8
}

/// Measure text width in pixels using TTF
pub fn measure_text_width_ttf(text: &str, size: f32) -> i32 {
    let font = get_font();
    let scale = Scale::uniform(size);
    let v_metrics = font.v_metrics(scale);
    let offset = Point { x: 0.0, y: v_metrics.ascent };
    
    let glyphs: Vec<_> = font.layout(text, scale, offset).collect();
    if let Some(last) = glyphs.last() {
        if let Some(bb) = last.pixel_bounding_box() {
             return bb.max.x;
        }
    }
    0
}

/// Draw text string using TTF font
pub fn draw_text_ttf(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4], // BGRA
    size: f32,
) -> i32 {
    let font = get_font();
    let scale = Scale::uniform(size);
    let v_metrics = font.v_metrics(scale);
    let offset = Point { x: x as f32, y: y as f32 + v_metrics.ascent };
    
    let glyphs: Vec<_> = font.layout(text, scale, offset).collect();
    
    for glyph in glyphs {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            glyph.draw(|gx, gy, v| {
                let px = bounding_box.min.x + gx as i32;
                let py = bounding_box.min.y + gy as i32;
                
                if px >= 0 && px < width as i32 && py >= 0 && py < height as i32 {
                    let idx = ((py as u32 * width + px as u32) * 4) as usize;
                    if idx + 3 < buffer.len() {
                        let alpha = (v * 255.0 * (color[3] as f32 / 255.0)) as u8;
                        if alpha > 0 {
                            let bg_b = buffer[idx];
                            let bg_g = buffer[idx + 1];
                            let bg_r = buffer[idx + 2];
                            
                            // Alpha blend
                            // color is BGRA based on usage
                            let a = alpha as f32 / 255.0;
                            
                            buffer[idx] = lerp_u8(bg_b, color[0], a);     // B
                            buffer[idx+1] = lerp_u8(bg_g, color[1], a);   // G
                            buffer[idx+2] = lerp_u8(bg_r, color[2], a);   // R
                        }
                    }
                }
            });
        }
    }
    // Return approximate end x
    measure_text_width_ttf(text, size) + x
}
