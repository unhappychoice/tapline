use super::SIZE;
use image::RgbaImage;
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{sync_channel, Receiver},
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

type DecodedFrame = (u64, Frame, bool);

pub(super) struct Video {
    frames: Receiver<DecodedFrame>,
    current: Option<DecodedFrame>,
    target: Arc<AtomicU64>,
}

impl Video {
    pub fn open(path: &Path) -> Option<(Decoder, Self)> {
        Self::with_decoder(path, Path::new("ffmpeg"))
    }

    fn with_decoder(path: &Path, decoder: &Path) -> Option<(Decoder, Self)> {
        let mut child = Decoder(Command::new(decoder)
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args([
                "-map", "0:v:0", "-an", "-vf",
                "setpts=PTS-STARTPTS,fps=30,scale=64:64:force_original_aspect_ratio=decrease,format=rgba,pad=64:64:(ow-iw)/2:(oh-ih)/2:color=black@0",
                "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .ok()?);
        let mut stdout = child.0.stdout.take()?;
        let (sender, frames) = sync_channel(2);
        let target = Arc::new(AtomicU64::new(0));
        let requested = Arc::clone(&target);
        std::thread::Builder::new()
            .spawn(move || {
                let mut index = 0;
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
                    if index >= requested.load(Ordering::Relaxed)
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
        }
    }

    #[cfg(test)]
    pub fn test_frame(color: [u8; 4]) -> Self {
        let (sender, frames) = sync_channel(1);
        let image = RgbaImage::from_pixel(SIZE, SIZE, image::Rgba(color));
        sender.send((0, Frame(Arc::new(image)), true)).unwrap();
        Self::from_frames(frames, Arc::new(AtomicU64::new(0)))
    }

    pub fn frame(&mut self, elapsed_ms: f64) -> Option<Frame> {
        let target = (elapsed_ms.max(0.0) * 30.0 / 1000.0).floor() as u64;
        self.target.store(target, Ordering::Relaxed);
        while self
            .current
            .as_ref()
            .is_none_or(|(index, _, eof)| *index < target && !eof)
        {
            let Ok(frame) = self.frames.try_recv() else {
                break;
            };
            self.current = Some(frame);
        }
        self.current.as_ref().map(|(_, frame, _)| frame.clone())
    }
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
    fn missing_decoder_is_silently_disabled() {
        assert!(
            Video::with_decoder(Path::new("video.mp4"), Path::new("/missing/tapline-ffmpeg"))
                .is_none()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn worker_reaps_replaced_decoder_and_active_decoder_on_drop() {
        use super::super::video_worker::Worker;
        use std::{path::PathBuf, sync::mpsc::channel, time::Duration};

        let (started, processes) = channel();
        let worker = Worker::start(move |_| {
            let child = Decoder(Command::new("sleep").arg("60").spawn().unwrap());
            started.send(child.0.id()).unwrap();
            Some((child, Video::test_frame([0, 0, 0, 255])))
        })
        .unwrap();
        let request = || {
            worker.request(Some(PathBuf::from("video.mp4")));
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

        let first = Frame(Arc::new(RgbaImage::new(SIZE, SIZE)));
        sender.send((0, first.clone(), false)).unwrap();
        assert!(video.frame(0.0) == Some(first.clone()));
        assert!(video.frame(32.0) == Some(first.clone()));
        assert!(video.frame(34.0) == Some(first));

        let second = Frame(Arc::new(RgbaImage::new(SIZE, SIZE)));
        sender.send((1, second.clone(), false)).unwrap();
        assert!(video.frame(34.0) == Some(second.clone()));
        sender.send((1, second.clone(), true)).unwrap();
        drop(sender);
        assert!(video.frame(100.0) == Some(second.clone()));
        assert!(video.frame(10000.0) == Some(second));
    }

    fn video_with_frames(frames: Receiver<DecodedFrame>) -> Video {
        Video::from_frames(frames, Arc::new(AtomicU64::new(0)))
    }
}
