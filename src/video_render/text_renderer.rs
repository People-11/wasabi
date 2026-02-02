//! TrueType text renderer (Software)
//!
//! Uses rusttype to render TTF fonts to a pixel buffer.

use super::utils::lerp_u8;
use rusttype::{Font, Point, Scale};
use std::sync::OnceLock;

static FONT: OnceLock<Font<'static>> = OnceLock::new();

pub fn get_font() -> &'static Font<'static> {
    FONT.get_or_init(|| {
        let font_data = include_bytes!("../../assets/UbuntuSansMono-Medium.ttf");
        Font::try_from_bytes(font_data as &[u8]).expect("Error constructing Font")
    })
}

/// Measure text width in pixels using TTF
pub fn measure_text_width_ttf(text: &str, size: f32) -> i32 {
    let font = get_font();
    let scale = Scale::uniform(size);
    let v_metrics = font.v_metrics(scale);
    let offset = Point {
        x: 0.0,
        y: v_metrics.ascent,
    };

    // Optimization: Don't collect into Vec, just get the last glyph directly
    if let Some(last) = font.layout(text, scale, offset).last() {
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
    let offset = Point {
        x: x as f32,
        y: y as f32 + v_metrics.ascent,
    };

    let glyphs: Vec<_> = font.layout(text, scale, offset).collect();

    // Get text width - Retrieve from existing glyphs to avoid repeated layout
    let end_x = glyphs
        .last()
        .and_then(|g| g.pixel_bounding_box())
        .map(|bb| bb.max.x)
        .unwrap_or(x);

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
                            let a = alpha as f32 / 255.0;
                            buffer[idx] = lerp_u8(buffer[idx], color[0], a);
                            buffer[idx + 1] = lerp_u8(buffer[idx + 1], color[1], a);
                            buffer[idx + 2] = lerp_u8(buffer[idx + 2], color[2], a);
                        }
                    }
                }
            });
        }
    }
    
    end_x
}
