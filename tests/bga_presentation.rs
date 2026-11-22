use std::{
    sync::mpsc::{channel, TryRecvError},
    time::{Duration, Instant},
};
use tapline::{
    bga::{ImageBank, Presentation},
    chart::{BgaEvent, BgaLayer, Chart},
};

#[test]
fn public_presentation_accepts_a_custom_loader_and_polls_while_hidden() {
    let chart = Chart {
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

    presentation.prepare_deferred(&chart, true, |_| ImageBank::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    while active_id(&presentation.bank).is_some() {
        assert!(Instant::now() < deadline, "custom bank was not installed");
        presentation.prepare_deferred(&chart, false, Presentation::load_assets);
        std::thread::sleep(Duration::from_millis(1));
    }
}
