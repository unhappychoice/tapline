use super::*;
use proptest::prelude::*;

#[test]
fn absent_target_and_future_events_have_bounded_search_cost() {
    let chart = Chart {
        volume_events: (0..65_536)
            .map(|i| VolumeEvent {
                time_ms: i as f64 + 1000.0,
                target: VolumeTarget::Key,
                volume: 64,
            })
            .collect(),
        ..Default::default()
    };
    let index = PlaybackVolume::new(&chart);
    assert_eq!(index.gain(VolumeTarget::Bgm, 0.0), 1.0);
    assert_eq!(index.visits.get(), 0);
    assert_eq!(index.gain(VolumeTarget::Key, 0.0), 1.0);
    assert!(index.visits.get() <= 18);
    assert_eq!(index.gain(VolumeTarget::Key, 1000.0), 64.0 / 255.0);
    assert!(index.visits.get() <= 18);
}

#[test]
fn simultaneous_events_keep_the_last_volume_for_each_target() {
    let chart = Chart {
        vol_wav: Some(50),
        volume_events: [
            (VolumeTarget::Bgm, 64),
            (VolumeTarget::Key, 32),
            (VolumeTarget::Bgm, 128),
            (VolumeTarget::Key, 255),
        ]
        .map(|(target, volume)| VolumeEvent {
            time_ms: 1000.0,
            target,
            volume,
        })
        .to_vec(),
        ..Default::default()
    };
    let index = PlaybackVolume::new(&chart);
    assert_eq!(index.gain(VolumeTarget::Bgm, 999.0), 0.5);
    assert_eq!(index.gain(VolumeTarget::Key, 999.0), 0.5);
    assert_eq!(index.gain(VolumeTarget::Bgm, 1000.0), 0.5 * 128.0 / 255.0);
    assert_eq!(index.gain(VolumeTarget::Key, 1000.0), 0.5);
}

proptest! {
    #[test]
    fn indexed_gain_matches_reverse_scan(
        events in prop::collection::vec((0u16..1000, any::<bool>(), 1u8..=255), 0..500),
        time in 0u16..1100,
        master in 0u8..200,
    ) {
        let mut chart = Chart { vol_wav: Some(master), ..Default::default() };
        chart.volume_events = events.into_iter().map(|(time, key, volume)| VolumeEvent {
            time_ms: time as f64,
            target: if key { VolumeTarget::Key } else { VolumeTarget::Bgm },
            volume,
        }).collect();
        chart.volume_events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        let index = PlaybackVolume::new(&chart);
        for target in [VolumeTarget::Bgm, VolumeTarget::Key] {
            let volume = chart.volume_events.iter().rev()
                .find(|event| event.target == target && event.time_ms <= time as f64)
                .map_or(255, |event| event.volume);
            let expected = master as f32 / 100.0 * volume as f32 / 255.0;
            prop_assert_eq!(index.gain(target, time as f64), expected);
        }
    }
}
