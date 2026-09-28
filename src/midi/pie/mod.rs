use std::{path::PathBuf, sync::Arc, thread};
use time::Duration;

use midi_toolkit::{
    events::{Event, MIDIEventEnum},
    io::MIDIFile as TKMIDIFile,
    pipe,
    sequence::{
        event::{cancel_tempo_events, scale_event_time, Delta, EventBatch, Track},
        unwrap_items, TimeCaster,
    },
};

use crate::{
    audio_playback::WasabiAudioPlayer,
    gui::window::WasabiError,
    midi::{
        audio::ram::InRamAudioPlayer,
        pie::{
            blocks::FlatPieBlocks,
            tree_threader::{NoteEvent, ThreadedTreeSerializers},
        },
        shared::{audio::FlatAudio, timer::TimeKeeper},
        MIDIColor,
    },
    settings::MidiSettings,
};

use super::{MIDIFileBase, MIDIFileStats};

pub mod blocks;
mod tree_serializer;
mod tree_threader;
mod unended_note_batch;

pub struct PieMIDIFile {
    blocks: FlatPieBlocks,
    timer: TimeKeeper,
    length: f64,
    note_count: u64,
    ticks_per_second: u32,
}

impl PieMIDIFile {
    pub fn load_from_file(
        path: impl Into<PathBuf>,
        player: Arc<WasabiAudioPlayer>,
        settings: &MidiSettings,
    ) -> Result<Self, WasabiError> {
        fn channel_track(channel: u8, track: u32) -> i32 {
            channel as i32 + track as i32 * 16
        }

        let ticks_per_second = 10000;

        let file = std::fs::File::open(path.into()).map_err(WasabiError::FilesystemError)?;
        let midi = TKMIDIFile::open_from_stream(file, None).map_err(WasabiError::MidiLoadError)?;

        let ppq = midi.ppq();
        let merged = pipe!(
            midi.iter_all_track_events_merged_batches()
            |>TimeCaster::<f64>::cast_event_delta()
            |>cancel_tempo_events(250000)
            |>scale_event_time(1.0 / ppq as f64)
            |>unwrap_items()
        );

        let colors = MIDIColor::new_vec_from_settings(midi.track_count(), settings)?;

        type Ev = Delta<f64, Track<EventBatch<Event>>>;
        let (key_snd, key_rcv) = crossbeam_channel::bounded::<Arc<Ev>>(1000);
        let (audio_snd, audio_rcv) = crossbeam_channel::bounded::<Arc<Ev>>(1000);

        let key_join_handle = thread::spawn(move || {
            let mut trees = ThreadedTreeSerializers::new(colors);

            let mut time = 0.0;
            let mut note_count = 0u64;
            let mut note_ons: Vec<(i32, u64)> = Vec::new();

            for batch in key_rcv.into_iter() {
                time += batch.delta;

                let int_time = (time * ticks_per_second as f64) as i32;

                for event in batch.iter_events() {
                    let track = event.track;
                    let note_event = match event.as_event() {
                        Event::NoteOn(e) => {
                            note_count += 1;
                            Some((
                                e.key as usize,
                                NoteEvent::on(int_time, channel_track(e.channel, track)),
                            ))
                        }
                        Event::NoteOff(e) => Some((
                            e.key as usize,
                            NoteEvent::off(int_time, channel_track(e.channel, track)),
                        )),
                        _ => None,
                    };

                    if let Some((key, note_event)) = note_event {
                        trees.push_event(key, note_event);
                    }
                }

                if note_ons.last().map_or(0, |&(_, count)| count) != note_count {
                    match note_ons.last_mut() {
                        Some(last) if last.0 == int_time => last.1 = note_count,
                        _ => note_ons.push((int_time, note_count)),
                    }
                }
            }
            let final_time = (time * ticks_per_second as f64) as i32;

            let blocks = trees.seal_flat(final_time, note_ons.into_boxed_slice());

            (blocks, note_count)
        });

        let audio_join_handle =
            thread::spawn(move || FlatAudio::build_from_batches(audio_rcv.into_iter()));

        let mut length = 0.0;

        for batch in merged {
            length += batch.delta;
            let batch = Arc::new(batch);
            key_snd.send(batch.clone()).unwrap();
            audio_snd.send(batch).unwrap();
        }
        // Drop the writers so the threads finish
        drop(key_snd);
        drop(audio_snd);

        let (blocks, note_count) = key_join_handle.join().unwrap();
        let audio = Arc::new(audio_join_handle.join().unwrap());

        let mut timer = TimeKeeper::new(settings.start_delay);

        InRamAudioPlayer::new(audio, timer.get_listener(), player).spawn_playback();

        Ok(PieMIDIFile {
            blocks,
            timer,
            length,
            note_count,
            ticks_per_second,
        })
    }

    pub fn flat_blocks(&self) -> &FlatPieBlocks {
        &self.blocks
    }

    pub fn flat_blocks_mut(&mut self) -> &mut FlatPieBlocks {
        &mut self.blocks
    }

    pub fn ticks_per_second(&self) -> u32 {
        self.ticks_per_second
    }

    pub fn current_time(&self) -> Duration {
        self.timer.get_time()
    }
}

impl MIDIFileBase for PieMIDIFile {
    fn midi_length(&self) -> Option<f64> {
        Some(self.length)
    }

    fn timer(&self) -> &TimeKeeper {
        &self.timer
    }

    fn timer_mut(&mut self) -> &mut TimeKeeper {
        &mut self.timer
    }

    fn allows_seeking_backward(&self) -> bool {
        true
    }

    fn stats(&self) -> MIDIFileStats {
        let time = self.timer.get_time().as_seconds_f64();
        let time_int = (time * self.ticks_per_second as f64) as i32;

        MIDIFileStats {
            total_notes: Some(self.note_count),
            passed_notes: Some(self.blocks.notes_passed_at(time_int)),
        }
    }
}
