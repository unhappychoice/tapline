use super::SIZE;
use image::RgbaImage;
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{sync_channel, Receiver},
};

pub(super) struct Video {
    child: Child,
    frames: Receiver<RgbaImage>,
    current: Option<RgbaImage>,
    next: u64,
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
        std::thread::spawn(move || loop {
            let mut bytes = vec![0; (SIZE * SIZE * 4) as usize];
            if stdout.read_exact(&mut bytes).is_err() {
                break;
            }
            let frame = RgbaImage::from_raw(SIZE, SIZE, bytes).unwrap();
            if sender.send(frame).is_err() {
                break;
            }
        });
        Some(Self {
            child,
            frames,
            current: None,
            next: 0,
        })
    }

    pub fn frame(&mut self, elapsed_ms: f64) -> Option<RgbaImage> {
        let target = (elapsed_ms.max(0.0) * 30.0 / 1000.0).floor() as u64;
        while self.next <= target {
            let Ok(frame) = self.frames.try_recv() else {
                break;
            };
            self.current = Some(frame);
            self.next += 1;
        }
        self.current.clone()
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
        sender.send(RgbaImage::new(SIZE, SIZE)).unwrap();
        sender
            .send(RgbaImage::from_pixel(
                SIZE,
                SIZE,
                image::Rgba([1, 2, 3, 255]),
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
            next: 0,
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
