use std::ops::Range;

use super::block::{BasicMIDINote, InRamNoteBlock};

pub struct InRamNoteColumnViewData {
    /// Number of notes from the beginning of the midi to the start of the render view
    pub notes_to_render_end: u64,
    /// Number of notes from the beginning of the midi to the end of the render view
    pub notes_to_render_start: u64,
    /// The range of blocks that are in the view
    pub block_range: Range<usize>,

    /// Number of notes that have passed the keyboard
    pub notes_to_keyboard: u64,
    /// Number of blocks that have passed the keyboard
    pub blocks_to_keyboard: usize,
}

impl InRamNoteColumnViewData {
    pub fn new() -> Self {
        InRamNoteColumnViewData {
            notes_to_render_end: 0,
            notes_to_render_start: 0,
            block_range: 0..0,
            notes_to_keyboard: 0,
            blocks_to_keyboard: 0,
        }
    }
}

pub struct InRamNoteColumn {
    pub data: InRamNoteColumnViewData,
    pub blocks: Vec<InRamNoteBlock>,
}

impl InRamNoteColumn {
    pub fn new(blocks: Vec<InRamNoteBlock>) -> Self {
        InRamNoteColumn {
            blocks,
            data: InRamNoteColumnViewData::new(),
        }
    }
}

/// Flattened storage for a single column's note data
/// All notes are stored in a single contiguous buffer
pub struct FlatNoteColumn {
    pub data: InRamNoteColumnViewData,
    block_info: Vec<NoteBlockInfo>,
    notes_buffer: Vec<BasicMIDINote>,
}

#[derive(Clone, Copy)]
pub struct NoteBlockInfo {
    pub start: f64,
    pub max_length: f32,
    notes_offset: u32,
    notes_len: u32,
}

impl FlatNoteColumn {
    /// Build a flattened column from individual blocks
    pub fn build_from_blocks(blocks: Vec<InRamNoteBlock>) -> Self {
        let mut block_info = Vec::with_capacity(blocks.len());
        let mut notes_buffer = Vec::new();

        for block in blocks {
            let notes_offset = notes_buffer.len() as u32;
            let notes_len = block.notes.len() as u32;

            notes_buffer.extend_from_slice(&block.notes);

            block_info.push(NoteBlockInfo {
                start: block.start,
                max_length: block.max_length,
                notes_offset,
                notes_len,
            });
        }

        FlatNoteColumn {
            data: InRamNoteColumnViewData::new(),
            block_info,
            notes_buffer,
        }
    }

    /// Get the notes slice for a specific block
    pub fn get_block_notes(&self, block_index: usize) -> &[BasicMIDINote] {
        let info = &self.block_info[block_index];
        let start = info.notes_offset as usize;
        let end = start + info.notes_len as usize;
        &self.notes_buffer[start..end]
    }

    /// Get block info for a specific block
    pub fn get_block_info(&self, block_index: usize) -> NoteBlockInfo {
        self.block_info[block_index]
    }

    /// Get the number of blocks
    pub fn blocks_len(&self) -> usize {
        self.block_info.len()
    }

    /// Get the number of notes in a specific block
    pub fn block_notes_len(&self, block_index: usize) -> usize {
        self.block_info[block_index].notes_len as usize
    }

    /// Check if blocks are empty
    pub fn is_empty(&self) -> bool {
        self.block_info.is_empty()
    }
}
