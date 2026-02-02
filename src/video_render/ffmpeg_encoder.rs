//! FFmpeg encoder for video generation
//!
//! Handles piping raw frame data to FFmpeg for H.265 encoding.
//! Uses an async channel to decouple rendering from encoding.
//! Automatically detects and uses hardware acceleration when available.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::{self, JoinHandle};

/// Message types for the writer thread
enum WriterMessage {
    Frame(Vec<u8>),
    Finish,
}

/// Available hardware encoders (in priority order)
#[derive(Debug, Clone, Copy, PartialEq)]
enum HwEncoder {
    NvencHevc, // NVIDIA
    VaapiHevc, // vaapi
    AmfHevc,   // AMD Proprietary
    QsvHevc,   // Intel Proprietary
    Software,  // Fallback: libx265
}

impl HwEncoder {
    fn codec_name(&self) -> &'static str {
        match self {
            HwEncoder::NvencHevc => "hevc_nvenc",
            HwEncoder::VaapiHevc => "hevc_vaapi", //vaapi
            HwEncoder::QsvHevc => "hevc_qsv",
            HwEncoder::AmfHevc => "hevc_amf",
            HwEncoder::Software => "libx265",
        }
    }

    fn global_args(&self) -> &'static [&'static str] {
        match self {
            HwEncoder::VaapiHevc => &[
                "-init_hw_device",
                "vaapi=va:/dev/dri/renderD128",
                "-filter_hw_device",
                "va",
            ],
            _ => &[],
        }
    }

    fn format_args(&self) -> &'static [&'static str] {
        match self {
            HwEncoder::VaapiHevc => &["-vf", "format=nv12,hwupload"],
            HwEncoder::Software => &["-pix_fmt", "yuv420p"],
            _ => &["-pix_fmt", "nv12"],
        }
    }

    fn quality_args(&self, quality: u8, fps: u32) -> Vec<String> {
        match self {
            HwEncoder::NvencHevc => vec![
                "-preset".to_string(), "p7".to_string(), "-tune".to_string(), "hq".to_string(), "-rc".to_string(), "constqp".to_string(), "-qp".to_string(), quality.to_string(), "-spatial-aq".to_string(), "1".to_string(), "-temporal-aq".to_string(), "1".to_string(), "-rc-lookahead".to_string(), (fps / 2).to_string(),
            ],
            HwEncoder::VaapiHevc => vec![
                "-rc_mode".to_string(), "CQP".to_string(), "-qp".to_string(), quality.to_string(),
                // "-compression_level", "1", Because 1 represents prioritizing speed over quality, the CQP mode embodies the “quality-first” logic for this hardware.
            ],
            HwEncoder::AmfHevc => vec![
                "-quality".to_string(), "quality".to_string(), "-rc".to_string(), "cqp".to_string(), "-qp_i".to_string(), quality.to_string(), "-qp_p".to_string(), quality.to_string(),
            ],
            HwEncoder::QsvHevc => vec!["-preset".to_string(), "veryslow".to_string(), "-global_quality".to_string(), quality.to_string(), "-look_ahead".to_string(), "1".to_string()],
            HwEncoder::Software => vec!["-crf".to_string(), quality.to_string(), "-preset".to_string(), "medium".to_string()],
        }
    }
}

/// FFmpeg video encoder with async writing
/// Takes raw BGRA frame data and encodes it to H.265 video.
/// Uses a background thread for writing to prevent blocking the render loop.
/// Maximum number of frames to buffer before blocking
/// At 60 FPS, this is 1 second
const MAX_FRAME_BUFFER: usize = 60;

pub struct FFmpegEncoder {
    sender: Option<SyncSender<WriterMessage>>,
    recycle_receiver: Receiver<Vec<u8>>,
    writer_thread: Option<JoinHandle<std::io::Result<()>>>,
    width: u32,
    height: u32,
}

/// Detect available hardware encoder by testing FFmpeg
fn detect_hw_encoder(ffmpeg_path: &Path) -> HwEncoder {
    let encoders_to_try = [
        HwEncoder::NvencHevc,
        HwEncoder::QsvHevc,
        HwEncoder::VaapiHevc,
        HwEncoder::AmfHevc,
    ];

    for encoder in encoders_to_try {
        if test_encoder(ffmpeg_path, encoder) {
            println!("[FFmpegEncoder] Detected hardware encoder: {:?}", encoder);
            return encoder;
        }
    }

    println!("[FFmpegEncoder] No hardware encoder detected, using software (libx265)");
    HwEncoder::Software
}

