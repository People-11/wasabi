mod cake;
mod live;
mod ram;

mod audio;

mod shared;
use std::{fs::File, path::PathBuf, time::UNIX_EPOCH};

use image::{DynamicImage, GenericImageView, ImageReader};
use palette::{convert::FromColorUnclamped, Hsv, Srgb};
use rand::seq::IteratorRandom;
use rand::Rng;

pub use cake::{CakeBlock, CakeMIDIFile, CakeSignature, IntVector4};
pub use live::LiveLoadMIDIFile;
pub use ram::InRamMIDIFile;

use crate::{
    gui::window::WasabiError,
    settings::{Colors, MidiSettings},
};

use self::shared::timer::TimeKeeper;

#[derive(Debug, Clone, Copy, Default)]
pub struct MIDIFileStats {
    pub total_notes: Option<u64>,
    pub passed_notes: Option<u64>,
}

/// A struct that represents the view range of a midi screen render
#[derive(Debug, Clone, Copy, Default)]
pub struct MIDIViewRange {
    pub start: f64,
    pub end: f64,
}

impl MIDIViewRange {
    pub fn new(start: f64, end: f64) -> Self {
        MIDIViewRange { start, end }
    }

    pub fn length(&self) -> f64 {
        self.end - self.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MIDIFileUniqueSignature {
    pub filepath: PathBuf,
    pub length_in_bytes: u64,
    pub last_modified: u128,
}

fn open_file_and_signature(
    path: impl Into<PathBuf>,
) -> Result<(File, MIDIFileUniqueSignature), WasabiError> {
    let path = path.into();
    let file = std::fs::File::open(&path).map_err(WasabiError::FilesystemError)?;
    let file_length = file.metadata().map_err(WasabiError::FilesystemError)?.len();
    let file_last_modified = file
        .metadata()
        .map_err(WasabiError::FilesystemError)?
        .modified()
        .map_err(WasabiError::FilesystemError)?
        .duration_since(UNIX_EPOCH)
        .map_err(|e: std::time::SystemTimeError| WasabiError::Other(e.to_string()))?
        .as_micros();

    let signature = MIDIFileUniqueSignature {
        filepath: path,
        length_in_bytes: file_length,
        last_modified: file_last_modified,
    };

    Ok((file, signature))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MIDIColor(u32);

impl MIDIColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        let num = (b as u32) | ((g as u32) << 8) | ((r as u32) << 16);
        MIDIColor(num)
    }

    pub fn new_from_hue(hue: f64) -> Self {
        let hsv: Hsv<Srgb, f64> = palette::Hsv::new(hue, 1.0, 0.8);
        let rgb = palette::rgb::Rgb::from_color_unclamped(hsv);
        Self::new(
            (rgb.red * 255.0) as u8,
            (rgb.green * 255.0) as u8,
            (rgb.blue * 255.0) as u8,
        )
    }

    pub fn new_vec(tracks: usize) -> Vec<Self> {
        (0..tracks * 16)
            .map(|i| {
                let (track, channel) = (i / 16, i % 16);
                MIDIColor::new_from_hue((track + channel) as f64 * -16.0 % 360.0)
            })
            .collect()
    }

    pub fn new_random_vec(tracks: usize) -> Vec<Self> {
        let mut rng = rand::rng();
        (0..tracks * 16)
            .map(|_| MIDIColor::new(rng.random(), rng.random(), rng.random()))
            .collect()
    }

    pub fn new_white_vec(tracks: usize) -> Vec<Self> {
        vec![MIDIColor::new(255, 255, 255); tracks * 16]
    }

    pub fn new_pfa_vec(tracks: usize) -> Vec<Self> {
        let mut base_colors: [MIDIColor; 16] = std::array::from_fn(|count| {
            let i = (10 + count * 7) % 16;
            let hue = 360.0 * i as f64 / 16.0;
            let hsv: Hsv<Srgb, f64> = palette::Hsv::new(hue, 0.73, 0.87);
            let rgb = palette::rgb::Rgb::from_color_unclamped(hsv);
            Self::new(
                (rgb.red * 255.0) as u8,
                (rgb.green * 255.0) as u8,
                (rgb.blue * 255.0) as u8,
            )
        });
        base_colors.swap(2, 4);
        base_colors.into_iter().cycle().take(tracks * 16).collect()
    }

    pub fn new_vec_from_palette(tracks: usize, image: DynamicImage, randomize: bool) -> Vec<Self> {
        let image = image.to_rgb8();
        let all_colors = image.pixels().map(|p| Self::new(p.0[0], p.0[1], p.0[2]));

        let num = tracks * 16;
        if randomize {
            let mut rng = rand::rng();
            all_colors
                .choose_multiple(&mut rng, num)
                .into_iter()
                .cycle()
                .take(num)
                .collect()
        } else {
            all_colors.cycle().take(num).collect()
        }
    }

    pub fn new_vec_from_settings(
        tracks: usize,
        settings: &MidiSettings,
    ) -> Result<Vec<Self>, WasabiError> {
        match settings.colors {
            Colors::Rainbow => Ok(MIDIColor::new_vec(tracks)),
            Colors::Random => Ok(MIDIColor::new_random_vec(tracks)),
            Colors::White => Ok(MIDIColor::new_white_vec(tracks)),
            Colors::PianoFromAbove => Ok(MIDIColor::new_pfa_vec(tracks)),
            Colors::Palette => {
                let path = &settings.palette_path;
                if path.exists() {
                    let image = ImageReader::open(path)
                        .map_err(|e| WasabiError::PaletteError(e.to_string()))?;
                    let image = image
                        .with_guessed_format()
                        .map_err(|e| WasabiError::PaletteError(e.to_string()))?;
                    let image = image
                        .decode()
                        .map_err(|e| WasabiError::PaletteError(e.to_string()))?;

                    if image.dimensions().0 == 16 {
                        Ok(MIDIColor::new_vec_from_palette(
                            tracks,
                            image,
                            settings.randomize_palette,
                        ))
                    } else {
                        Err(WasabiError::PaletteError(format!(
                            "Palette has invalid dimensions: {path:?}"
                        )))
                    }
                } else {
                    Err(WasabiError::PaletteError(format!(
                        "Palette does not exist: {path:?}"
                    )))
                }
            }
        }
    }

    pub fn as_u32(&self) -> u32 {
        self.0
    }

    pub fn from_u32(num: u32) -> Self {
        MIDIColor(num)
    }

    pub fn red(&self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub fn green(&self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub fn blue(&self) -> u8 {
        self.0 as u8
    }
}

/// The basic shared functions in a midi file. The columns related functions are
/// inside the [`MIDIFile`] trait.
pub trait MIDIFileBase {
    fn midi_length(&self) -> Option<f64>;

    fn timer(&self) -> &TimeKeeper;
    fn timer_mut(&mut self) -> &mut TimeKeeper;

    fn stats(&self) -> MIDIFileStats;

    fn allows_seeking_backward(&self) -> bool;
}

/// This trait contains a function to retrieve the column view of the midi
pub trait MIDIFile: MIDIFileBase {
    type ColumnsViews<'a>: 'a + MIDINoteViews
    where
        Self: 'a;

    fn get_current_column_views(&mut self, range: f64) -> Self::ColumnsViews<'_>;
}

pub trait MIDINoteViews {
    type View<'a>: 'a + MIDINoteColumnView
    where
        Self: 'a;

    fn get_column(&self, key: usize) -> Self::View<'_>;
    fn range(&self) -> MIDIViewRange;
}

pub trait MIDINoteColumnView: Send {
    type Iter<'a>: 'a + ExactSizeIterator<Item = DisplacedMIDINote> + Send
    where
        Self: 'a;

    fn iterate_displaced_notes(&self) -> Self::Iter<'_>;
}

pub struct DisplacedMIDINote {
    pub start: f32,
    pub len: f32,
    pub color: MIDIColor,
}

pub enum MIDIFileUnion {
    InRam(ram::InRamMIDIFile),
    Live(live::LiveLoadMIDIFile),
    Cake(cake::CakeMIDIFile),
}

impl MIDIFileBase for MIDIFileUnion {
    fn midi_length(&self) -> Option<f64> {
        match self {
            MIDIFileUnion::InRam(f) => f.midi_length(),
            MIDIFileUnion::Live(f) => f.midi_length(),
            MIDIFileUnion::Cake(f) => f.midi_length(),
        }
    }
    fn timer(&self) -> &TimeKeeper {
        match self {
            MIDIFileUnion::InRam(f) => f.timer(),
            MIDIFileUnion::Live(f) => f.timer(),
            MIDIFileUnion::Cake(f) => f.timer(),
        }
    }
    fn timer_mut(&mut self) -> &mut TimeKeeper {
        match self {
            MIDIFileUnion::InRam(f) => f.timer_mut(),
            MIDIFileUnion::Live(f) => f.timer_mut(),
            MIDIFileUnion::Cake(f) => f.timer_mut(),
        }
    }
    fn stats(&self) -> MIDIFileStats {
        match self {
            MIDIFileUnion::InRam(f) => f.stats(),
            MIDIFileUnion::Live(f) => f.stats(),
            MIDIFileUnion::Cake(f) => f.stats(),
        }
    }
    fn allows_seeking_backward(&self) -> bool {
        match self {
            MIDIFileUnion::InRam(f) => f.allows_seeking_backward(),
            MIDIFileUnion::Live(f) => f.allows_seeking_backward(),
            MIDIFileUnion::Cake(f) => f.allows_seeking_backward(),
        }
    }
}
