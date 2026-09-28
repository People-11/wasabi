use std::ops::RangeInclusive;

#[cfg(supported_os)]
use crate::settings::WasabiSoundfont;

pub const WIN_MARGIN: egui::Margin = egui::Margin::same(12);
pub const NOTE_SPEED_RANGE: RangeInclusive<f64> = 10.0..=0.01;

pub fn calculate_border_width(width_pixels: f32, keys_len: f32) -> f32 {
    ((width_pixels / keys_len) / 12.0).clamp(1.0, 5.0).round() * 2.0
}

pub fn convert_seconds_to_time_string(sec: f64) -> String {
    let time_millis = (sec * 10.0) as i64 % 10;
    let time_sec = sec as i64 % 60;
    let time_min = sec as i64 / 60;

    format!(
        "{:02}:{:02}.{}",
        time_min.abs(),
        time_sec.abs(),
        time_millis.abs()
    )
}

pub fn create_window_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::inner_margin(egui::Frame::window(ctx.style().as_ref()), WIN_MARGIN)
}

#[cfg(supported_os)]
pub fn create_om_sf_list(list: &[WasabiSoundfont]) -> String {
    list.iter()
        .map(|sf| {
            format!(
                "sf.start\nsf.path = {}\nsf.enabled = {}\nsf.preload = 1\nsf.srcb = {}\nsf.srcp = {}\n\
                 sf.desb = 0\nsf.desp = -1\nsf.desblsb = 0\nsf.xgdrums = 0\nsf.end\n\n",
                sf.path.to_str().unwrap_or_default(),
                sf.enabled as u8,
                sf.options.bank.map_or(-1, i32::from),
                sf.options.preset.map_or(-1, i32::from),
            )
        })
        .collect()
}

#[cfg(supported_os)]
pub fn create_reset_midi_messages() -> Vec<u32> {
    // All Sound Off (120) and Reset All Controllers (121) on every channel
    (0..16u32)
        .flat_map(|ch| [120u32, 121].map(|cc| cc << 8 | 0xB0 | ch))
        .collect()
}
