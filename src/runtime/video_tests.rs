use super::tests::{base_chart, opts};
use super::*;
use crate::{
    bga::{self, ImageBank},
    chart::{BgaEvent, BgaLayer, Note},
};
use std::sync::mpsc::{channel, Receiver, Sender};

const TIMEOUT: Duration = Duration::from_secs(5);

struct Decoder {
    stopping: Sender<String>,
    name: String,
    release: Option<Receiver<()>>,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.stopping.send(self.name.clone());
        if let Some(release) = self.release.take() {
            let _ = release.recv_timeout(TIMEOUT);
        }
    }
}

#[test]
fn gameplay_draws_and_handles_keys_during_decoder_transitions() {
    let times = [3000.0, 3200.0, 3400.0];
    let mut chart = base_chart(
        4,
        times
            .map(|time_ms| Note {
                time_ms,
                ..Note::default()
            })
            .to_vec(),
    );
    for (id, (time_ms, name)) in times
        .into_iter()
        .zip(["first.mp4", "skipped.mp4", "last.mp4"])
        .enumerate()
    {
        let bmp_id = id as u32;
        chart.bmp_paths.insert(bmp_id, name.into());
        chart.bga.push(BgaEvent {
            time_ms,
            bmp_id,
            layer: BgaLayer::Base,
        });
    }
    let mut images = ImageBank::load(&chart.bmp_paths, &chart.bga, &chart.bga_extensions);
    let (starting, started) = channel();
    let (stopping, stopped) = channel();
    let (allow_start, start_gate) = channel();
    let (allow_stop, stop_gate) = channel();
    let (mut start_gate, mut stop_gate) = (Some(start_gate), Some(stop_gate));
    images.set_test_decoder(move |path| {
        let name = path.to_str().unwrap().to_owned();
        starting.send(name.clone()).unwrap();
        let release = stop_gate.take();
        let color = if let Some(gate) = start_gate.take() {
            gate.recv_timeout(TIMEOUT).unwrap();
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        };
        Some((
            Decoder {
                stopping: stopping.clone(),
                name,
                release,
            },
            color,
        ))
    });

    let mut game = game::Game::new(chart);
    let bank = audio::SampleBank::silent();
    let mut state = LoopState::new(Vec::new());
    let mut frame_and_key = |now| {
        let mut output = Vec::new();
        bga::draw(&mut output, &game, &images, now, 18, 4, 16).unwrap();
        assert!(!output.is_empty());
        let stale_color = b"38;2;255;0;0";
        assert!(!output
            .windows(stale_color.len())
            .any(|bytes| bytes == stale_color));
        handle_key(
            KeyCode::Char('s'),
            KeyEventKind::Press,
            &mut game,
            &bank,
            &opts(false),
            &mut state,
            now,
        );
    };
    frame_and_key(times[0]);
    assert_eq!(started.recv_timeout(TIMEOUT).unwrap(), "first.mp4");
    frame_and_key(times[1]);
    allow_start.send(()).unwrap();
    assert_eq!(stopped.recv_timeout(TIMEOUT).unwrap(), "first.mp4");
    frame_and_key(times[1]);
    frame_and_key(times[2]);
    allow_stop.send(()).unwrap();
    assert_eq!(started.recv_timeout(TIMEOUT).unwrap(), "last.mp4");
    assert_eq!(game.perfect, 3);

    drop(images);
    assert_eq!(stopped.recv_timeout(TIMEOUT).unwrap(), "last.mp4");
}
