//! Software keyboard renderer for video export
//!
//! This module provides a CPU-based keyboard renderer that draws directly to a pixel buffer.
//! It replicates the exact visual style of the original egui-based keyboard renderer.

use super::utils::{
    calculate_border_width, darken_color, draw_gradient_rect, draw_solid_rect, lerp_u8,
    lighten_color,
};
use crate::gui::window::keyboard_layout::{KeyPosition, KeyboardView};
use crate::midi::MIDIColor;

/// Render the static part of the keyboard (background + all keys in unpressed state)
pub fn render_static_keyboard(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    keyboard_height: u32,
    key_view: &KeyboardView,
    bar_color: [u8; 4], // BGRA
) {
    let rect_top = height - keyboard_height;
    let rect_bottom = height;
    let rect_height = keyboard_height as f32;
    let rect_width = width as f32;

    let note_border = calculate_border_width(rect_width, key_view.visible_range.len() as f32);
    let key_border = (note_border / 2.0).round();

    let md_height = rect_height * 0.048;
    let bar = rect_height * 0.06;

    let black_key_overlap = bar / 2.35;
    let top = rect_top as f32 + bar;
    let bottom = rect_bottom as f32;

    // Helper to map x coordinate
    let map_x = |num: f32| (num * rect_width) as i32;

    // Draw white keys (unpressed)
    for (_, key) in key_view.iter_visible_keys() {
        if !key.black {
            draw_white_key_exact(
                buffer,
                width,
                height,
                &key,
                None, // Force unpressed
                top,
                bottom,
                black_key_overlap,
                md_height,
                key_border,
                &map_x,
            );
        }
    }

    // Draw black keys (unpressed) - NOW included in static buffer
    let black_bottom = rect_bottom as f32 - rect_height * 0.34;
    for (_, key) in key_view.iter_visible_keys() {
        if key.black {
            draw_black_key_exact(
                buffer,
                width,
                height,
                &key,
                None, // Unpressed color
                top,
                black_bottom,
                black_key_overlap,
                md_height,
                key_border,
                &map_x,
                None, // No bar fix needed for unpressed keys
            );
        }
    }

    // Draw coloured bar
    draw_bar_exact(buffer, width, height, top, black_key_overlap, bar_color);

    // Draw progress bar (gray bar at very top)
    draw_progress_bar(
        buffer,
        width,
        height,
        rect_top as f32,
        top - black_key_overlap,
    );
}

pub fn render_pressed_keys(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    keyboard_height: u32,
    key_view: &KeyboardView,
    key_colors: &[Option<MIDIColor>],
    dirty_black_keys: &std::collections::HashSet<usize>,
    bar_color: [u8; 4], // BGRA, needed to fix black key gap
) {
    let rect_top = height - keyboard_height;
    let rect_bottom = height;
    let rect_height = keyboard_height as f32;
    let rect_width = width as f32;

    let note_border = calculate_border_width(rect_width, key_view.visible_range.len() as f32);
    let key_border = (note_border / 2.0).round();

    let md_height = rect_height * 0.048;
    let bar = rect_height * 0.06;

    let black_key_overlap = bar / 2.35;
    let top = rect_top as f32 + bar;
    let bottom = rect_bottom as f32;
    let black_bottom = rect_bottom as f32 - rect_height * 0.34;

    // Helper to map x coordinate
    let map_x = |num: f32| (num * rect_width) as i32;

    // Draw PRESSED white keys only
    for (i, key) in key_view.iter_visible_keys() {
        if !key.black {
            if let Some(color) = key_colors.get(i).and_then(|c| *c) {
                draw_white_key_exact(
                    buffer,
                    width,
                    height,
                    &key,
                    Some(color),
                    top,
                    bottom,
                    black_key_overlap,
                    md_height,
                    key_border,
                    &map_x,
                );
            }
        }
    }

    // Draw DIRTY black keys (Pressed OR Neighbors of pressed white keys)
    for (i, key) in key_view.iter_visible_keys() {
        if key.black {
            if dirty_black_keys.contains(&i) {
                let color = key_colors.get(i).and_then(|c| *c);
                let bar_fix = if color.is_some() {
                    Some(bar_color)
                } else {
                    None
                };

                draw_black_key_exact(
                    buffer,
                    width,
                    height,
                    &key,
                    color,
                    top,
                    black_bottom,
                    black_key_overlap,
                    md_height,
                    key_border,
                    &map_x,
                    bar_fix,
                );
            }
        }
    }
}

