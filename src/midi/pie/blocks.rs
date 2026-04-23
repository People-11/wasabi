use crate::midi::MIDIColor;

/// Flattened storage for all cake blocks' tree data
/// This stores all 256 keys' IntVector4 data in a single contiguous buffer
pub struct FlatPieBlocks {
    start_time: u32,
    end_time: u32,
    block_info: Vec<PieBlockInfo>,
    pub tree_buffer: Vec<i32>,
}

#[derive(Clone, Copy)]
pub struct PieBlockInfo {
    pub tree_offset: usize,
    pub tree_len: usize,
}

#[derive(Clone, Copy)]
pub struct PieNoteData {
    #[allow(dead_code)]
    pub start_time: u32,
    #[allow(dead_code)]
    pub end_time: u32,
    pub color: MIDIColor,
}

impl FlatPieBlocks {
    pub(super) fn from_parts(
        start_time: u32,
        end_time: u32,
        block_info: Vec<PieBlockInfo>,
        tree_buffer: Vec<i32>,
    ) -> Self {
        FlatPieBlocks {
            start_time,
            end_time,
            block_info,
            tree_buffer,
        }
    }

    pub fn start_time(&self) -> u32 {
        self.start_time
    }

    pub fn end_time(&self) -> u32 {
        self.end_time
    }

    /// Get the tree slice for a specific key
    pub fn get_tree(&self, key: usize) -> &[i32] {
        let info = &self.block_info[key];
        let start = info.tree_offset as usize;
        let end = start + info.tree_len as usize;
        &self.tree_buffer[start..end]
    }

    /// Get block info for a specific key
    pub fn get_block_info(&self, key: usize) -> PieBlockInfo {
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

    #[inline(always)]
    fn traverse_leaf(ptr: *const i32, time: i32) -> (u32, usize) {
        let mut next_index = unsafe { *ptr } as usize;

        loop {
            let cutoff = unsafe { *ptr.add(next_index) };
            let child_idx = if time < cutoff { next_index + 1 } else { next_index + 2 };
            let offset = unsafe { *ptr.add(child_idx) };

            if offset > 0 {
                return (unsafe { *ptr.add(next_index + 3) } as u32, next_index - offset as usize);
            }

            next_index -= (-offset) as usize;
        }
    }

    #[inline(always)]
    fn note_at(ptr: *const i32, leaf_index: usize, time: i32) -> Option<PieNoteData> {
        let note_start = unsafe { *ptr.add(leaf_index) };
        if time < note_start {
            return None;
        }

        let note_end = unsafe { *ptr.add(leaf_index + 1) };
        if time >= note_end {
            return None;
        }

        let note_color = unsafe { *ptr.add(leaf_index + 2) };
        (note_color != -1).then(|| PieNoteData {
            start_time: note_start as u32,
            end_time: note_end as u32,
            color: MIDIColor::from_u32(note_color as u32),
        })
    }

    /// Get the number of notes that have passed at a specific time for a specific key
    pub fn get_notes_passed_at(&self, key: usize, time: i32) -> u32 {
        let tree = self.get_tree(key);
        if tree.is_empty() {
            return 0;
        }

        Self::traverse_leaf(tree.as_ptr(), time).0
    }

    pub fn get_window_stats_at(
        &self,
        key: usize,
        start_time: i32,
        end_time: i32,
    ) -> (Option<PieNoteData>, u32, u32) {
        let tree = self.get_tree(key);
        if tree.is_empty() {
            return (None, 0, 0);
        }

        let ptr = tree.as_ptr();
        let (start_notes_passed, start_leaf) = Self::traverse_leaf(ptr, start_time);
        let end_notes_passed = if start_time == end_time {
            start_notes_passed
        } else {
            Self::traverse_leaf(ptr, end_time).0
        };

        (
            Self::note_at(ptr, start_leaf, start_time),
            start_notes_passed,
            end_notes_passed,
        )
    }
}
