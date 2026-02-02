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

    fn global_args(&self) -> Vec<&'static str> {
        match self {
            HwEncoder::VaapiHevc => vec![
                "-init_hw_device",
                "vaapi=va:/dev/dri/renderD128",
                "-filter_hw_device",
                "va",
            ],
            _ => vec![],
        }
    }

    fn format_args(&self) -> Vec<&'static str> {
        match self {
            HwEncoder::VaapiHevc => vec!["-vf", "format=nv12,hwupload"],
            _ => vec!["-pix_fmt", "yuv420p"],
        }
    }

    fn quality_args(&self) -> Vec<&'static str> {
        match self {
            // NVENC: Use slowest preset for best quality, CQ mode
            HwEncoder::NvencHevc => vec![
                "-preset", "p7", // Slowest = best quality
                "-tune", "hq", // High quality tuning
                "-rc", "vbr", // Variable bitrate
                "-cq", "22", // Quality level (like CRF)
                "-b:v", "0", // Let CQ control quality
            ],
            // vaaaapi
            HwEncoder::VaapiHevc => vec![
                "-rc_mode", "CQP", "-qp",
                "22",
                // "-compression_level", "1", Because 1 represents prioritizing speed over quality, the CQP mode embodies the “quality-first” logic for this hardware.
            ],
            // AMF: Quality preset with CQP mode
            HwEncoder::AmfHevc => vec![
                "-quality", "quality", "-rc", "cqp", "-qp_i", "22", "-qp_p", "22",
            ],
            // QSV: Veryslow for best quality
            HwEncoder::QsvHevc => vec!["-preset", "veryslow", "-global_quality", "22"],
            // Software: Best quality settings
            HwEncoder::Software => vec!["-crf", "22", "-preset", "medium"],
        }
    }
}

/// FFmpeg video encoder with async writing
///
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
        .arg("nullsrc=s=1280x720:d=0.1");

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
    /// Create a new FFmpeg encoder process
    ///
    /// # Arguments
    /// * `ffmpeg_path` - Path to ffmpeg.exe
    /// * `output_path` - Output video file path
    /// * `width` - Video width in pixels
    /// * `height` - Video height in pixels
    /// * `fps` - Frames per second
    pub fn new(
        ffmpeg_path: &Path,
        output_path: &Path,
        width: u32,
        height: u32,
        fps: u32,
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
        for arg in encoder.quality_args() {
            cmd.arg(arg);
        }

        cmd.arg("-y")
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