/// Draw white key with exact original styling
fn draw_white_key_exact<F: Fn(f32) -> i32>(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    key: &KeyPosition,
    color: Option<MIDIColor>,
    top: f32,
    bottom: f32,
    black_key_overlap: f32,
    md_height: f32,
    key_border: f32,
    map_x: &F,
) {
    let left = map_x(key.left);
    let right = map_x(key.right);
    let top_i = top as i32;
    let bottom_i = bottom as i32;
    let overlap_y = (top + black_key_overlap) as i32;

    if let Some(c) = color {
        // Pressed white key
        let base = (c.red(), c.green(), c.blue());
        let darkened = darken_color(base, 0.6);
        let darkened2 = darken_color(base, 0.3);

        // Top section: darkened2 -> darkened
        draw_gradient_rect(
            buffer, width, height, left, right, top_i, overlap_y, darkened2, darkened,
        );

        // Middle section: darkened -> base
        draw_gradient_rect(
            buffer, width, height, left, right, overlap_y, bottom_i, darkened, base,
        );

        // Bottom highlight strip
        let strip_top = (bottom - key_border * 2.0) as i32;
        draw_gradient_rect(
            buffer, width, height, left, right, strip_top, bottom_i, darkened2, darkened,
        );
    } else {
        // Not pressed white key
        // Top section: gray(110) -> gray(210)
        draw_gradient_rect(
            buffer,
            width,
            height,
            left,
            right,
            top_i,
            overlap_y,
            (110, 110, 110),
            (210, 210, 210),
        );

        // Middle section: gray(210) -> white
        let md_y = (bottom - md_height) as i32;
        draw_gradient_rect(
            buffer,
            width,
            height,
            left,
            right,
            overlap_y,
            md_y,
            (210, 210, 210),
            (255, 255, 255),
        );

        // Bottom section: gray(190) -> gray(120)
        draw_gradient_rect(
            buffer,
            width,
            height,
            left,
            right,
            md_y,
            bottom_i,
            (190, 190, 190),
            (120, 120, 120),
        );

        // Bottom shadow strip: gray(70) -> gray(140)
        let strip_bottom = (bottom - md_height + key_border * 2.0) as i32;
        draw_gradient_rect(
            buffer,
            width,
            height,
            left,
            right,
            md_y,
            strip_bottom,
            (70, 70, 70),
            (140, 140, 140),
        );
    }

    // White key right border
    let border_left = right - key_border as i32;
    draw_solid_rect(
        buffer,
        width,
        height,
        border_left,
        right,
        top_i,
        bottom_i,
        (40, 40, 40),
    );
}

/// Draw coloured bar with gradient
fn draw_bar_exact(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    top: f32,
    black_key_overlap: f32,
    bar_color: [u8; 4], // BGRA
) {
    let bar_top = (top - black_key_overlap) as i32;
    let bar_bottom = top as i32;

    // bar_color is BGRA, convert to RGB tuple
    let bar = (bar_color[2], bar_color[1], bar_color[0]);
    let dark = darken_color(bar, 0.3);

    draw_gradient_rect(
        buffer,
        width,
        height,
        0,
        width as i32,
        bar_top,
        bar_bottom,
        dark,
        bar,
    );
}

/// Draw progress bar (gray gradient at top)
fn draw_progress_bar(buffer: &mut [u8], width: u32, height: u32, rect_top: f32, bar_top: f32) {
    let top = rect_top as i32;
    let bottom = bar_top as i32;

    draw_gradient_rect(
        buffer,
        width,
        height,
        0,
        width as i32,
        top,
        bottom,
        (90, 90, 90),
        (40, 40, 40),
    );
}

