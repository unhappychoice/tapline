use super::SIZE;
use image::RgbaImage;
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{sync_channel, Receiver, TryRecvError},
        Arc,
    },
};

pub(super) struct Decoder(Child);

// Each decoded frame owns a distinct allocation that is shared through EOF.
#[derive(Clone)]
pub(super) struct Frame(pub(super) Arc<RgbaImage>);

impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Frame {}

const SEEK_THRESHOLD: u64 = 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Start {
    #[default]
    Beginning,
    Frame(u64),
    End,
}

impl Start {
    pub fn at(index: u64) -> Self {
        if index > SEEK_THRESHOLD {
            Self::Frame(index)
        } else {
            Self::Beginning
        }
    }

    pub fn frame_index(self) -> u64 {
        match self {
            Self::Frame(index) => index,
            _ => 0,
        }
    }
}

pub(super) fn frame_index(elapsed_ms: f64) -> u64 {
    (elapsed_ms.max(0.0) * 30.0 / 1000.0).floor() as u64
}

type DecodedFrame = (u64, Frame, bool);

pub(super) struct Video {
    frames: Receiver<DecodedFrame>,
    current: Option<DecodedFrame>,
    target: Arc<AtomicU64>,
    empty: bool,
}

impl Video {
    pub fn open(path: &Path, start: Start) -> Option<(Decoder, Self)> {
        Self::with_decoder(path, Path::new("ffmpeg"), start)
    }

    fn with_decoder(path: &Path, decoder: &Path, start: Start) -> Option<(Decoder, Self)> {
        let mut child = Decoder(
            decoder_command(path, decoder, start)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()
                .ok()?,
        );
        let mut stdout = child.0.stdout.take()?;
        let (sender, frames) = sync_channel(2);
        let first = start.frame_index();
        let target = Arc::new(AtomicU64::new(first));
        let requested = Arc::clone(&target);
        std::thread::Builder::new()
            .spawn(move || {
                let mut index = first;
                let mut last = None;
                loop {
                    let mut bytes = vec![0; (SIZE * SIZE * 4) as usize];
                    if stdout.read_exact(&mut bytes).is_err() {
                        if let Some((index, frame)) = last {
                            let _ = sender.send((index, frame, true));
                        }
                        break;
                    }
                    let frame = Frame(Arc::new(RgbaImage::from_raw(SIZE, SIZE, bytes).unwrap()));
                    if start != Start::End
                        && (index == first || index >= requested.load(Ordering::Relaxed))
                        && sender.send((index, frame.clone(), false)).is_err()
                    {
                        break;
                    }
                    last = Some((index, frame));
                    index += 1;
                }
            })
            .ok()?;
        Some((child, Self::from_frames(frames, target)))
    }

    fn from_frames(frames: Receiver<DecodedFrame>, target: Arc<AtomicU64>) -> Self {
        Self {
            frames,
            current: None,
            target,
            empty: false,
        }
    }

    pub fn is_behind(&self, target: u64) -> bool {
        self.current
            .as_ref()
            .is_some_and(|(index, _, eof)| !eof && target.saturating_sub(*index) > SEEK_THRESHOLD)
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }

    #[cfg(test)]
    pub fn reached_eof(&self) -> bool {
        self.current.as_ref().is_some_and(|(_, _, eof)| *eof)
    }

    #[cfg(test)]
    pub fn test_frame(color: [u8; 4]) -> Self {
        let (sender, frames) = sync_channel(1);
        let image = RgbaImage::from_pixel(SIZE, SIZE, image::Rgba(color));
        sender.send((0, Frame(Arc::new(image)), true)).unwrap();
        Self::from_frames(frames, Arc::new(AtomicU64::new(0)))
    }

    pub fn frame(&mut self, elapsed_ms: f64) -> Option<Frame> {
        let target = frame_index(elapsed_ms);
        self.target.store(target, Ordering::Relaxed);
        while self
            .current
            .as_ref()
            .is_none_or(|(index, _, eof)| *index < target && !eof)
        {
            match self.frames.try_recv() {
                Ok(frame) => self.current = Some(frame),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.empty = self.current.is_none();
                    break;
                }
            }
        }
        self.current.as_ref().map(|(_, frame, _)| frame.clone())
    }
}

