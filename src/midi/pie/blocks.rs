use crate::midi::MIDIColor;

/// Note data for all 256 keys.
///
/// The binary trees are only needed by the GPU, so the renderer takes them on its first
/// draw and they are freed from RAM after upload. The CPU keeps what it queries every
/// frame in a much smaller form: each key's top-note color over time, and a global count
/// of started notes.
pub struct FlatPieBlocks {
    start_time: u32,
    end_time: u32,
    block_info: Vec<PieBlockInfo>,
    trees: Option<Vec<Box<[i32]>>>,
    key_colors: Vec<KeyColors>,
    /// (tick, notes started up to and including that tick), one entry per tick with note-ons
    note_ons: Box<[(i32, u64)]>,
}

/// A key's position in the virtual concatenation of all trees, which is how the
/// renderer lays them out on the GPU.
#[derive(Clone, Copy)]
pub struct PieBlockInfo {
    pub tree_offset: usize,
    pub tree_len: usize,
}

impl FlatPieBlocks {
    pub(super) fn new(
        start_time: u32,
        end_time: u32,
        trees: Vec<Box<[i32]>>,
        key_colors: Vec<KeyColors>,
        note_ons: Box<[(i32, u64)]>,
    ) -> Self {
        let mut tree_offset = 0;
        let block_info = trees
            .iter()
            .map(|tree| {
                let info = PieBlockInfo {
                    tree_offset,
                    tree_len: tree.len(),
                };
                tree_offset += tree.len();
                info
            })
            .collect();

        FlatPieBlocks {
            start_time,
            end_time,
            block_info,
            trees: Some(trees),
            key_colors,
            note_ons,
        }
    }

    pub fn start_time(&self) -> u32 {
        self.start_time
    }

    pub fn end_time(&self) -> u32 {
        self.end_time
    }

    /// Hands the per-key trees over for GPU upload. Returns None if already taken.
    pub fn take_trees(&mut self) -> Option<Vec<Box<[i32]>>> {
        self.trees.take()
    }

    pub fn get_block_info(&self, key: usize) -> PieBlockInfo {
        self.block_info[key]
    }

    /// Get the number of blocks (should be 256)
    pub fn len(&self) -> usize {
        self.block_info.len()
    }

    /// Total length of all trees
    pub fn total_len(&self) -> usize {
        self.block_info
            .last()
            .map_or(0, |info| info.tree_offset + info.tree_len)
    }

    /// Color of the note shown on top of `key` at `time`, if any
    pub fn key_color_at(&self, key: usize, time: i32) -> Option<MIDIColor> {
        self.key_colors[key].color_at(time)
    }

    /// Number of notes (on all keys) that started before `time`
    pub fn notes_passed_at(&self, time: i32) -> u64 {
        let index = self.note_ons.partition_point(|&(tick, _)| tick < time);
        index.checked_sub(1).map_or(0, |i| self.note_ons[i].1)
    }
}

const NO_NOTE: u32 = u32::MAX;

/// The top note's color on one key as a step function of time: `colors[i]` applies from
/// `starts[i]` until `starts[i + 1]`. Consecutive runs of the same color are merged, so
/// this is a fraction of the size of the key's tree.
pub struct KeyColors {
    starts: Box<[i32]>,
    colors: Box<[u32]>,
}

impl KeyColors {
    /// Walks the tree's leaves in time order and records where the visible color changes.
    ///
    /// Tree layout: a node is `[cutoff, left, right, notes_to_the_left]`; a child offset
    /// > 0 points back to a note `[start, end, color]`, otherwise to another node.
    pub fn from_tree(tree: &[i32]) -> Self {
        enum Child {
            Node(usize),
            Note(usize),
        }
        fn child(node: usize, offset: i32) -> Child {
            if offset > 0 {
                Child::Note(node - offset as usize)
            } else {
                Child::Node(node - (-offset) as usize)
            }
        }

        let mut starts = Vec::new();
        let mut colors = Vec::new();
        let mut push = |time: i32, color: u32| {
            // A later change at the same time replaces an earlier one
            if starts.last() == Some(&time) {
                starts.pop();
                colors.pop();
            }
            if colors.last() != Some(&color) {
                starts.push(time);
                colors.push(color);
            }
        };

        // (child, interval start, interval end), visited left to right
        let mut stack = vec![(Child::Node(tree[0] as usize), i32::MIN, i32::MAX)];
        while let Some((current, lo, hi)) = stack.pop() {
            match current {
                Child::Node(node) => {
                    let cutoff = tree[node];
                    stack.push((child(node, tree[node + 2]), cutoff, hi));
                    stack.push((child(node, tree[node + 1]), lo, cutoff));
                }
                Child::Note(note) => {
                    // The leaf's note may start after the interval does or end before it ends
                    let (start, end, color) = (tree[note], tree[note + 1], tree[note + 2]);
                    let (from, to) = (start.max(lo), end.min(hi));
                    push(lo, NO_NOTE);
                    if color != -1 && from < to {
                        push(from, color as u32);
                        push(to, NO_NOTE);
                    }
                }
            }
        }

        KeyColors {
            starts: starts.into_boxed_slice(),
            colors: colors.into_boxed_slice(),
        }
    }

    fn color_at(&self, time: i32) -> Option<MIDIColor> {
        // The first run always starts at i32::MIN, so the index is at least 1
        let index = self.starts.partition_point(|&start| start <= time);
        let color = self.colors[index - 1];
        (color != NO_NOTE).then(|| MIDIColor::from_u32(color))
    }
}