/// Draw black key with exact original styling (complex 3D effect with bevels)
fn draw_black_key_exact<F: Fn(f32) -> i32>(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    key: &KeyPosition,
    color: Option<MIDIColor>,
    top: f32,
    black_bottom: f32,
    black_key_overlap: f32,
    md_height: f32,
    key_border: f32,
    map_x: &F,
    bar_color_fix: Option<[u8; 4]>,
) {
    let left = map_x(key.left);
    let right = map_x(key.right);
    let black_bottom_i = black_bottom as i32;

    // Key horizontal dimensions (constant)
    let inner_left = left + (key_border * 2.0) as i32;
    let inner_right = right - (key_border * 2.0) as i32;

    if let Some(c) = color {
        // Pressed black key: Bevels are smaller (key is sunk)
        let bk_md_height = md_height / 2.0;
        let bk_overlap = black_key_overlap / 2.2;

        let inner_top = (top - bk_overlap) as i32;
        let inner_bottom = (black_bottom - bk_md_height) as i32;

        // FIX: Rewrite the gap above the pressed key with the bar gradient
        if let Some(bar_c) = bar_color_fix {
            let bar_top_f = top - black_key_overlap;
            let gap_bottom_f = top - bk_overlap;

            let bar_top_i = bar_top_f as i32;
            let gap_bottom_i = gap_bottom_f as i32;

            // Bar gradient logic from draw_bar_exact
            let bar_rgb = (bar_c[2], bar_c[1], bar_c[0]);
            let start_color = darken_color(bar_rgb, 0.3);
            let end_color = bar_rgb; // Gradient goes from dark(top) to normal(bottom)

            // We need to interpolate end_color because we stop early at gap_bottom
            let total_h = black_key_overlap;
            let current_h = gap_bottom_f - bar_top_f;
            let t = current_h / total_h;

            let gap_end_color = (
                lerp_u8(start_color.0, end_color.0, t),
                lerp_u8(start_color.1, end_color.1, t),
                lerp_u8(start_color.2, end_color.2, t),
            );

            draw_gradient_rect(
                buffer,
                width,
                height,
                left,
                right, // Fill full width of key slot
                bar_top_i,
                gap_bottom_i,
                start_color,
                gap_end_color,
            );
        }

        let base = (c.red(), c.green(), c.blue());
        let darkened = darken_color(base, 0.76);
        let lightened = lighten_color(base, 1.3);

        // Bottom bevel: base -> darkened (with inset)
        draw_trapezoid_vertical_gradient(
            buffer,
            width,
            height,
            inner_bottom,
            black_bottom_i,
            (left + key_border as i32) as f32,
            (right - key_border as i32) as f32,
            left as f32,
            right as f32,
            base,
            darkened,
        );

        // Left side bevel: lightened -> darkened (Horizontal)
        draw_slanted_vertical_gradient_strip(
            buffer,
            width,
            height,
            left,
            inner_left,
            top,
            inner_top as f32,
            black_bottom,
            inner_bottom as f32,
            lightened,
            darkened,
        );

        // Right side bevel: lightened -> darkened (Horizontal)
        draw_slanted_vertical_gradient_strip(
            buffer,
            width,
            height,
            inner_right,
            right,
            inner_top as f32,
            top,
            inner_bottom as f32,
            black_bottom,
            lightened,
            darkened,
        );

        // Top surface (main body): base -> darkened
        draw_gradient_rect(
            buffer,
            width,
            height,
            inner_left,
            inner_right,
            inner_top,
            inner_bottom,
            base,
            darkened,
        );
    } else {
        // Not pressed black key: Full bevel height (key is raised)
        let inner_top = (top - black_key_overlap) as i32;
        let inner_bottom = (black_bottom - md_height) as i32;

        // Bottom bevel: gray(105) -> gray(20)
        draw_trapezoid_vertical_gradient(
            buffer,
            width,
            height,
            inner_bottom,
            black_bottom_i,
            (left + key_border as i32) as f32,
            (right - key_border as i32) as f32,
            left as f32,
            right as f32,
            (105, 105, 105),
            (20, 20, 20),
        );

        // Left side bevel: dark edge -> light inner
        draw_slanted_vertical_gradient_strip(
            buffer,
            width,
            height,
            left,
            inner_left,
            top,
            inner_top as f32,
            black_bottom,
            inner_bottom as f32,
            (20, 20, 20),
            (105, 105, 105),
        );

        // Right side bevel: light inner -> dark edge
        draw_slanted_vertical_gradient_strip(
            buffer,
            width,
            height,
            inner_right,
            right,
            inner_top as f32,
            top,
            inner_bottom as f32,
            black_bottom,
            (105, 105, 105),
            (20, 20, 20),
        );

        // Top surface (main body): gray(20) -> gray(40)
        draw_gradient_rect(
            buffer,
            width,
            height,
            inner_left,
            inner_right,
            inner_top,
            inner_bottom,
            (20, 20, 20),
            (40, 40, 40),
        );
    }
}

