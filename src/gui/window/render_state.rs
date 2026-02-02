use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

/// 渲染分辨率预设
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
            RenderResolution::HD1080 => "1080P (1920×1080)",
            RenderResolution::UHD4K => "4K (3840×2160)",
        }
    }
}

/// 渲染帧率预设
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

/// 渲染进度跟踪（线程安全）
pub struct RenderProgress {
    pub current_frame: Arc<AtomicU64>,
    pub total_frames: Arc<AtomicU64>,
    pub is_cancelled: Arc<AtomicBool>,
    pub is_complete: Arc<AtomicBool>,
    pub is_parsing: Arc<AtomicBool>,
    pub fps_history: Arc<std::sync::Mutex<std::collections::VecDeque<(std::time::Instant, u64)>>>,
}

impl Default for RenderProgress {
    fn default() -> Self {
        Self {
            current_frame: Arc::new(AtomicU64::new(0)),
            total_frames: Arc::new(AtomicU64::new(0)),
            is_cancelled: Arc::new(AtomicBool::new(false)),
            is_complete: Arc::new(AtomicBool::new(false)),
            is_parsing: Arc::new(AtomicBool::new(true)),
            fps_history: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
        }
    }
}

impl RenderProgress {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

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
        self.fps_history.lock().unwrap().clear();
    }

    /// Calculate sliding window FPS and ETA
    pub fn get_performance_stats(&self) -> Option<(f64, u64)> {
        let current = self.current_frame.load(Ordering::Relaxed);
        let total = self.total_frames.load(Ordering::Relaxed);
        if current == 0 {
            return None;
        }

        let mut history = self.fps_history.lock().ok()?;
        let now = std::time::Instant::now();

        // Push current sample
        history.push_back((now, current));

        // Keep last 1.5s for a smooth window
        while history.len() > 2 && now.duration_since(history.front()?.0).as_secs_f64() > 1.5 {
            history.pop_front();
        }

        if history.len() < 2 {
            return None;
        }

        let (start_time, start_frame) = history.front()?;
        let (end_time, end_frame) = history.back()?;

        let dt = end_time.duration_since(*start_time).as_secs_f64();
        if dt < 0.1 {
            return None;
        }

        let fps = (end_frame - start_frame) as f64 / dt;
        let remaining = total.saturating_sub(current);
        let eta = if fps > 0.1 {
            (remaining as f64 / fps) as u64
        } else {
            0
        };

        Some((fps, eta))
    }
}

/// 渲染状态
#[derive(Default)]
pub struct RenderState {
    pub midi_path: Option<PathBuf>,
    pub ffmpeg_path: Option<PathBuf>,
    pub output_path: Option<PathBuf>,
    pub resolution: RenderResolution,
    pub frame_rate: RenderFrameRate,
    pub is_rendering: bool,
    pub original_vsync: Option<bool>,
    pub progress: RenderProgress,
}

impl RenderState {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// 检查是否可以开始渲染
    #[allow(dead_code)]
    pub fn can_start(&self) -> bool {
        self.midi_path.is_some() && self.ffmpeg_path.is_some()
    }

    /// 重置渲染状态
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.is_rendering = false;
        self.progress.reset();
    }
}
