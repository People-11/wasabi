use std::sync::Arc;

use gen_iter::GenIter;
use midi_toolkit::{
    events::{Event, MIDIEventEnum},
    sequence::event::{Delta, EventBatch, Track},
};

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
/// A note-on with the velocity of the previous one, which is left out (see `EventWriter`)
const EV_ON_SAME_VELOCITY: u8 = 0xF0;

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
                    control_builder_vec.clear();

                    let mut data = EventWriter::new(&mut builder_vec);
                    let mut control_data = EventWriter::new(&mut control_builder_vec);
                    for event in block.iter_events() {
                        append_event(event.as_event(), &mut data, &mut control_data);
                    }

                    let control_only_data = (!control_builder_vec.is_empty())
                        .then(|| control_builder_vec.clone());

                    yield RawAudioBlock {
                        data: builder_vec.clone(),
                        control_only_data,
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
                // Running state, see `EventWriter`
                let mut status = 0u8;
                let mut velocity = 0u8;
                while let Some(mut next) = iter.next() {
                    if next & 0x80 != 0 {
                        status = next;
                        next = iter.next().unwrap();
                    }
                    let val2 = next as u32;
                    let val = match status & 0xF0 {
                        EV_ON_SAME_VELOCITY => {
                            (EV_ON | status & 0x0F) as u32 | (val2 << 8) | ((velocity as u32) << 16)
                        }
                        EV_OFF | EV_PROGRAM | EV_CHAN_PRESSURE => (status as u32) | (val2 << 8),
                        ev @ (EV_ON | EV_POLYPHONIC | EV_CONTROL | EV_PITCH_BEND) => {
                            let val3 = iter.next().unwrap();
                            if ev == EV_ON {
                                velocity = val3;
                            }
                            (status as u32) | (val2 << 8) | ((val3 as u32) << 16)
                        }
                        _ => panic!("Can't reach {status:#x}"),
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

            let mut data = EventWriter::new(&mut audio.data_buffer);
            let mut control_data = EventWriter::new(&mut audio.control_data_buffer);
            for event in batch.iter_events() {
                append_event(event.as_event(), &mut data, &mut control_data);
            }

            audio.finish_batch(time, data_start, control_start);
        }

        audio.blocks.shrink_to_fit();
        audio.control_blocks.shrink_to_fit();
        audio.data_buffer.shrink_to_fit();
        audio.control_data_buffer.shrink_to_fit();
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

/// Writes one block's events. Within a block a status byte equal to the previous one is
/// left out (running status), as is the velocity of a note-on that repeats the previous
/// note-on's (written as `EV_ON_SAME_VELOCITY`). A new writer starts with no running state,
/// so every block can be decoded on its own.
struct EventWriter<'a> {
    buf: &'a mut Vec<u8>,
    status: u8,
    velocity: u8,
}

impl<'a> EventWriter<'a> {
    fn new(buf: &'a mut Vec<u8>) -> Self {
        // Status bytes are >= 0x80 and velocities < 0x80, so neither matches at first
        EventWriter {
            buf,
            status: 0,
            velocity: 0xFF,
        }
    }

    fn write(&mut self, mut status: u8, data: &[u8]) {
        // Data bytes must stay below 0x80 to be told apart from status bytes
        let mut bytes = [0; 2];
        for (byte, value) in bytes.iter_mut().zip(data) {
            *byte = value & 0x7F;
        }
        let mut data = &bytes[..data.len()];

        if status & 0xF0 == EV_ON {
            if data[1] == self.velocity {
                status = EV_ON_SAME_VELOCITY | status & 0x0F;
                data = &data[..1];
            } else {
                self.velocity = data[1];
            }
        }
        if status != self.status {
            self.buf.push(status);
            self.status = status;
        }
        self.buf.extend_from_slice(data);
    }
}

fn append_event(event: &Event, data: &mut EventWriter, control_data: &mut EventWriter) {
    let (status, bytes, control) = match event {
        Event::NoteOn(e) => (EV_ON | e.channel, [e.key, e.velocity], false),
        Event::NoteOff(e) => (EV_OFF | e.channel, [e.key, 0], false),
        Event::PolyphonicKeyPressure(e) => (EV_POLYPHONIC | e.channel, [e.key, e.velocity], false),
        Event::ControlChange(e) => (EV_CONTROL | e.channel, [e.controller, e.value], true),
        Event::ProgramChange(e) => (EV_PROGRAM | e.channel, [e.program, 0], true),
        Event::ChannelPressure(e) => (EV_CHAN_PRESSURE | e.channel, [e.pressure, 0], true),
        Event::PitchWheelChange(e) => {
            let value = e.pitch + 8192;
            let bytes = [(value & 0x7F) as u8, ((value >> 7) & 0x7F) as u8];
            (EV_PITCH_BEND | e.channel, bytes, true)
        }
        _ => return,
    };
    let len = match status & 0xF0 {
        EV_OFF | EV_PROGRAM | EV_CHAN_PRESSURE => 1,
        _ => 2,
    };
    data.write(status, &bytes[..len]);
    if control {
        control_data.write(status, &bytes[..len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_status_round_trip() {
        // (status, data): repeated statuses and velocities, a velocity change, a channel
        // change, an out-of-range data byte, and every other event type
        let events: &[(u8, &[u8])] = &[
            (0x90, &[60, 100]),
            (0x90, &[62, 100]),
            (0x90, &[64, 90]),
            (0x91, &[64, 90]),
            (0x80, &[60]),
            (0x80, &[62]),
            (0x91, &[65, 200]),
            (0xA3, &[1, 2]),
            (0xB4, &[7, 127]),
            (0xC5, &[3]),
            (0xD6, &[4]),
            (0xE7, &[0, 64]),
            (0x90, &[66, 72]),
        ];
        let mut buf = Vec::new();
        let mut writer = EventWriter::new(&mut buf);
        for &(status, data) in events {
            writer.write(status, data);
        }
        let decoded: Vec<u32> = RawAudioBlock::iter_events_from_vec(buf.iter().cloned()).collect();
        let expected: Vec<u32> = events
            .iter()
            .map(|&(status, data)| {
                data.iter()
                    .enumerate()
                    .fold(status as u32, |v, (i, &b)| v | ((b & 0x7F) as u32) << (8 * (i + 1)))
            })
            .collect();
        assert_eq!(decoded, expected);
        // 35 bytes without running status or velocity
        assert_eq!(buf.len(), 31);
    }
}
