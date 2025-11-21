use std::sync::Arc;

use gen_iter::GenIter;
use midi_toolkit::{
    events::{Event, MIDIEventEnum},
    sequence::event::{Delta, EventBatch, Track},
};

// New struct to represent individual audio blocks, similar to the old CompressedAudio
pub struct RawAudioBlock {
    pub time: f64,
    pub data: Vec<u8>,
    pub control_only_data: Option<Vec<u8>>,
}

pub struct FlatAudio {
    pub blocks: Vec<AudioBlockInfo>,
    data_buffer: Vec<u8>,
    control_data_buffer: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct AudioBlockInfo {
    pub time: f64,
    data_offset: u64,
    data_len: u64,
    control_data_offset: u64,
    control_data_len: u64,
}

const EV_OFF: u8 = 0x80;
const EV_ON: u8 = 0x90;
const EV_POLYPHONIC: u8 = 0xA0;
const EV_CONTROL: u8 = 0xB0;
const EV_PROGRAM: u8 = 0xC0;
const EV_CHAN_PRESSURE: u8 = 0xD0;
const EV_PITCH_BEND: u8 = 0xE0;

impl RawAudioBlock {
    pub fn build_raw_blocks<
        Iter: Iterator<Item = Arc<Delta<f64, Track<EventBatch<E>>>>>,
        E: MIDIEventEnum,
    >(
        iter: Iter,
    ) -> impl Iterator<Item = RawAudioBlock> {
        let mut builder_vec: Vec<u8> = Vec::new();
        let mut control_builder_vec: Vec<u8> = Vec::new();
        GenIter(
            #[coroutine]
            move || {
                let mut time = 0.0;

                for block in iter {
                    time += block.delta;

                    let min_len: usize = block.count() * 3;

                    builder_vec.reserve(min_len);
                    builder_vec.clear();
                    control_builder_vec.clear(); // Clear control builder for each block

                    for event in block.iter_events() {
                        match event.as_event() {
                            Event::NoteOn(e) => {
                                let head = EV_ON | e.channel;
                                let events = &[head, e.key, e.velocity];
                                builder_vec.extend_from_slice(events);
                            }
                            Event::NoteOff(e) => {
                                let head = EV_OFF | e.channel;
                                let events = &[head, e.key];
                                builder_vec.extend_from_slice(events);
                            }
                            Event::PolyphonicKeyPressure(e) => {
                                let head = EV_POLYPHONIC | e.channel;
                                let events = &[head, e.key, e.velocity];
                                builder_vec.extend_from_slice(events);
                            }
                            Event::ControlChange(e) => {
                                let head = EV_CONTROL | e.channel;
                                let events = &[head, e.controller, e.value];
                                builder_vec.extend_from_slice(events);
                                control_builder_vec.extend_from_slice(events);
                            }
                            Event::ProgramChange(e) => {
                                let head = EV_PROGRAM | e.channel;
                                let events = &[head, e.program];
                                builder_vec.extend_from_slice(events);
                                control_builder_vec.extend_from_slice(events);
                            }
                            Event::ChannelPressure(e) => {
                                let head = EV_CHAN_PRESSURE | e.channel;
                                let events = &[head, e.pressure];
                                builder_vec.extend_from_slice(events);
                                control_builder_vec.extend_from_slice(events);
                            }
                            Event::PitchWheelChange(e) => {
                                let head = EV_PITCH_BEND | e.channel;
                                let value = e.pitch + 8192;
                                let events =
                                    &[head, (value & 0x7F) as u8, ((value >> 7) & 0x7F) as u8];
                                builder_vec.extend_from_slice(events);
                                control_builder_vec.extend_from_slice(events);
                            }
                            _ => {}
                        }
                    }

                    let new_control_vec = if control_builder_vec.is_empty() {
                        None
                    } else {
                        let mut new_control_vec = Vec::with_capacity(control_builder_vec.len());
                        new_control_vec.append(&mut control_builder_vec);
                        Some(new_control_vec)
                    };

                    yield RawAudioBlock {
                        data: builder_vec.drain(..).collect(), // Collect drained items
                        control_only_data: new_control_vec,
                        time,
                    };
                }
            },
        )
    }

    pub fn iter_events(&self) -> impl '_ + Iterator<Item = u32> {
        RawAudioBlock::iter_events_from_vec(self.data.iter().cloned())
    }

    pub fn iter_control_events(&self) -> impl '_ + Iterator<Item = u32> {
        RawAudioBlock::iter_events_from_vec(self.control_only_data.iter().flatten().cloned())
    }

    fn iter_events_from_vec<'a>(
        mut iter: impl 'a + Iterator<Item = u8>,
    ) -> impl 'a + Iterator<Item = u32> {
        GenIter(
            #[coroutine]
            move || {
                while let Some(next) = iter.next() {
                    let ev = next & 0xF0;
                    let val = match ev {
                        EV_OFF | EV_PROGRAM | EV_CHAN_PRESSURE => {
                            let val2 = iter.next().unwrap() as u32;
                            (next as u32) | (val2 << 8)
                        }
                        EV_ON | EV_POLYPHONIC | EV_CONTROL | EV_PITCH_BEND => {
                            let val2 = iter.next().unwrap() as u32;
                            let val3 = iter.next().unwrap() as u32;
                            (next as u32) | (val2 << 8) | (val3 << 16)
                        }
                        _ => panic!("Can't reach {next:#x}"),
                    };

                    yield val;
                }
            },
        )
    }
}

impl FlatAudio {
    pub fn build_blocks<Iter: Iterator<Item = RawAudioBlock>>(iter: Iter) -> FlatAudio {
        let mut blocks = Vec::new();
        let mut data_buffer = Vec::new();
        let mut control_data_buffer = Vec::new();

        for raw_block in iter {
            let data_offset = data_buffer.len() as u64;
            let data_len = raw_block.data.len() as u64;
            data_buffer.extend_from_slice(&raw_block.data);

            let control_data_offset = control_data_buffer.len() as u64;
            let control_data_len = raw_block
                .control_only_data
                .as_ref()
                .map_or(0, |v| v.len() as u64);
            if let Some(control_data) = raw_block.control_only_data {
                control_data_buffer.extend_from_slice(&control_data);
            }

            blocks.push(AudioBlockInfo {
                time: raw_block.time,
                data_offset,
                data_len,
                control_data_offset,
                control_data_len,
            });
        }

        FlatAudio {
            blocks,
            data_buffer,
            control_data_buffer,
        }
    }

    pub fn iter_events(&self, block_index: usize) -> impl '_ + Iterator<Item = u32> {
        let block_info = self.blocks[block_index];
        let start = block_info.data_offset as usize;
        let end = start + block_info.data_len as usize;
        let iter = self.data_buffer[start..end].iter().cloned();
        RawAudioBlock::iter_events_from_vec(iter)
    }

    pub fn iter_control_events(&self, block_index: usize) -> impl '_ + Iterator<Item = u32> {
        let block_info = self.blocks[block_index];
        let start = block_info.control_data_offset as usize;
        let end = start + block_info.control_data_len as usize;
        let iter = self.control_data_buffer[start..end].iter().cloned();
        RawAudioBlock::iter_events_from_vec(iter)
    }
}
