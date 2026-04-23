pub mod notes_render_pass;

use std::sync::Arc;

use vulkano::image::view::ImageView;

use crate::{
    gui::window::keyboard_layout::KeyboardView,
    midi::{DisplacedMIDINote, MIDIColor, MIDIFile, MIDINoteColumnView, MIDINoteViews},
    utils,
};

use self::notes_render_pass::{NotePassStatus, NoteRenderPass, NoteVertex};

use super::RenderResultData;

#[derive(Default)]
struct ColumnReturnData {
    polyphony: usize,
    written_notes: usize,
}

pub struct NoteRenderer {
    render_pass: NoteRenderPass,
}

impl NoteRenderer {
    pub fn new(
        device: Arc<vulkano::device::Device>,
        queue: Arc<vulkano::device::Queue>,
        format: vulkano::format::Format,
    ) -> NoteRenderer {
        NoteRenderer {
            render_pass: NoteRenderPass::new(device, queue, format),
        }
    }

    pub fn draw(
        &mut self,
        key_view: &KeyboardView,
        final_image: Arc<ImageView>,
        midi_file: &mut impl MIDIFile,
        view_range: f64,
        bg_color: Option<[f32; 4]>,
        viewport: Option<vulkano::pipeline::graphics::viewport::Viewport>,
    ) -> RenderResultData {
        let note_views = midi_file.get_current_column_views(view_range);

        struct ColumnViewInfo<Iter: ExactSizeIterator<Item = DisplacedMIDINote> + Send> {
            offset: usize,
            iter: Iter,
            key: u8,
            remaining: usize,
            color: Option<MIDIColor>,
            border_width: f32,
        }

        let mut total_notes = 0;

        let columns: Vec<_> = (0..256).map(|i| note_views.get_column(i)).collect();

        let mut columns_view_info = Vec::new();

        let border_width = utils::calculate_border_width(
            final_image.image().extent()[0] as f32,
            key_view.visible_range.len() as f32,
        );

        // Black keys first
        for (i, column) in columns.iter().enumerate() {
            if key_view.key(i).black {
                let iter = column.iterate_displaced_notes();
                let length = iter.len();
                columns_view_info.push(ColumnViewInfo {
                    offset: total_notes,
                    iter,
                    key: i as u8,
                    remaining: length,
                    color: None,
                    border_width,
                });
                total_notes += length;
            }
        }

        // Then white keys after
        for (i, column) in columns.iter().enumerate() {
            if !key_view.key(i).black {
                let iter = column.iterate_displaced_notes();
                let length = iter.len();
                columns_view_info.push(ColumnViewInfo {
                    offset: total_notes,
                    iter,
                    key: i as u8,
                    remaining: length,
                    color: None,
                    border_width,
                });
                total_notes += length;
            }
        }

        let mut notes_pushed = 0;
        let mut polyphony = 0;

        let view_range = note_views.range().length() as f32;

        self.render_pass.draw(
            final_image,
            key_view,
            view_range,
            bg_color,
            viewport,
            |buffer| {
                let buffer_length = buffer.len() as usize;

                let mut mapped_buffer = buffer.write().unwrap();
                let mut column_data = ColumnReturnData::default();

                for column in columns_view_info.iter_mut().rev() {
                    if column.remaining == 0 {
                        continue;
                    }

                    let offset = (column.offset as i64 - notes_pushed as i64).max(0) as usize;

                    if offset >= buffer_length {
                        continue;
                    }

                    let remaining_buffer_space = buffer_length - offset;
                    let allowed_to_write = column.remaining.min(remaining_buffer_space);
                    let mut poly = 0;

                    // Hoist per-column constants to avoid redundant casts per note
                    let key_u32 = column.key as u32;
                    let border_width_u32 = column.border_width as u32;

                    for i in 0..allowed_to_write {
                        let note = column.iter.next().unwrap();
                        mapped_buffer[i + offset] = NoteVertex {
                            start_length: [note.start, note.len],
                            key_color: key_u32 | (note.color.as_u32() << 8),
                            border_width: border_width_u32,
                        };

                        if note.start <= 0.0 && note.start + note.len > 0.0 {
                            poly += 1;
                            if column.color.is_none() {
                                column.color = Some(note.color);
                            }
                        }
                    }

                    column.remaining -= allowed_to_write;
                    column_data.polyphony += poly;
                    column_data.written_notes += allowed_to_write;
                }

                drop(mapped_buffer);

                polyphony += column_data.polyphony;
                notes_pushed += column_data.written_notes;

                if notes_pushed >= total_notes {
                    NotePassStatus::Finished {
                        remaining: column_data.written_notes as u32,
                    }
                } else {
                    NotePassStatus::HasMoreNotes
                }
            },
        );

        // Sort for output metrics
        columns_view_info.sort_unstable_by_key(|k| k.key);

        RenderResultData {
            notes_rendered: notes_pushed as u64,
            polyphony: Some(polyphony as u64),
            key_colors: columns_view_info
                .iter()
                .map(|column| column.color)
                .collect(),
        }
    }
}
