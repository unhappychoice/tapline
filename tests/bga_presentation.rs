use std::{
    path::Path,
    sync::mpsc::{channel, TryRecvError},
    time::{Duration, Instant},
};
use tapline::{
    bga::{BgaAssetInput, ImageBank, Presentation},
    chart::{BgaEvent, BgaLayer, Chart},
};

#[test]
fn public_presentation_accepts_a_custom_loader_and_polls_while_hidden() {
    let chart = Chart {
        asset_dir: "songs/example".into(),
        bmp_paths: [(1, "background.png".into())].into(),
        bga: vec![BgaEvent {
            time_ms: 0.0,
            layer: BgaLayer::Base,
            bmp_id: 1,
        }],
        ..Chart::default()
    };
    let active_id = |bank: &ImageBank| bank.active_bga_id(BgaLayer::Base, 0.0);
    assert_eq!(active_id(&Presentation::new(&chart, true).bank), Some(1));

    let mut presentation = Presentation::new(&chart, false);
    let (started, attempts) = channel();
    presentation.prepare_deferred(&chart, false, move |_| {
        started.send(()).unwrap();
        ImageBank::default()
    });
    assert_eq!(attempts.try_recv(), Err(TryRecvError::Disconnected));
    assert_eq!(active_id(&presentation.bank), Some(1));

    let (loaded_path, paths) = channel();
    presentation.prepare_deferred(&chart, true, move |assets: &BgaAssetInput| {
        loaded_path
            .send(assets.asset_dir.join(&assets.bmp_paths[&1]))
            .unwrap();
        ImageBank::default()
    });
    assert_eq!(
        paths.recv_timeout(Duration::from_secs(5)).unwrap(),
        Path::new("songs/example").join("background.png")
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while active_id(&presentation.bank).is_some() {
        assert!(Instant::now() < deadline, "custom bank was not installed");
        presentation.prepare_deferred(&chart, false, BgaAssetInput::load);
        std::thread::sleep(Duration::from_millis(1));
    }
}
