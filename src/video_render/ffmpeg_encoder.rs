//! FFmpeg encoder for video generation
//!
//! Handles piping raw frame data to FFmpeg for H.265 encoding.
//! Uses an async channel to decouple rendering from encoding.
//! Automatically detects and uses hardware acceleration when available.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};

/// Message types for the writer thread
enum WriterMessage {
    Frame(Vec<u8>),
    Finish,
}

/// Available hardware encoders (in priority order)
#[derive(Debug, Clone, Copy)]
enum HwEncoder {
    NvencHevc,   // NVIDIA
    AmfHevc,     // AMD
    QsvHevc,     // Intel
    Software,    // Fallback: libx265
}

impl HwEncoder {
    fn codec_name(&self) -> &'static str {
        match self {
            HwEncoder::NvencHevc => "hevc_nvenc",
            HwEncoder::AmfHevc => "hevc_amf",
            HwEncoder::QsvHevc => "hevc_qsv",
            HwEncoder::Software => "libx265",
        }
    }
    
    fn quality_args(&self) -> Vec<&'static str> {
        match self {
            // NVENC: Use slowest preset for best quality, CQ mode
            HwEncoder::NvencHevc => vec![
                "-preset", "p7",        // Slowest = best quality
                "-tune", "hq",          // High quality tuning
                "-rc", "vbr",           // Variable bitrate
                "-cq", "26",            // Quality level (like CRF)
                "-b:v", "0",            // Let CQ control quality
            ],
            // AMF: Quality preset with CQP mode
            HwEncoder::AmfHevc => vec![
                "-quality", "quality",  // Quality preset
                "-rc", "cqp",           // Constant QP mode
                "-qp_i", "26",
                "-qp_p", "26",
            ],
            // QSV: Veryslow for best quality
            HwEncoder::QsvHevc => vec![
                "-preset", "veryslow",
                "-global_quality", "26",
            ],
            // Software: Best quality settings
            HwEncoder::Software => vec![
                "-crf", "24",
                "-preset", "medium",
            ],
        }
    }
}

/// FFmpeg video encoder with async writing
///
/// Takes raw BGRA frame data and encodes it to H.265 video.
/// Uses a background thread for writing to prevent blocking the render loop.
/// Maximum number of frames to buffer before blocking
/// At 120 FPS, this is about 1 second of video
const MAX_FRAME_BUFFER: usize = 120;

pub struct FFmpegEncoder {
    sender: Option<SyncSender<WriterMessage>>,
    writer_thread: Option<JoinHandle<std::io::Result<()>>>,
    width: u32,
    height: u32,
}

/// Detect available hardware encoder by testing FFmpeg
fn detect_hw_encoder(ffmpeg_path: &Path) -> HwEncoder {
    let encoders_to_try = [
        HwEncoder::NvencHevc,
        HwEncoder::AmfHevc,
        HwEncoder::QsvHevc,
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
    cmd.arg("-hide_banner")
       .args(["-loglevel", "error"])
       .arg("-f").arg("lavfi")
       .arg("-i").arg("nullsrc=s=1280x720:d=0.1")
       .arg("-c:v").arg(encoder.codec_name())
       .arg("-f").arg("null")
       .arg("-");
    
    // Hide console window on Windows
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    
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
        cmd
            // Hide banner
            .args(["-hide_banner", "-loglevel", "error"])
            // Input format
            .args(["-f", "rawvideo"])
            .args(["-pixel_format", "bgra"])
            .args(["-video_size", &format!("{}x{}", width, height)])
            .args(["-framerate", &fps.to_string()])
            .args(["-i", "-"]) // Read from stdin
            // Output encoding
            .args(["-c:v", encoder.codec_name()]);
        
        // Add encoder-specific quality args
        for arg in encoder.quality_args() {
            cmd.arg(arg);
        }
        cmd
            .args(["-pix_fmt", "yuv420p"])
            // Overwrite output file
            .arg("-y")
            .arg(output_path.to_str().unwrap_or("output.mp4"))
            // Stdin/stdout
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        
        // Hide console window on Windows
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        
        let process = cmd.spawn()?;

        // Create bounded channel for async frame writing (limits memory usage)
        let (sender, receiver) = mpsc::sync_channel::<WriterMessage>(MAX_FRAME_BUFFER);

        // Spawn writer thread
        let writer_thread = thread::spawn(move || {
            writer_thread_fn(process, receiver)
        });

        Ok(Self {
            sender: Some(sender),
            writer_thread: Some(writer_thread),
            width,
            height,
        })
    }

    /// Write a single frame of BGRA data (async - returns immediately)
    ///
    /// The frame data must be exactly width * height * 4 bytes.
    pub fn write_frame(&mut self, frame_data: &[u8]) -> std::io::Result<()> {
        let expected_size = (self.width * self.height * 4) as usize;
        if frame_data.len() != expected_size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Invalid frame size: expected {} bytes, got {}",
                    expected_size,
                    frame_data.len()
                ),
            ));
        }

        if let Some(ref sender) = self.sender {
            // Send frame to writer thread (non-blocking for the render loop)
            sender.send(WriterMessage::Frame(frame_data.to_vec()))
                .map_err(|_| std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Writer thread has stopped"
                ))?;
        }
        Ok(())
    }

    /// Finish encoding and wait for FFmpeg to complete
    pub fn finish(&mut self) -> std::io::Result<()> {
        // Send finish signal
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(WriterMessage::Finish);
        }

        // Wait for writer thread to complete
        if let Some(handle) = self.writer_thread.take() {
            match handle.join() {
                Ok(result) => result?,
                Err(_) => return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Writer thread panicked"
                )),
            }
        }

        Ok(())
    }

    /// Cancel encoding but let FFmpeg finish muxing (produces valid partial video)
    pub fn cancel(&mut self) -> std::io::Result<()> {
        // Send finish signal to complete muxing properly
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(WriterMessage::Finish);
        }
        
        // Wait for writer thread to complete muxing
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
        
        Ok(())
    }
}

impl Drop for FFmpegEncoder {
    fn drop(&mut self) {
        // Signal writer thread to stop
        self.sender = None;
        
        // Wait for writer thread
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Writer thread function - handles actual writing to FFmpeg stdin
fn writer_thread_fn(
    mut process: Child,
    receiver: mpsc::Receiver<WriterMessage>,
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
                }
            }
            Ok(WriterMessage::Finish) => {
                // Normal finish - close stdin and wait for FFmpeg
                break;
            }
            Err(_) => {
                // Sender dropped (cancel or encoder dropped) - kill process
                let _ = process.kill();
                return Ok(());
            }
        }
    }
    
    // Close stdin to signal end of input
    drop(stdin);
    
    // Wait for FFmpeg to complete
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
