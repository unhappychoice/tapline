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

pub(super) struct Video {
    child: Child,
    frames: Receiver<(u64, RgbaImage, bool)>,
    current: Option<(u64, RgbaImage, bool)>,
    target: Arc<AtomicU64>,
}

impl Video {
    pub fn open(path: &Path) -> Option<Self> {
        Self::with_decoder(path, Path::new("ffmpeg"))
    }

    fn with_decoder(path: &Path, decoder: &Path) -> Option<Self> {
        let mut child = Command::new(decoder)
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
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let (sender, frames) = sync_channel(2);
        let target = Arc::new(AtomicU64::new(0));
        let requested = Arc::clone(&target);
        std::thread::spawn(move || {
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
                let frame = RgbaImage::from_raw(SIZE, SIZE, bytes).unwrap();
                if index >= requested.load(Ordering::Relaxed)
                    && sender.send((index, frame.clone(), false)).is_err()
                {
                    break;
                }
                last = Some((index, frame));
                index += 1;
            }
        });
        Some(Self {
            child,
            frames,
            current: None,
            target,
        })
    }

    pub fn frame(&mut self, elapsed_ms: f64) -> Option<RgbaImage> {
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
        self.current
            .as_ref()
            .filter(|(index, _, eof)| *index == target || *eof)
            .map(|(_, frame, _)| frame.clone())
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

    #[test]
    fn frames_follow_elapsed_time_and_hold_at_eof() {
        let (sender, frames) = sync_channel(2);
        sender.send((0, RgbaImage::new(SIZE, SIZE), false)).unwrap();
        sender
            .send((
                1,
                RgbaImage::from_pixel(SIZE, SIZE, image::Rgba([1, 2, 3, 255])),
                true,
            ))
            .unwrap();
        drop(sender);
        let child = Command::new("rustc")
            .arg("--version")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut video = Video {
            child,
            frames,
            current: None,
            target: Arc::new(AtomicU64::new(0)),
        };
        assert_eq!(video.frame(0.0).unwrap().get_pixel(0, 0).0, [0; 4]);
        assert_eq!(video.frame(32.0).unwrap().get_pixel(0, 0).0, [0; 4]);
        assert_eq!(video.frame(34.0).unwrap().get_pixel(0, 0).0, [1, 2, 3, 255]);
        assert_eq!(
            video.frame(10000.0).unwrap().get_pixel(0, 0).0,
            [1, 2, 3, 255]
        );
    }
}
