use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Mutex,
    },
    thread::{self, JoinHandle},
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Encoder {
    Nvenc,
    Vaapi,
    Amf,
    Qsv,
    Soft,
}

#[cfg(windows)]
const HARDWARE_ENCODERS: &[Encoder] = &[Encoder::Nvenc, Encoder::Qsv, Encoder::Amf];
#[cfg(not(windows))]
const HARDWARE_ENCODERS: &[Encoder] = &[Encoder::Nvenc, Encoder::Qsv, Encoder::Vaapi, Encoder::Amf];

impl Encoder {
    fn codec(&self) -> &'static str {
        match self {
            Self::Nvenc => "hevc_nvenc",
            Self::Vaapi => "hevc_vaapi",
            Self::Qsv => "hevc_qsv",
            Self::Amf => "hevc_amf",
            Self::Soft => "libx265",
        }
    }

    fn args(&self, q: u8, fps: u32) -> Vec<String> {
        let q = q.to_string();
        let lookahead = (fps / 4).to_string();
        let args: &[&str] = match self {
            Self::Nvenc => &[
                "-preset",
                "p7",
                "-tune",
                "hq",
                "-rc",
                "constqp",
                "-qp",
                &q,
                "-spatial-aq",
                "1",
                "-temporal-aq",
                "1",
                "-rc-lookahead",
                &lookahead,
            ],
            Self::Vaapi => &["-rc_mode", "CQP", "-qp", &q],
            Self::Amf => &[
                "-quality", "quality", "-rc", "cqp", "-qp_i", &q, "-qp_p", &q,
            ],
            Self::Qsv => &[
                "-preset",
                "veryslow",
                "-global_quality",
                &q,
                "-look_ahead",
                "1",
            ],
            Self::Soft => &["-crf", &q, "-preset", "medium"],
        };
        args.iter().map(|s| s.to_string()).collect()
    }
}

fn ffmpeg_command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// The first hardware encoder this ffmpeg can open, probed once per ffmpeg path
fn pick_encoder(path: &Path) -> Encoder {
    static PROBED: Mutex<Option<(PathBuf, Encoder)>> = Mutex::new(None);
    let mut probed = PROBED.lock().unwrap();
    if let Some((_, encoder)) = probed.as_ref().filter(|(p, _)| p == path) {
        return *encoder;
    }

    let encoder = HARDWARE_ENCODERS
        .iter()
        .copied()
        .find(|e| {
            ffmpeg_command(path)
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "nullsrc=s=1280x720:d=1",
                    "-frames:v",
                    "1",
                ])
                .args(["-c:v", e.codec(), "-f", "null", "-"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
        .unwrap_or(Encoder::Soft);
    *probed = Some((path.to_path_buf(), encoder));
    encoder
}

/// Pipes raw BGRA frames into an ffmpeg process on a writer thread
pub struct FFmpegEncoder {
    frames: Option<SyncSender<Vec<u8>>>,
    recycled: Receiver<Vec<u8>>,
    writer: Option<JoinHandle<io::Result<()>>>,
}

impl FFmpegEncoder {
    pub fn new(path: &Path, out: &Path, w: u32, h: u32, fps: u32, q: u8) -> io::Result<Self> {
        let enc = pick_encoder(path);

        let mut cmd = ffmpeg_command(path);
        cmd.args(["-f", "rawvideo", "-pixel_format", "bgra"]).args([
            "-video_size",
            &format!("{w}x{h}"),
            "-framerate",
            &fps.to_string(),
            "-i",
            "-",
        ]);
        if enc == Encoder::Vaapi {
            cmd.args(["-vf", "format=nv12,hwupload"]);
        }
        if enc == Encoder::Qsv {
            cmd.args(["-pix_fmt", "nv12"]);
        }
        cmd.args(["-c:v", enc.codec()])
            .args(enc.args(q, fps))
            .args(["-y", "-movflags", "+faststart"])
            .arg(out);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut proc = cmd.spawn()?;
        let mut stdin = proc.stdin.take().unwrap();
        let mut stderr = proc.stderr.take().unwrap();
        // Drained concurrently so ffmpeg can never block on a full stderr pipe
        let log = thread::spawn(move || {
            let mut log = String::new();
            let _ = stderr.read_to_string(&mut log);
            log
        });

        let (frames, frames_rx) = mpsc::sync_channel::<Vec<u8>>(4);
        let (recycle, recycled) = mpsc::channel();
        let writer = thread::spawn(move || {
            let mut result = Ok(());
            for frame in frames_rx {
                if let Err(e) = stdin.write_all(&frame) {
                    result = Err(e);
                    break;
                }
                let _ = recycle.send(frame);
            }
            drop(stdin);

            let status = proc.wait()?;
            let log = log.join().unwrap_or_default();
            if status.success() {
                return result;
            }
            let last_lines: Vec<_> = log.lines().rev().take(3).collect();
            let message = last_lines.into_iter().rev().collect::<Vec<_>>().join("\n");
            Err(io::Error::other(if message.is_empty() {
                format!("ffmpeg exited with {status}")
            } else {
                message
            }))
        });

        Ok(Self {
            frames: Some(frames),
            recycled,
            writer: Some(writer),
        })
    }

    pub fn write_frame(&mut self, data: &[u8]) -> io::Result<()> {
        let mut buf = self.recycled.try_recv().unwrap_or_default();
        buf.clear();
        buf.extend_from_slice(data);
        if self.frames.as_ref().is_some_and(|f| f.send(buf).is_ok()) {
            return Ok(());
        }
        // The writer stopped, so ffmpeg failed: report why
        Err(self
            .finish()
            .err()
            .unwrap_or_else(|| io::ErrorKind::BrokenPipe.into()))
    }

    /// Closes ffmpeg's input and waits for it to finish the file
    pub fn finish(&mut self) -> io::Result<()> {
        self.frames.take();
        self.writer.take().map_or(Ok(()), |w| w.join().unwrap())
    }
}

impl Drop for FFmpegEncoder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