/// Draw a vertical strip where top and bottom Y coordinates are interpolated based on X
fn draw_slanted_vertical_gradient_strip(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    x_start: i32,
    x_end: i32,
    y_top_start: f32,
    y_top_end: f32,
    y_bottom_start: f32,
    y_bottom_end: f32,
    color_start: (u8, u8, u8),
    color_end: (u8, u8, u8),
) {
    if x_start >= x_end {
        return;
    }

    let total_w = (x_end - x_start) as f32;

    // Clamp X range
    let x_min = x_start.max(0);
    let x_max = x_end.min(width as i32);

    for x in x_min..x_max {
        // Calculate t for the left edge of the pixel column
        let t0 = (x - x_start) as f32 / total_w;
        // Calculate t for the right edge of the pixel column (for conservative coverage)
        let t1 = (x + 1 - x_start) as f32 / total_w;

        let y_top_0 = (y_top_start * (1.0 - t0) + y_top_end * t0) as i32;
        let y_top_1 = (y_top_start * (1.0 - t1) + y_top_end * t1) as i32;
        // Use the minimum top Y (highest point) to cover the full pixel
        let y_top = y_top_0.min(y_top_1);

        let y_bottom_0 = (y_bottom_start * (1.0 - t0) + y_bottom_end * t0) as i32;
        let y_bottom_1 = (y_bottom_start * (1.0 - t1) + y_bottom_end * t1) as i32;
        // Use the maximum bottom Y (lowest point) to cover the full pixel
        let y_bottom = y_bottom_0.max(y_bottom_1);

        if y_top >= y_bottom {
            continue;
        }

        // Use color at the center of the pixel for smoothness
        let t_center = (x as f32 + 0.5 - x_start as f32) / total_w;
        let r = lerp_u8(color_start.0, color_end.0, t_center);
        let g = lerp_u8(color_start.1, color_end.1, t_center);
        let b = lerp_u8(color_start.2, color_end.2, t_center);

        // Clamp Y range
        let y_min = y_top.max(0);
        let y_max = y_bottom.min(height as i32);

        let color_packed = super::utils::pack_color((r, g, b), 255);

        // Vertical fill optimization
        unsafe {
            let mut offset = ((y_min as u32 * width + x as u32) as usize) * 4;
            let stride_bytes = (width as usize) * 4;

            for _ in y_min..y_max {
                // ptr_base is a byte pointer cast to u32 pointer?? NO.
                // ptr_base is u32 pointer. arithmetic is in u32 units.
                // offset is in BYTES.
                // We need distinct handling.

                // Let's keep it simple:
                let ptr = buffer.as_mut_ptr().add(offset) as *mut u32;
                *ptr = color_packed;
                offset += stride_bytes;
            }
        }
    }
}

/// Draw a trapezoid with vertical gradient
fn draw_trapezoid_vertical_gradient(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    top_y: i32,
    bottom_y: i32,
    top_left_x: f32,
    top_right_x: f32,
    bottom_left_x: f32,
    bottom_right_x: f32,
    top_color: (u8, u8, u8),
    bottom_color: (u8, u8, u8),
) {
    let total_h = (bottom_y - top_y) as f32;

    // Clamp Y range
    let y_min = top_y.max(0);
    let y_max = bottom_y.min(height as i32);

    for y in y_min..y_max {
        let t = (y - top_y) as f32 / total_h;
        let color = (
            lerp_u8(top_color.0, bottom_color.0, t),
            lerp_u8(top_color.1, bottom_color.1, t),
            lerp_u8(top_color.2, bottom_color.2, t),
        );

        // Use floor/ceil to be slightly generous with horizontal coverage
        let start_x = (top_left_x * (1.0 - t) + bottom_left_x * t).floor() as i32;
        let end_x = (top_right_x * (1.0 - t) + bottom_right_x * t).ceil() as i32;

        // Clamp X range
        let x_min = start_x.max(0);
        let x_max = end_x.min(width as i32);

        let color_packed = super::utils::pack_color(color, 255);

        unsafe {
            super::utils::fill_row_unchecked(buffer, width, x_min, x_max, y, color_packed);
        }
    }
}
