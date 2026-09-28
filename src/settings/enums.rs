use serde_derive::{Deserialize, Serialize};
use std::{fmt::Debug, slice::Iter};

#[repr(usize)]
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum MidiParsing {
    #[default]
    Ram = 0,
    Live = 1,
    Cake = 2,
    Pie = 3,
}

impl MidiParsing {
    pub const fn as_str(self) -> &'static str {
        match self {
            MidiParsing::Ram => "Standard (RAM)",
            MidiParsing::Live => "Standard (Live)",
            MidiParsing::Cake => "Cake",
            MidiParsing::Pie => "Pie",
        }
    }
}


#[allow(clippy::enum_variant_names)]
#[repr(usize)]
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Synth {
    #[default]
    XSynth = 0,
    #[cfg(supported_os)]
    Kdmapi = 1,
    #[cfg(all(supported_os, not(target_os = "freebsd")))]
    MidiDevice = 2,
    None = 3,
}

impl Synth {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Synth::XSynth => "Built-In (XSynth)",
            #[cfg(supported_os)]
            Synth::Kdmapi => "KDMAPI",
            #[cfg(all(supported_os, not(target_os = "freebsd")))]
            Synth::MidiDevice => "MIDI Device",
            Synth::None => "None",
        }
    }
}


#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
#[serde(rename_all = "lowercase")]
pub enum Statistics {
    #[default]
    Time = 0,
    Fps = 1,
    VoiceCount = 2,
    Rendered = 3,
    NoteCount = 4,
    Polyphony = 5,
    Nps = 6,
}

impl Statistics {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Statistics::Time => "Time",
            Statistics::Fps => "FPS",
            Statistics::VoiceCount => "Voice Count",
            Statistics::Rendered => "Rendered",
            Statistics::NoteCount => "Note Count",
            Statistics::Polyphony => "Polyphony",
            Statistics::Nps => "NPS",
        }
    }

    pub fn iter() -> Iter<'static, Statistics> {
        static STATISTICS: [Statistics; 7] = [
            Statistics::Time,
            Statistics::Fps,
            Statistics::Rendered,
            Statistics::Nps,
            Statistics::Polyphony,
            Statistics::VoiceCount,
            Statistics::NoteCount,
        ];
        STATISTICS.iter()
    }
}


#[repr(usize)]
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Colors {
    #[default]
    Rainbow = 0,
    Random = 1,
    Palette = 2,
    White = 3,
    PianoFromAbove = 4,
}

impl Colors {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Colors::Rainbow => "Rainbow",
            Colors::Random => "Random",
            Colors::Palette => "Palette",
            Colors::White => "White",
            Colors::PianoFromAbove => "Piano From Above",
        }
    }
}

