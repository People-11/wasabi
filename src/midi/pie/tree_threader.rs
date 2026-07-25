use std::sync::{Arc, Mutex};

use rayon::iter::{IndexedParallelIterator, IntoParallelRefMutIterator, ParallelIterator};

use crate::midi::MIDIColor;

use super::{
    blocks::{FlatPieBlocks, PieBlockInfo},
    tree_serializer::TreeSerializer,
};

const OFF_FLAG: i32 = i32::MIN;

/// One note-on or note-off, as handed to the serializer threads.
///
/// Kept at 8 bytes: this is the file's biggest data stream (two of these per
/// note, so billions of them), and the note's color used to ride along here even
/// though it is just a lookup on `channel_track` that the worker can do itself.
#[derive(Clone, Copy)]
pub struct NoteEvent {
    time: i32,
    /// The channel/track, with the sign bit set to mark a note-off
    tagged_channel_track: i32,
}

impl NoteEvent {
    pub fn on(time: i32, channel_track: i32) -> Self {
        NoteEvent {
            time,
            tagged_channel_track: channel_track,
        }
    }

    pub fn off(time: i32, channel_track: i32) -> Self {
        NoteEvent {
            time,
            tagged_channel_track: channel_track | OFF_FLAG,
        }
    }
}

pub struct ThreadedTreeSerializers {
    trees: Arc<Mutex<Vec<TreeSerializer>>>,
    rcv: crossbeam_channel::Receiver<Vec<Vec<NoteEvent>>>,
    snd: crossbeam_channel::Sender<Vec<Vec<NoteEvent>>>,
    join: std::thread::JoinHandle<()>,

    current_vec: Vec<Vec<NoteEvent>>,
    cached_event_count: usize,
}

impl ThreadedTreeSerializers {
    fn make_vecs() -> Vec<Vec<NoteEvent>> {
        vec![Vec::new(); 256]
    }

    pub fn new(colors: Vec<MIDIColor>) -> ThreadedTreeSerializers {
        let trees = (0..256).map(|_| TreeSerializer::new()).collect::<Vec<_>>();
        let trees = Arc::new(Mutex::new(trees));

        let (snd_in, rcv_in) = crossbeam_channel::unbounded::<Vec<Vec<NoteEvent>>>();
        let (snd_back, rcv_back) = crossbeam_channel::unbounded::<Vec<Vec<NoteEvent>>>();

        let trees_thread = trees.clone();
        let handle = std::thread::spawn(move || {
            let mut trees = trees_thread.lock().unwrap();
            let colors = &colors;

            for mut vecs in rcv_in.into_iter() {
                vecs.par_iter_mut()
                    .zip(trees.par_iter_mut())
                    .for_each(|(events, tree)| {
                        for event in events.drain(..) {
                            let tagged = event.tagged_channel_track;
                            if tagged < 0 {
                                tree.end_note(event.time, tagged & i32::MAX);
                            } else {
                                let color = colors[tagged as usize].as_u32() as i32;
                                tree.start_note(event.time, tagged, color);
                            }
                        }
                    });
                snd_back.send(vecs).unwrap();
            }
        });

        snd_in.send(ThreadedTreeSerializers::make_vecs()).unwrap();

        ThreadedTreeSerializers {
            trees,
            rcv: rcv_back,
            snd: snd_in,
            join: handle,

            current_vec: ThreadedTreeSerializers::make_vecs(),
            cached_event_count: 0,
        }
    }

    fn swap_buffers(&mut self) {
        self.cached_event_count = 0;
        let recieved = self.rcv.recv().unwrap();

        let send = std::mem::replace(&mut self.current_vec, recieved);
        self.snd.send(send).unwrap();
    }

    pub fn push_event(&mut self, key: usize, event: NoteEvent) {
        self.current_vec[key].push(event);
        self.cached_event_count += 1;

        if self.cached_event_count > 1024 * 1024 {
            self.swap_buffers();
        }
    }

    pub fn seal_flat(self, time: i32) -> FlatPieBlocks {
        self.snd.send(self.current_vec).unwrap();
        drop(self.snd);

        self.rcv.recv().unwrap();
        self.rcv.recv().unwrap();

        self.join.join().unwrap();

        let trees = Arc::try_unwrap(self.trees).unwrap().into_inner().unwrap();

        let mut block_info = Vec::with_capacity(trees.len());
        let mut tree_buffer = Vec::new();

        for tree in trees.into_iter() {
            let (tree_offset, tree_len) = tree.complete_and_append_to(time, &mut tree_buffer);
            block_info.push(PieBlockInfo {
                tree_offset,
                tree_len,
            });
        }

        FlatPieBlocks::from_parts(0, time as u32, block_info, tree_buffer)
    }
}
