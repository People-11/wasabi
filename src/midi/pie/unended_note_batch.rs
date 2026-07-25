use std::hash::Hash;

use rustc_hash::FxHashMap;

pub struct RemovedValue<T> {
    pub value: T,
    pub is_last: bool,
}

const NONE: u32 = u32::MAX;

struct Node<T> {
    value: Option<T>,
    /// Previous/next live note in insertion order. Doubles as the free list
    /// link (in `next`) while the slot is free.
    prev: u32,
    next: u32,
    /// Next live note with the same key, for the per-key FIFO.
    next_in_key: u32,
}

/// Holds the notes that have started but not ended yet, for one MIDI key.
///
/// Two orderings are needed at once: insertion order (so `top_mut` can find the
/// most recently started note) and a FIFO per key (so a note-off matches the
/// oldest note-on of that track/channel). This is a slab of nodes threaded by
/// both, which makes every operation O(1) with no allocation per note. It used
/// to be a `BTreeMap` plus a `VecDeque` per key, which allocated and rebalanced
/// on every one of the file's note-ons and note-offs.
pub struct UnendedNotes<K: Hash + Eq, T> {
    nodes: Vec<Node<T>>,
    free: Vec<u32>,
    /// Head/tail of the insertion-ordered list of live notes
    head: u32,
    tail: u32,
    /// Head/tail of the FIFO chain for each key. Entries are left in place when
    /// a chain empties, so a track that keeps playing doesn't re-hash.
    keys: FxHashMap<K, (u32, u32)>,
    len: usize,
}

impl<K: Hash + Eq, T> UnendedNotes<K, T> {
    pub fn new() -> Self {
        UnendedNotes {
            nodes: Vec::new(),
            free: Vec::new(),
            head: NONE,
            tail: NONE,
            keys: FxHashMap::default(),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// The most recently started note that hasn't ended yet.
    pub fn top_mut(&mut self) -> Option<&mut T> {
        if self.tail == NONE {
            return None;
        }
        self.nodes[self.tail as usize].value.as_mut()
    }

    pub fn push_note(&mut self, key: K, note: T) -> u32 {
        let id = match self.free.pop() {
            Some(id) => {
                let node = &mut self.nodes[id as usize];
                node.value = Some(note);
                node.prev = self.tail;
                node.next = NONE;
                node.next_in_key = NONE;
                id
            }
            None => {
                let id = self.nodes.len() as u32;
                self.nodes.push(Node {
                    value: Some(note),
                    prev: self.tail,
                    next: NONE,
                    next_in_key: NONE,
                });
                id
            }
        };

        if self.tail == NONE {
            self.head = id;
        } else {
            self.nodes[self.tail as usize].next = id;
        }
        self.tail = id;

        let chain = self.keys.entry(key).or_insert((NONE, NONE));
        if chain.1 == NONE {
            *chain = (id, id);
        } else {
            let prev_tail = chain.1;
            chain.1 = id;
            self.nodes[prev_tail as usize].next_in_key = id;
        }

        self.len += 1;
        id
    }

    /// Pops the oldest unended note for this key, if any.
    pub fn get_note_for(&mut self, key: K) -> Option<RemovedValue<T>> {
        let chain = self.keys.get_mut(&key)?;
        let id = chain.0;
        if id == NONE {
            return None;
        }

        let node = &mut self.nodes[id as usize];
        let value = node.value.take()?;
        let (prev, next) = (node.prev, node.next);
        chain.0 = node.next_in_key;
        if chain.0 == NONE {
            chain.1 = NONE;
        }

        if prev == NONE {
            self.head = next;
        } else {
            self.nodes[prev as usize].next = next;
        }
        if next == NONE {
            self.tail = prev;
        } else {
            self.nodes[next as usize].prev = prev;
        }

        self.free.push(id);
        self.len -= 1;

        Some(RemovedValue {
            value,
            // `tail` was the most recently started live note before the unlink
            is_last: next == NONE,
        })
    }

    pub fn drain_all(&mut self) -> impl Iterator<Item = T> {
        let mut drained = Vec::with_capacity(self.len);

        let mut id = self.head;
        while id != NONE {
            let node = &mut self.nodes[id as usize];
            id = node.next;
            if let Some(value) = node.value.take() {
                drained.push(value);
            }
        }

        self.nodes = Vec::new();
        self.free = Vec::new();
        self.keys = FxHashMap::default();
        self.head = NONE;
        self.tail = NONE;
        self.len = 0;

        drained.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::UnendedNotes;

    #[test]
    fn fifo_per_key_and_insertion_order() {
        let mut notes: UnendedNotes<i32, i32> = UnendedNotes::new();

        // Two notes on track 1 with one on track 2 interleaved between them
        notes.push_note(1, 10);
        notes.push_note(2, 20);
        notes.push_note(1, 11);
        assert_eq!(notes.len(), 3);
        assert_eq!(notes.top_mut().copied(), Some(11));

        // Track 1 pops its oldest note first, and it isn't the newest live one
        let removed = notes.get_note_for(1).unwrap();
        assert_eq!(removed.value, 10);
        assert!(!removed.is_last);
        assert_eq!(notes.top_mut().copied(), Some(11));

        // 11 is the newest live note
        let removed = notes.get_note_for(1).unwrap();
        assert_eq!(removed.value, 11);
        assert!(removed.is_last);
        assert_eq!(notes.top_mut().copied(), Some(20));

        assert!(notes.get_note_for(1).is_none(), "chain is drained");
        assert!(notes.get_note_for(9).is_none(), "unknown key");

        // Freed slots get reused without disturbing the ordering
        notes.push_note(1, 30);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes.top_mut().copied(), Some(30));

        let drained: Vec<i32> = notes.drain_all().collect();
        assert_eq!(drained, vec![20, 30]);
        assert_eq!(notes.len(), 0);
        assert!(notes.top_mut().is_none());
    }
}
