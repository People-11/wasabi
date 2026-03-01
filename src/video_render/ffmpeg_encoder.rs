use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

type BufferPool = Arc<Mutex<Vec<Vec<u8>>>>;
enum Msg { Frame(Vec<u8>), Finish }

#[derive(Debug, Clone, Copy, PartialEq)]
enum Encoder { Nvenc, Vaapi, Amf, Qsv, Soft }

impl Encoder {
    fn codec(&self) -> &'static str { match self { Self::Nvenc => "hevc_nvenc", Self::Vaapi => "hevc_vaapi", Self::Qsv => "hevc_qsv", Self::Amf => "hevc_amf", Self::Soft => "libx265" } }
    fn args(&self, q: u8, fps: u32) -> Vec<String> {
        let q = q.to_string();
        match self {
            Self::Nvenc => vec!["-preset", "p7", "-tune", "hq", "-rc", "constqp", "-qp", &q, "-spatial-aq", "1", "-temporal-aq", "1", "-rc-lookahead", &(fps/4).to_string()].into_iter().map(str::to_string).collect(),
            Self::Vaapi => vec!["-rc_mode", "CQP", "-qp", &q].into_iter().map(str::to_string).collect(),
            Self::Amf => vec!["-quality", "quality", "-rc", "cqp", "-qp_i", &q, "-qp_p", &q].into_iter().map(str::to_string).collect(),
            Self::Qsv => vec!["-preset", "veryslow", "-global_quality", &q, "-look_ahead", "1"].into_iter().map(str::to_string).collect(),
            Self::Soft => vec!["-crf", &q, "-preset", "medium"].into_iter().map(str::to_string).collect(),
        }
    }
}

pub struct FFmpegEncoder {
    sender: Option<SyncSender<Msg>>, pool: BufferPool,
    thread: Option<JoinHandle<std::io::Result<()>>>,
    frame_size: usize,
}

impl FFmpegEncoder {
    pub fn new(path: &Path, out: &Path, w: u32, h: u32, fps: u32, q: u8) -> std::io::Result<Self> {
        let enc = [Encoder::Nvenc, Encoder::Qsv, Encoder::Vaapi, Encoder::Amf].into_iter()
            .find(|&e| {
                let mut c = Command::new(path);
                c.args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "nullsrc=s=1280x720:d=1", "-frames:v", "1", "-c:v", e.codec(), "-f", "null", "-"]);
                #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); }
                c.status().map(|s| s.success()).unwrap_or(false)
            })
            .unwrap_or(Encoder::Soft);
        
        let mut cmd = Command::new(path);
        cmd.args(["-hide_banner", "-loglevel", "error", "-f", "rawvideo", "-pixel_format", "bgra", "-video_size", &format!("{w}x{h}"), "-framerate", &fps.to_string(), "-i", "-"]);
        if enc == Encoder::Vaapi { cmd.args(["-vf", "format=nv12,hwupload"]); }
        if enc == Encoder::Qsv { cmd.args(["-pix_fmt", "nv12"]); }
        cmd.args(["-c:v", enc.codec()]).args(enc.args(q, fps)).args(["-y", "-movflags", "+faststart", out.to_str().unwrap()]);
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(windows)] { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }

        let frame_size = (w * h * 4) as usize;
        let pool: BufferPool = Arc::new(Mutex::new(Vec::with_capacity(4)));
        for _ in 0..4 { pool.lock().unwrap().push(vec![0u8; frame_size]); }

        let mut proc = cmd.spawn()?;
        let (tx, rx) = mpsc::sync_channel::<Msg>(4);
        let pool_clone = Arc::clone(&pool);
        let thread = thread::spawn(move || {
            let mut stdin = proc.stdin.take().unwrap();
            while let Ok(msg) = rx.recv() {
                match msg {
                    Msg::Frame(d) => { stdin.write_all(&d)?; pool_clone.lock().unwrap().push(d); }
                    Msg::Finish => break,
                }
            }
            drop(stdin);
            if proc.wait()?.success() { Ok(()) } else { Err(std::io::Error::new(std::io::ErrorKind::Other, "FFmpeg failed")) }
        });

        Ok(Self { sender: Some(tx), pool, thread: Some(thread), frame_size })
    }

    pub fn write_frame_slice(&mut self, data: &[u8]) -> std::io::Result<()> {
        let mut buf = self.pool.lock().unwrap().pop().unwrap_or_else(|| vec![0u8; self.frame_size]);
        buf.copy_from_slice(data);
        self.sender.as_ref().unwrap().send(Msg::Frame(buf)).map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"))
    }

    pub fn finish(&mut self) -> std::io::Result<()> {
        if let Some(s) = self.sender.take() { let _ = s.send(Msg::Finish); }
        self.thread.take().map(|t| t.join().unwrap()).unwrap_or(Ok(()))
    }
}

impl Drop for FFmpegEncoder { fn drop(&mut self) { let _ = self.finish(); } }