/// Test if an encoder is available by running FFmpeg with a minimal test
fn test_encoder(ffmpeg_path: &Path, encoder: HwEncoder) -> bool {
    let mut cmd = Command::new(ffmpeg_path);
    cmd.arg("-hide_banner").args(["-loglevel", "error"]);

    // Add global args (device init) for the test
    for arg in encoder.global_args() {
        cmd.arg(arg);
    }

    cmd.arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("nullsrc=s=1280x720:d=1")
        .arg("-frames:v")
        .arg("1");

    // Add format conversion args for the test
    for arg in encoder.format_args() {
        cmd.arg(arg);
    }

    cmd.arg("-c:v")
        .arg(encoder.codec_name())
        .arg("-f")
        .arg("null")
        .arg("-");

    // Hide console window on Windows
    configure_command(&mut cmd);

    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

impl FFmpegEncoder {
    pub fn new(
        ffmpeg_path: &Path,
        output_path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        quality: u8,
    ) -> std::io::Result<Self> {
        // Auto-detect best encoder
        let encoder = detect_hw_encoder(ffmpeg_path);

        let mut cmd = Command::new(ffmpeg_path);

        // 1. Global Args (Device Init)
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        for arg in encoder.global_args() {
            cmd.arg(arg);
        }

        // 2. Input Format
        cmd.args(["-f", "rawvideo"])
            .args(["-pixel_format", "bgra"])
            .args(["-video_size", &format!("{}x{}", width, height)])
            .args(["-framerate", &fps.to_string()])
            .args(["-i", "-"]); // Read from stdin

        // 3. Filter / Pixel Format (Critical for VAAPI)
        for arg in encoder.format_args() {
            cmd.arg(arg);
        }

        // 4. Codec & Output
        cmd.args(["-c:v", encoder.codec_name()]);
        for arg in encoder.quality_args(quality, fps) {
            cmd.arg(arg);
        }

        cmd.arg("-y")
            .arg("-movflags")
            .arg("+faststart")
            .arg(output_path.to_str().unwrap_or("output.mp4"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        configure_command(&mut cmd);

        let process = cmd.spawn()?;

        let (sender, receiver) = mpsc::sync_channel::<WriterMessage>(MAX_FRAME_BUFFER);
        let (recycle_sender, recycle_receiver) = mpsc::channel();

        let writer_thread =
            thread::spawn(move || writer_thread_fn(process, receiver, recycle_sender));

        Ok(Self {
            sender: Some(sender),
            recycle_receiver,
            writer_thread: Some(writer_thread),
            width,
            height,
        })
    }

    /// Get a buffer from the pool or create a new one
    pub fn get_buffer(&self) -> Vec<u8> {
        if let Ok(mut buffer) = self.recycle_receiver.try_recv() {
            buffer.clear();
            buffer
        } else {
            let capacity = (self.width * self.height * 4) as usize;
            Vec::with_capacity(capacity)
        }
    }

    /// Write a single frame of BGRA data
    pub fn write_frame(&mut self, frame_data: Vec<u8>) -> std::io::Result<()> {
        let expected_size = (self.width * self.height * 4) as usize;
        if frame_data.len() != expected_size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Invalid frame size: expected {}, got {}",
                    expected_size,
                    frame_data.len()
                ),
            ));
        }

        if let Some(ref sender) = self.sender {
            sender.send(WriterMessage::Frame(frame_data)).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Writer thread has stopped")
            })?;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> std::io::Result<()> {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(WriterMessage::Finish);
        }
        if let Some(handle) = self.writer_thread.take() {
            match handle.join() {
                Ok(result) => result?,
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Thread panicked",
                    ))
                }
            }
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> std::io::Result<()> {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(WriterMessage::Finish);
        }
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
        Ok(())
    }
}

impl Drop for FFmpegEncoder {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }
}

fn writer_thread_fn(
    mut process: Child,
    receiver: mpsc::Receiver<WriterMessage>,
    recycle_sender: Sender<Vec<u8>>,
) -> std::io::Result<()> {
    let mut stdin = process.stdin.take();

    if stdin.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to open stdin for FFmpeg process",
        ));
    }

    loop {
        match receiver.recv() {
            Ok(WriterMessage::Frame(data)) => {
                if let Some(ref mut stdin_pipe) = stdin {
                    if let Err(e) = stdin_pipe.write_all(&data) {
                        eprintln!("[FFmpegEncoder] Write error: {}", e);
                        break;
                    }
                    let _ = recycle_sender.send(data);
                }
            }
            Ok(WriterMessage::Finish) => break,
            Err(_) => {
                let _ = process.kill();
                return Ok(());
            }
        }
    }

    drop(stdin);

    let output = process.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("FFmpeg error: {}", stderr),
        ));
    }

    Ok(())
}

fn configure_command(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}
