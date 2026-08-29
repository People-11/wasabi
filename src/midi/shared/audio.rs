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
    control_blocks: Vec<AudioBlockInfo>,
    data_buffer: Vec<u8>,
    control_data_buffer: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct AudioBlockInfo {
    pub time: f64,
    data_end: usize,
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
                        append_event(event.as_event(), &mut builder_vec, &mut control_builder_vec);
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
    pub fn build_from_batches<
        Iter: Iterator<Item = Arc<Delta<f64, Track<EventBatch<E>>>>>,
        E: MIDIEventEnum,
    >(
        iter: Iter,
    ) -> FlatAudio {
        let mut audio = FlatAudio {
            blocks: Vec::new(),
            control_blocks: Vec::new(),
            data_buffer: Vec::new(),
            control_data_buffer: Vec::new(),
        };
        let mut time = 0.0;

        for batch in iter {
            time += batch.delta;
            let data_start = audio.data_buffer.len();
            let control_start = audio.control_data_buffer.len();

            audio.data_buffer.reserve(batch.count() * 3);
            for event in batch.iter_events() {
                append_event(
                    event.as_event(),
                    &mut audio.data_buffer,
                    &mut audio.control_data_buffer,
                );
            }

            audio.finish_batch(time, data_start, control_start);
        }

        audio
    }

    fn finish_batch(&mut self, time: f64, data_start: usize, control_start: usize) {
        fn finish_stream(blocks: &mut Vec<AudioBlockInfo>, time: f64, data_end: usize) {
            if let Some(block) = blocks.last_mut().filter(|block| block.time == time) {
                block.data_end = data_end;
            } else {
                blocks.push(AudioBlockInfo { time, data_end });
            }
        }

        if self.data_buffer.len() != data_start {
            finish_stream(&mut self.blocks, time, self.data_buffer.len());
        }
        if self.control_data_buffer.len() != control_start {
            finish_stream(
                &mut self.control_blocks,
                time,
                self.control_data_buffer.len(),
            );
        }
    }

    pub fn iter_events(&self, block_index: usize) -> impl '_ + Iterator<Item = u32> {
        let start = block_index
            .checked_sub(1)
            .map_or(0, |index| self.blocks[index].data_end);
        let end = self.blocks[block_index].data_end;
        let iter = self.data_buffer[start..end].iter().cloned();
        RawAudioBlock::iter_events_from_vec(iter)
    }

    pub fn iter_control_events_before(&self, time: f64) -> impl '_ + Iterator<Item = u32> {
        self.control_blocks
            .iter()
            .enumerate()
            .take_while(move |(_, block)| block.time < time)
            .flat_map(|(index, block)| {
                let start = index
                    .checked_sub(1)
                    .map_or(0, |index| self.control_blocks[index].data_end);
                let iter = self.control_data_buffer[start..block.data_end]
                    .iter()
                    .cloned();
                RawAudioBlock::iter_events_from_vec(iter)
            })
    }
}

fn append_event(event: &Event, data: &mut Vec<u8>, control_data: &mut Vec<u8>) {
    match event {
        Event::NoteOn(e) => data.extend_from_slice(&[EV_ON | e.channel, e.key, e.velocity]),
        Event::NoteOff(e) => data.extend_from_slice(&[EV_OFF | e.channel, e.key]),
        Event::PolyphonicKeyPressure(e) => {
            data.extend_from_slice(&[EV_POLYPHONIC | e.channel, e.key, e.velocity])
        }
        Event::ControlChange(e) => {
            let event = [EV_CONTROL | e.channel, e.controller, e.value];
            data.extend_from_slice(&event);
            control_data.extend_from_slice(&event);
        }
        Event::ProgramChange(e) => {
            let event = [EV_PROGRAM | e.channel, e.program];
            data.extend_from_slice(&event);
            control_data.extend_from_slice(&event);
        }
        Event::ChannelPressure(e) => {
            let event = [EV_CHAN_PRESSURE | e.channel, e.pressure];
            data.extend_from_slice(&event);
            control_data.extend_from_slice(&event);
        }
        Event::PitchWheelChange(e) => {
            let value = e.pitch + 8192;
            let event = [
                EV_PITCH_BEND | e.channel,
                (value & 0x7F) as u8,
                ((value >> 7) & 0x7F) as u8,
            ];
            data.extend_from_slice(&event);
            control_data.extend_from_slice(&event);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioBlockInfo, FlatAudio};

    fn append_batch(audio: &mut FlatAudio, time: f64, data: &[u8], control_data: &[u8]) {
        let data_start = audio.data_buffer.len();
        let control_start = audio.control_data_buffer.len();
        audio.data_buffer.extend_from_slice(data);
        audio.control_data_buffer.extend_from_slice(control_data);
        audio.finish_batch(time, data_start, control_start);
    }

    #[test]
    fn flat_audio_merges_times_and_indexes_controls_sparsely() {
        let mut audio = FlatAudio {
            blocks: Vec::new(),
            control_blocks: Vec::new(),
            data_buffer: Vec::new(),
            control_data_buffer: Vec::new(),
        };

        append_batch(&mut audio, 1.0, &[0x92, 60, 100], &[]);
        append_batch(&mut audio, 1.0, &[0x82, 60], &[]);
        append_batch(&mut audio, 1.5, &[], &[]);
        append_batch(&mut audio, 2.0, &[0xb2, 7, 99], &[0xb2, 7, 99]);
        append_batch(&mut audio, 2.0, &[0xc2, 4], &[0xc2, 4]);

        assert_eq!(std::mem::size_of::<AudioBlockInfo>(), 16);
        assert_eq!(audio.blocks.len(), 2);
        assert_eq!(audio.control_blocks.len(), 1);
        assert_eq!(
            audio.iter_events(0).collect::<Vec<_>>(),
            vec![0x0064_3c92, 0x0000_3c82]
        );
        assert_eq!(
            audio.iter_events(1).collect::<Vec<_>>(),
            vec![0x0063_07b2, 0x0000_04c2]
        );
        assert!(audio.iter_control_events_before(2.0).next().is_none());
        assert_eq!(
            audio.iter_control_events_before(3.0).collect::<Vec<_>>(),
            vec![0x0063_07b2, 0x0000_04c2]
        );
    }
}
