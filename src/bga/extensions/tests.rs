use super::*;
use crate::chart::Chart;

fn chart(events: &[(f64, u32)]) -> Chart {
    let mut chart = Chart::default();
    chart.bga_extensions.events = events
        .iter()
        .map(|&(time_ms, id)| EffectEvent {
            time_ms,
            effect: Effect::Switch(id),
        })
        .collect();
    [Some(0), Some(1), None]
        .into_iter()
        .enumerate()
        .for_each(|(i, lane)| {
            chart.bga_extensions.switches.insert(
                i as u32 + 1,
                Switch {
                    frame_ms: 100,
                    duration_ms: 0,
                    channel: String::new(),
                    lane,
                    looping: false,
                    argb: [255; 4],
                    pattern: vec![1],
                },
            );
        });
    chart
}

#[test]
fn latest_switch_preserves_ties_lanes_and_undefined_ids() {
    let chart = chart(&[(100.0, 1), (200.0, 1), (200.0, 2), (300.0, 99), (400.0, 3)]);
    let mut playback = SwitchPlayback::new(&chart.bga_extensions);
    playback.press(&chart, 0, 99.0);
    assert_eq!(playback.active, None);
    playback.press(&chart, 0, 100.0);
    assert_eq!(playback.active, Some((1, 0, 100.0)));
    playback.press(&chart, 0, 200.0);
    assert_eq!(playback.active, Some((1, 0, 100.0)));
    playback.press(&chart, 1, 200.0);
    assert_eq!(playback.active, Some((2, 1, 200.0)));
    [300.0, 400.0].into_iter().for_each(|now| {
        [0, 1].into_iter().for_each(|lane| {
            playback.press(&chart, lane, now);
            assert_eq!(playback.active, Some((2, 1, 200.0)));
        });
    });
    playback.press(&chart, 0, 150.0);
    assert_eq!(playback.active, Some((1, 0, 150.0)));
}

#[test]
fn lookup_work_depends_only_on_switch_count() {
    [0, 1024].into_iter().for_each(|switch_count| {
        let events: Vec<_> = (0..switch_count).map(|i| (i as f64, 1)).collect();
        let mut chart = chart(&events);
        let mut baseline = SwitchPlayback::new(&chart.bga_extensions);
        (0..100_000).for_each(|i| {
            chart.bga_extensions.events.push(EffectEvent {
                time_ms: i as f64,
                effect: if i % 2 == 0 {
                    Effect::Opacity(BgaLayer::Base, 128)
                } else {
                    Effect::Color(BgaLayer::Overlay, 1)
                },
            });
        });
        chart
            .bga_extensions
            .events
            .sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        let mut indexed = SwitchPlayback::new(&chart.bga_extensions);
        assert_eq!(indexed.events.len(), switch_count);
        [-1.0, 0.0, 512.0, 100_000.0].into_iter().for_each(|now| {
            baseline.comparisons = 0;
            indexed.comparisons = 0;
            baseline.press(&chart, 0, now);
            indexed.press(&chart, 0, now);
            assert_eq!(indexed.active, baseline.active);
            assert_eq!(indexed.comparisons, baseline.comparisons);
            assert!(indexed.comparisons <= 11);
            if switch_count == 0 {
                assert_eq!(indexed.comparisons, 0);
            }
        });
    });
}
