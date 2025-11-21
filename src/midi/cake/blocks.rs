use crate::midi::{IntVector4, MIDIColor};

pub struct CakeBlock {
    pub start_time: u32,
    pub end_time: u32,
    pub tree: Vec<IntVector4>,
}

/// Flattened storage for all cake blocks' tree data
/// This stores all 256 keys' IntVector4 data in a single contiguous buffer
pub struct FlatCakeBlocks {
    block_info: Vec<CakeBlockInfo>,
    tree_buffer: Vec<IntVector4>,
}

#[derive(Clone, Copy)]
pub struct CakeBlockInfo {
    pub start_time: u32,
    pub end_time: u32,
    tree_offset: u32,
    tree_len: u32,
}

pub struct CakeNoteData {
    pub start_time: u32,
    pub end_time: u32,
    pub color: MIDIColor,
}

impl FlatCakeBlocks {
    /// Build flattened blocks from individual tree vectors
    pub fn build_blocks(trees: Vec<Vec<IntVector4>>, start_time: u32, end_time: u32) -> Self {
        let mut block_info = Vec::with_capacity(trees.len());
        let mut tree_buffer = Vec::new();

        for tree in trees {
            let tree_offset = tree_buffer.len() as u32;
            let tree_len = tree.len() as u32;

            tree_buffer.extend_from_slice(&tree);

            block_info.push(CakeBlockInfo {
                start_time,
                end_time,
                tree_offset,
                tree_len,
            });
        }

        FlatCakeBlocks {
            block_info,
            tree_buffer,
        }
    }

    /// Get the tree slice for a specific key
    pub fn get_tree(&self, key: usize) -> &[IntVector4] {
        let info = &self.block_info[key];
        let start = info.tree_offset as usize;
        let end = start + info.tree_len as usize;
        &self.tree_buffer[start..end]
    }

    /// Get block info for a specific key
    pub fn get_block_info(&self, key: usize) -> CakeBlockInfo {
        self.block_info[key]
    }

    /// Get the number of blocks (should be 256)
    pub fn len(&self) -> usize {
        self.block_info.len()
    }

    /// Get tree length for a specific key
    pub fn tree_len(&self, key: usize) -> usize {
        self.block_info[key].tree_len as usize
    }

    /// Get note at a specific time for a specific key
    pub fn get_note_at(&self, key: usize, time: u32) -> Option<CakeNoteData> {
        let tree = self.get_tree(key);
        if tree.is_empty() {
            return None;
        }

        let mut next_index = tree[0].length_marker_len();

        loop {
            let node = tree[next_index];

            let offset = if time < node.leaf_cutoff() as u32 {
                node.leaf_left()
            } else {
                node.leaf_right()
            };

            if offset > 0 {
                next_index -= offset as usize;
                break;
            }
            let offset = -offset;
            next_index -= offset as usize;
        }

        let note = tree[next_index];

        if note.is_note_empty() {
            None
        } else {
            Some(CakeNoteData {
                start_time: note.note_start(),
                end_time: note.note_end(),
                color: MIDIColor::from_u32(note.note_color()),
            })
        }
    }

    /// Get the number of notes that have passed at a specific time for a specific key
    pub fn get_notes_passed_at(&self, key: usize, time: i32) -> u32 {
        let tree = self.get_tree(key);
        if tree.is_empty() {
            return 0;
        }

        let mut last_notes_passed;
        let mut next_index = tree[0].length_marker_len();

        loop {
            let node = tree[next_index];

            let offset = if time < node.leaf_cutoff() {
                node.leaf_left()
            } else {
                node.leaf_right()
            };

            last_notes_passed = node.leaf_notes_to_the_left();

            if offset > 0 {
                break;
            }
            let offset = -offset;
            next_index -= offset as usize;
        }

        last_notes_passed
    }
}

impl CakeBlock {
    pub fn get_note_at(&self, time: u32) -> Option<CakeNoteData> {
        let mut next_index = self.tree[0].length_marker_len();

        loop {
            let node = self.tree[next_index];

            let offset = if time < node.leaf_cutoff() as u32 {
                node.leaf_left()
            } else {
                node.leaf_right()
            };

            if offset > 0 {
                next_index -= offset as usize;
                break;
            }
            let offset = -offset;
            next_index -= offset as usize;
        }

        let note = self.tree[next_index];

        if note.is_note_empty() {
            None
        } else {
            Some(CakeNoteData {
                start_time: note.note_start(),
                end_time: note.note_end(),
                color: MIDIColor::from_u32(note.note_color()),
            })
        }
    }
    pub fn get_notes_passed_at(&self, time: i32) -> u32 {
        let mut last_notes_passed;
        let mut next_index = self.tree[0].length_marker_len();

        loop {
            let node = self.tree[next_index];

            let offset = if time < node.leaf_cutoff() {
                node.leaf_left()
            } else {
                node.leaf_right()
            };

            last_notes_passed = node.leaf_notes_to_the_left();

            if offset > 0 {
                break;
            }
            let offset = -offset;
            next_index -= offset as usize;
        }

        last_notes_passed
    }
}
