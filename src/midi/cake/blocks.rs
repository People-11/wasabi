use crate::midi::MIDIColor;

pub struct CakeBlock {
    pub start_time: u32,
    pub end_time: u32,
    pub tree: Vec<i32>,
}

/// Flattened storage for all cake blocks' tree data
/// This stores all 256 keys' IntVector4 data in a single contiguous buffer
pub struct FlatCakeBlocks {
    block_info: Vec<CakeBlockInfo>,
    pub tree_buffer: Vec<i32>,
}

#[derive(Clone, Copy)]
pub struct CakeBlockInfo {
    pub start_time: u32,
    pub end_time: u32,
    pub tree_offset: usize,
    pub tree_len: usize,
}

pub struct CakeNoteData {
    pub start_time: u32,
    pub end_time: u32,
    pub color: MIDIColor,
}

impl FlatCakeBlocks {
    /// Build flattened blocks from individual tree vectors
    pub fn build_blocks(trees: Vec<Vec<i32>>, start_time: u32, end_time: u32) -> Self {
        let mut block_info = Vec::with_capacity(trees.len());
        let mut tree_buffer = Vec::new();

        for tree in trees {
            let tree_offset = tree_buffer.len();
            let tree_len = tree.len();

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
    pub fn get_tree(&self, key: usize) -> &[i32] {
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
    pub fn get_note_at(&self, key: usize, time: i32) -> Option<CakeNoteData> {
        let tree = self.get_tree(key);
        if tree.is_empty() {
            return None;
        }

        let mut next_index = tree[0] as usize; // Length

        loop {
            // Check if we are at a Leaf (Branch) or Note?
            // We loop until we FIND a Note (via offset > 0 logic).
            // Current index points to a Branch.

            let cutoff = tree[next_index];
            
            let offset = if time < cutoff {
                tree[next_index + 1]
            } else {
                tree[next_index + 2]
            };

            if offset > 0 {
                next_index -= offset as usize;
                break;
            }
            let offset = -offset;
            next_index -= offset as usize;
        }

        // Now next_index points to a Note (3 ints)
        // [Start, End, Color]
        let note_start = tree[next_index];
        let note_end = tree[next_index + 1];
        let note_color = tree[next_index + 2];

        if time < note_start || time >= note_end {
            return None;
        }

        if note_color == -1 {
            None
        } else {
            Some(CakeNoteData {
                start_time: note_start as u32,
                end_time: note_end as u32,
                color: MIDIColor::from_u32(note_color as u32),
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
        let mut next_index = tree[0] as usize;

        loop {
            let cutoff = tree[next_index];
            
            let offset = if time < cutoff {
                tree[next_index + 1]
            } else {
                tree[next_index + 2]
            };

            // Count is at index+3
            last_notes_passed = tree[next_index + 3] as u32;

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
    pub fn get_note_at(&self, time: i32) -> Option<CakeNoteData> {
        let mut next_index = self.tree[0] as usize;

        loop {
            let cutoff = self.tree[next_index];
            let offset = if time < cutoff {
                self.tree[next_index + 1]
            } else {
                self.tree[next_index + 2]
            };

            if offset > 0 {
                next_index -= offset as usize;
                break;
            }
            let offset = -offset;
            next_index -= offset as usize;
        }

        let note_start = self.tree[next_index];
        let note_end = self.tree[next_index + 1];
        let note_color = self.tree[next_index + 2];

        if time < note_start || time >= note_end {
            return None;
        }

        if note_color == -1 {
            None
        } else {
            Some(CakeNoteData {
                start_time: note_start as u32,
                end_time: note_end as u32,
                color: MIDIColor::from_u32(note_color as u32),
            })
        }
    }
    pub fn get_notes_passed_at(&self, time: i32) -> u32 {
        let mut last_notes_passed;
        let mut next_index = self.tree[0] as usize;

        loop {
            let cutoff = self.tree[next_index];
            let offset = if time < cutoff {
                self.tree[next_index + 1]
            } else {
                self.tree[next_index + 2]
            };

            last_notes_passed = self.tree[next_index + 3] as u32;

            if offset > 0 {
                break;
            }
            let offset = -offset;
            next_index -= offset as usize;
        }

        last_notes_passed
    }
}
