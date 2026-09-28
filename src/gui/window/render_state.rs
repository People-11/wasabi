use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderResolution {
    #[default]
    HD1080, // 1920x1080
    UHD4K, // 3840x2160
}

impl RenderResolution {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            RenderResolution::HD1080 => (1920, 1080),
            RenderResolution::UHD4K => (3840, 2160),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RenderResolution::HD1080 => "1920x1080",
            RenderResolution::UHD4K => "3840x2160",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderFrameRate {
    Fps30,
    #[default]
    Fps60,
    Fps120,
}

impl RenderFrameRate {
    pub fn value(&self) -> u32 {
        match self {
            RenderFrameRate::Fps30 => 30,
            RenderFrameRate::Fps60 => 60,
            RenderFrameRate::Fps120 => 120,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RenderFrameRate::Fps30 => "30 FPS",
            RenderFrameRate::Fps60 => "60 FPS",
            RenderFrameRate::Fps120 => "120 FPS",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ParseMode {
    #[default]
    Live,
    Pie,
}

impl ParseMode {
    pub fn label(&self) -> &'static str {
        match self {
            ParseMode::Live => "Live",
            ParseMode::Pie => "Pie",
        }
    }
}

/// Clones share the same counters
#[derive(Clone)]
pub struct RenderProgress {
    pub current_frame: Arc<AtomicU64>,
    pub total_frames: Arc<AtomicU64>,
    pub is_cancelled: Arc<AtomicBool>,
    pub is_complete: Arc<AtomicBool>,
    pub is_parsing: Arc<AtomicBool>,
    /// f64 bits, written by the render thread
    pub fps: Arc<AtomicU64>,
    pub eta: Arc<AtomicU64>,
    /// Why the last render failed, shown in the render window
    pub error: Arc<std::sync::Mutex<Option<String>>>,
}

impl Default for RenderProgress {
    fn default() -> Self {
        Self {
            current_frame: Arc::new(AtomicU64::new(0)),
            total_frames: Arc::new(AtomicU64::new(0)),
            is_cancelled: Arc::new(AtomicBool::new(false)),
            is_complete: Arc::new(AtomicBool::new(false)),
            is_parsing: Arc::new(AtomicBool::new(true)),
            fps: Arc::new(AtomicU64::new(0)),
            eta: Arc::new(AtomicU64::new(0)),
            error: Default::default(),
        }
    }
}

impl RenderProgress {
    pub fn progress(&self) -> f32 {
        let total = self.total_frames.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let current = self.current_frame.load(Ordering::Relaxed);
        current as f32 / total as f32
    }

    pub fn reset(&self) {
        self.current_frame.store(0, Ordering::Relaxed);
        self.total_frames.store(0, Ordering::Relaxed);
        self.is_cancelled.store(false, Ordering::Relaxed);
        self.is_complete.store(false, Ordering::Relaxed);
        self.is_parsing.store(true, Ordering::Relaxed);
        self.fps.store(0, Ordering::Relaxed);
        self.eta.store(0, Ordering::Relaxed);
        *self.error.lock().unwrap() = None;
    }

    /// Encode FPS and ETA in seconds
    pub fn get_performance_stats(&self) -> Option<(f64, u64)> {
        let fps = f64::from_bits(self.fps.load(Ordering::Relaxed));
        let eta = f64::from_bits(self.eta.load(Ordering::Relaxed));
        (fps > 0.0).then(|| (fps, eta as u64))
    }
}

pub struct RenderState {
    pub midi_path: Option<PathBuf>,
    pub ffmpeg_path: Option<PathBuf>,
    pub output_path: Option<PathBuf>,
    pub resolution: RenderResolution,
    pub frame_rate: RenderFrameRate,
    pub parse_mode: ParseMode,
    pub quality: u8,
    pub is_rendering: bool,
    pub progress: RenderProgress,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            midi_path: None,
            ffmpeg_path: None,
            output_path: None,
            resolution: RenderResolution::default(),
            frame_rate: RenderFrameRate::default(),
            parse_mode: ParseMode::default(),
            quality: 32,
            is_rendering: false,
            progress: RenderProgress::default(),
        }
    }
}
