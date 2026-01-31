//! Common utility functions for video rendering
//!
//! This module contains shared helper functions used across the video rendering system.

/// Linear interpolation between two u8 values
#[inline]
pub fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 * (1.0 - t) + b as f32 * t) as u8
}

/// Darken a color by multiplying with a factor (0.0 - 1.0)
#[inline]
pub fn darken_color(c: (u8, u8, u8), factor: f32) -> (u8, u8, u8) {
    (
        (c.0 as f32 * factor) as u8,
        (c.1 as f32 * factor) as u8,
        (c.2 as f32 * factor) as u8,
    )
}

/// Lighten a color by multiplying with a factor (> 1.0), clamped to 255
#[inline]
pub fn lighten_color(c: (u8, u8, u8), factor: f32) -> (u8, u8, u8) {
    (
        (c.0 as f32 * factor).min(255.0) as u8,
        (c.1 as f32 * factor).min(255.0) as u8,
        (c.2 as f32 * factor).min(255.0) as u8,
    )
}

/// Pack RGBA components into a single u32 (Little Endian: BGRA handling)
/// Note: Input color is (R, G, B). Alpha is separate.
/// Output format: 0xAARRGGBB (matches B8G8R8A8_UNORM)
#[inline(always)]
pub fn pack_color(c: (u8, u8, u8), a: u8) -> u32 {
    ((a as u32) << 24) | ((c.0 as u32) << 16) | ((c.1 as u32) << 8) | (c.2 as u32)
}

/// Fill a row with a packed color u32 (Unsafe, no bounds check)
/// Much faster than setting bytes individually.
#[inline]
pub unsafe fn fill_row_unchecked(buffer: &mut [u8], width: u32, x_start: i32, x_end: i32, y: i32, color_packed: u32) {
    if x_start >= x_end { return; }
    // Calculate byte offset. 4 bytes per pixel.
    let offset = ((y as u32 * width + x_start as u32) as usize) * 4;
    let count = (x_end - x_start) as usize;
    
    // Safety: Caller ensures buffer bounds. 
    // Pointer cast to *mut u32 is generally safe for Vec<u8> buffer due to alignment,
    // and x/y/width being integers means offset is multiple of 4.
    let ptr = buffer.as_mut_ptr().add(offset) as *mut u32;
    
    // Simple loop is usually auto-vectorized by LLVM (memset/stosd equivalent)
    for i in 0..count {
        *ptr.add(i) = color_packed;
    }
}

/// Draw a solid rectangle with alpha blending (optimized row-wise)
/// Color is (R, G, B), alpha 255 = opaque
pub fn draw_solid_rect_alpha(
    buffer: &mut [u8], width: u32, height: u32,
    left: i32, right: i32, top: i32, bottom: i32,
    color: (u8, u8, u8), alpha: u8,
) {
    // Clamp bounds
    let x_start = left.max(0) as usize;
    let x_end = right.min(width as i32) as usize;
    let y_start = top.max(0) as usize;
    let y_end = bottom.min(height as i32) as usize;
    
    if x_start >= x_end || y_start >= y_end {
        return;
    }
    
    let stride = width as usize * 4;
    
    if alpha == 255 {
        // Fast path: no blending, direct write
        for y in y_start..y_end {
            let row_offset = y * stride;
            for x in x_start..x_end {
                let idx = row_offset + x * 4;
                buffer[idx] = color.2;     // B
                buffer[idx + 1] = color.1; // G
                buffer[idx + 2] = color.0; // R
                buffer[idx + 3] = 255;     // A
            }
        }
    } else {
        // Alpha blending
        let a = alpha as f32 / 255.0;
        let inv_a = 1.0 - a;
        let src_b = color.2 as f32 * a;
        let src_g = color.1 as f32 * a;
        let src_r = color.0 as f32 * a;
        
        for y in y_start..y_end {
            let row_offset = y * stride;
            for x in x_start..x_end {
                let idx = row_offset + x * 4;
                buffer[idx] = (buffer[idx] as f32 * inv_a + src_b) as u8;
                buffer[idx + 1] = (buffer[idx + 1] as f32 * inv_a + src_g) as u8;
                buffer[idx + 2] = (buffer[idx + 2] as f32 * inv_a + src_r) as u8;
            }
        }
    }
}

/// Draw a solid rectangle (no alpha blending, fully opaque)
#[inline]
pub fn draw_solid_rect(
    buffer: &mut [u8], width: u32, height: u32,
    left: i32, right: i32, top: i32, bottom: i32,
    color: (u8, u8, u8),
) {
    draw_solid_rect_alpha(buffer, width, height, left, right, top, bottom, color, 255);
}

/// Draw a vertical gradient rectangle (optimized row-wise)
pub fn draw_gradient_rect(
    buffer: &mut [u8], width: u32, height: u32,
    left: i32, right: i32, top: i32, bottom: i32,
    top_color: (u8, u8, u8), bottom_color: (u8, u8, u8),
) {
    if top >= bottom || left >= right {
        return;
    }
    
    // Clamp bounds
    let x_start = left.max(0) as usize;
    let x_end = right.min(width as i32) as usize;
    let y_start = top.max(0) as i32;
    let y_end = bottom.min(height as i32) as i32;
    
    if x_start >= x_end {
        return;
    }
    
    let rect_height = (bottom - top) as f32;
    
    for y in y_start..y_end {
        let t = (y - top) as f32 / rect_height;
        let r = lerp_u8(top_color.0, bottom_color.0, t);
        let g = lerp_u8(top_color.1, bottom_color.1, t);
        let b = lerp_u8(top_color.2, bottom_color.2, t);
        
        // Check if we can use fast path? 
        // Gradient rect changes color per row, but CONSTANT per row.
        // So we can use fill_row_unchecked!
        let color_packed = pack_color((r, g, b), 255);
        
        unsafe {
             fill_row_unchecked(buffer, width, x_start as i32, x_end as i32, y, color_packed);
        }
    }
}

/// Calculate border width for keyboard keys
pub fn calculate_border_width(width_pixels: f32, keys_len: f32) -> f32 {
    let scale = (width_pixels / 1280.0).max(1.0);
    ((width_pixels / keys_len) / 12.0).clamp(1.0 * scale, 5.0 * scale).round() * 2.0
}
