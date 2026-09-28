use egui::{emath::GuiRounding, Color32, Mesh, Pos2, Rect, Sense, Ui};

use crate::midi::MIDIColor;

use super::keyboard_layout::KeyboardView;

pub fn draw_keyboard(
    ui: &mut Ui,
    key_view: &KeyboardView,
    colors: &[Option<MIDIColor>],
    bar_color: &Color32,
) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let mut mesh = Mesh::default();
    let note_border =
        crate::utils::calculate_border_width(rect.width(), key_view.visible_range.len() as f32);
    let key_border = (note_border / 2.0).round_to_pixels(ui.painter().pixels_per_point());

    let md_height = rect.height() * 0.048;
    let bar = rect.height() * 0.06;

    let black_key_overlap = bar / 2.35;
    let top = rect.top() + bar;
    let bottom = rect.bottom();
    let black_bottom = rect.bottom() - rect.height() * 0.34;
    let map_x = |num: f32| rect.left() + num * rect.width();
    let map_color = |col: MIDIColor| Color32::from_rgb(col.red(), col.green(), col.blue());
    let gray = |v: u8| Color32::from_rgb(v, v, v);

    for (i, key) in key_view.iter_visible_keys() {
        if key.black {
            continue;
        }
        let (left, right) = (map_x(key.left), map_x(key.right));
        let mut gradient = |top, bottom, top_color, bottom_color| {
            vertical_gradient(&mut mesh, left, right, top, bottom, top_color, bottom_color)
        };

        if let Some(color) = colors[i].map(map_color) {
            // Pressed
            let darkened = shade(color, 0.6);
            let darkened2 = shade(color, 0.3);

            gradient(top, top + black_key_overlap, darkened2, darkened);
            gradient(top + black_key_overlap, bottom, darkened, color);
            gradient(bottom - key_border * 2.0, bottom, darkened2, darkened);
            // Invisible, so pressed and unpressed keys have the same vertex count. egui's
            // per-frame vertex upload then keeps a stable size; a size that changes every
            // frame makes vulkano's buffer range tracking fragment without bound (it never
            // re-merges split ranges), which slowed every frame down over playback.
            gradient(bottom, bottom, color, color);
        } else {
            // Not pressed
            let front = bottom - md_height;
            gradient(top, top + black_key_overlap, gray(110), gray(210));
            gradient(top + black_key_overlap, front, gray(210), Color32::WHITE);
            gradient(front, bottom, gray(190), gray(120));
            gradient(front, front + key_border * 2.0, gray(70), gray(140));
        }

        // White key borders
        let border = Rect::from_min_max(Pos2::new(right, top), Pos2::new(right - key_border, bottom));
        mesh.add_colored_rect(border, gray(40));
    }

    // Coloured bar
    let (left, right) = (rect.left(), rect.right());
    let bar_top = top - black_key_overlap;
    vertical_gradient(&mut mesh, left, right, bar_top, top, shade(*bar_color, 0.3), *bar_color);

    // Progress bar
    vertical_gradient(&mut mesh, left, right, rect.top(), bar_top, gray(90), gray(40));

    for (i, key) in key_view.iter_visible_keys() {
        if !key.black {
            continue;
        }
        let (left, right) = (map_x(key.left), map_x(key.right));

        // A black key is a top face with bevels on the left, right and front. Colors are
        // front (top, bottom), left bevel (outer, inner), right bevel & face (light, dark)
        // and face top/bottom. Pressed keys sink in, so their bevels get thinner.
        let (md_height, overlap, [front, front_dark, left_outer, left_inner, face_top, face_bottom]) =
            if let Some(color) = colors[i].map(map_color) {
                let darkened = shade(color, 0.76);
                let lightened = shade(color, 1.3);
                let colors = [color, darkened, lightened, darkened, color, darkened];
                (md_height / 2.0, black_key_overlap / 2.2, colors)
            } else {
                let colors = [gray(105), gray(20), gray(20), gray(105), gray(20), gray(40)];
                (md_height, black_key_overlap, colors)
            };

        let bevel = 2.0 * key_border;
        let face_y_top = top - overlap;
        let face_y_bottom = black_bottom - md_height;

        // Front
        quad(
            &mut mesh,
            [
                (Pos2::new(left + key_border, face_y_bottom), front),
                (Pos2::new(right - key_border, face_y_bottom), front),
                (Pos2::new(left, black_bottom), front_dark),
                (Pos2::new(right, black_bottom), front_dark),
            ],
        );
        // Left bevel
        quad(
            &mut mesh,
            [
                (Pos2::new(left, top), left_outer),
                (Pos2::new(left + bevel, face_y_top), left_inner),
                (Pos2::new(left, black_bottom), left_outer),
                (Pos2::new(left + bevel, face_y_bottom), left_inner),
            ],
        );
        // Right bevel
        quad(
            &mut mesh,
            [
                (Pos2::new(right - bevel, face_y_top), front),
                (Pos2::new(right, top), front_dark),
                (Pos2::new(right - bevel, face_y_bottom), front),
                (Pos2::new(right, black_bottom), front_dark),
            ],
        );
        // Top face
        quad(
            &mut mesh,
            [
                (Pos2::new(left + bevel, face_y_top), face_top),
                (Pos2::new(right - bevel, face_y_top), face_top),
                (Pos2::new(left + bevel, face_y_bottom), face_bottom),
                (Pos2::new(right - bevel, face_y_bottom), face_bottom),
            ],
        );
    }

    ui.painter().add(mesh);
}

/// Scales the RGB channels, keeping the color opaque. (`gamma_multiply` scales alpha too,
/// which made pressed keys translucent.)
fn shade(color: Color32, factor: f32) -> Color32 {
    let [r, g, b, _] = color.to_array().map(|c| (c as f32 * factor) as u8);
    Color32::from_rgb(r, g, b)
}

/// Adds a quad given as (top-left, top-right, bottom-left, bottom-right) corners
fn quad(mesh: &mut Mesh, corners: [(Pos2, Color32); 4]) {
    let idx = mesh.vertices.len() as u32;
    mesh.add_triangle(idx, idx + 1, idx + 2);
    mesh.add_triangle(idx + 2, idx + 1, idx + 3);
    for (pos, color) in corners {
        mesh.colored_vertex(pos, color);
    }
}

fn vertical_gradient(
    mesh: &mut Mesh,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    top_color: Color32,
    bottom_color: Color32,
) {
    quad(
        mesh,
        [
            (Pos2::new(left, top), top_color),
            (Pos2::new(right, top), top_color),
            (Pos2::new(left, bottom), bottom_color),
            (Pos2::new(right, bottom), bottom_color),
        ],
    );
}