fn decoder_command(path: &Path, decoder: &Path, start: Start) -> Command {
    let mut command = Command::new(decoder);
    if let Start::Frame(index) = start {
        // Seek on the input so FFmpeg can jump to a keyframe before decoding.
        command.args(["-ss", &format!("{:.6}", index as f64 / 30.0)]);
    }
    // Recovery scans the video stream because the container may end after the video.
    let timing = if start == Start::End {
        command.args(["-vsync", "0"]);
        ""
    } else {
        "setpts=PTS-STARTPTS,fps=30,"
    };
    command
        .args([
            "-v", "error", "-nostdin",
            "-format_whitelist", "mov,matroska,avi,mpeg,mpegvideo,mpegts,m4v",
            "-protocol_whitelist", "file",
            "-i",
        ])
        .arg(path)
        .args([
            "-map", "0:v:0", "-an", "-vf",
        ])
        .arg(format!("{timing}scale=64:64:force_original_aspect_ratio=decrease,format=rgba,pad=64:64:(ow-iw)/2:(oh-ih)/2:color=black@0"))
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    command
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disguised_playlist_is_rejected_before_demuxing() {
        let decoder = Path::new("ffmpeg");
        if Command::new(decoder).arg("-version").output().is_err() {
            eprintln!("Skipping video integration test: ffmpeg is not installed");
            return;
        }
        let path =
            std::env::temp_dir().join(format!("tapline-playlist-{}.mp4", std::process::id()));
        std::fs::write(&path, "ffconcat version 1.0\n").unwrap();
        let output = decoder_command(&path, decoder, Start::Beginning).output();
        std::fs::remove_file(path).unwrap();
        let output = output.unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        // FFmpeg checks the whitelist before calling the demuxer's header reader.
        assert!(stderr.contains("Format not on whitelist"), "{stderr}");
    }

    #[test]
    fn missing_decoder_is_silently_disabled() {
        assert!(Video::with_decoder(
            Path::new("video.mp4"),
            Path::new("/missing/tapline-ffmpeg"),
            Start::Beginning
        )
        .is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn worker_reaps_replaced_decoder_and_active_decoder_on_drop() {
        use super::super::video_worker::Worker;
        use std::{path::PathBuf, sync::mpsc::channel, time::Duration};

        let (started, processes) = channel();
        let worker = Worker::start(move |_, _| {
            let child = Decoder(Command::new("sleep").arg("60").spawn().unwrap());
            started.send(child.0.id()).unwrap();
            Some((child, Video::test_frame([0, 0, 0, 255])))
        })
        .unwrap();
        let request = || {
            worker.request(Some(PathBuf::from("video.mp4")), Start::Beginning);
            let pid = processes.recv_timeout(Duration::from_secs(5)).unwrap();
            PathBuf::from(format!("/proc/{pid}"))
        };

        let first = request();
        assert!(first.exists());
        let second = request();
        assert!(!first.exists());
        assert!(second.exists());
        drop(worker);
        assert!(!second.exists());
    }

    #[test]
    fn delayed_frames_hold_until_a_new_frame_arrives() {
        let (sender, frames) = sync_channel(2);
        let mut video = video_with_frames(frames);
        assert!(video.frame(0.0).is_none());
        assert!(video.frame(34.0).is_none());
        assert!(!video.is_behind(1800));

        let first = Frame(Arc::new(RgbaImage::new(SIZE, SIZE)));
        sender.send((0, first.clone(), false)).unwrap();
        assert!(video.frame(0.0) == Some(first.clone()));
        assert!(video.frame(32.0) == Some(first.clone()));
        assert!(video.frame(34.0) == Some(first));

        let second = Frame(Arc::new(RgbaImage::new(SIZE, SIZE)));
        sender.send((1, second.clone(), false)).unwrap();
        assert!(video.frame(34.0) == Some(second.clone()));
        assert!(!video.reached_eof());
        sender.send((1, second.clone(), true)).unwrap();
        drop(sender);
        assert!(video.frame(100.0) == Some(second.clone()));
        assert!(video.reached_eof());
        assert!(video.frame(10000.0) == Some(second));
    }

    fn video_with_frames(frames: Receiver<DecodedFrame>) -> Video {
        Video::from_frames(frames, Arc::new(AtomicU64::new(0)))
    }
}
