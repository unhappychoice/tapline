use super::*;
use crate::bga::EffectEvent;

#[test]
fn snapshots_preserve_event_order_and_independent_layer_state_when_seeking() {
    let layers = [
        BgaLayer::Base,
        BgaLayer::Overlay,
        BgaLayer::Overlay2,
        BgaLayer::Poor,
    ];
    let mut ext = Extensions::default();
    ext.argb.insert(1, [128, 10, 20, 30]);
    layers.iter().enumerate().for_each(|(index, &layer)| {
        let effects = if index % 2 == 0 {
            vec![Effect::Color(layer, 1), Effect::Opacity(layer, 64)]
        } else {
            vec![Effect::Opacity(layer, 64), Effect::Color(layer, 1)]
        };
        ext.events
            .extend(effects.into_iter().map(|effect| EffectEvent {
                time_ms: 10.0,
                effect,
            }));
    });
    ext.events.push(EffectEvent {
        time_ms: 20.0,
        effect: Effect::Color(BgaLayer::Base, 999),
    });
    ext.events.push(EffectEvent {
        time_ms: 20.0,
        effect: Effect::Switch(1),
    });
    ext.events.push(EffectEvent {
        time_ms: 30.0,
        effect: Effect::Opacity(BgaLayer::Poor, 0),
    });
    let timeline = Timeline::new(&ext);
    for (index, layer) in layers.into_iter().enumerate() {
        let expected = State {
            alpha: if index % 2 == 0 { 64 } else { 128 },
            key: Some([10, 20, 30]),
        };
        assert_eq!(timeline.state(layer, 9.0), State::default());
        assert_eq!(timeline.state(layer, 10.0), expected);
        assert_eq!(timeline.state(layer, 20.0), expected);
        assert_eq!(
            timeline.state(layer, 30.0).alpha,
            if layer == BgaLayer::Poor {
                0
            } else {
                expected.alpha
            }
        );
        assert_eq!(timeline.state(layer, 10.0), expected);
        assert_eq!(timeline.state(layer, -1.0), State::default());
    }
}

#[test]
fn unchanged_frames_search_logarithmically_and_ignore_unrelated_history() {
    for count in [1_usize, 100, 100_000] {
        let ext = Extensions {
            events: (0..count)
                .flat_map(|index| {
                    [
                        Effect::Opacity(BgaLayer::Base, 128),
                        Effect::Switch(1),
                        Effect::Color(BgaLayer::Poor, 999),
                    ]
                    .into_iter()
                    .map(move |effect| EffectEvent {
                        time_ms: index as f64,
                        effect,
                    })
                })
                .collect(),
            ..Default::default()
        };
        let timeline = Timeline::new(&ext);
        for layer in [BgaLayer::Overlay, BgaLayer::Overlay2, BgaLayer::Poor] {
            assert_eq!(timeline.state(layer, count as f64), State::default());
            assert_eq!(timeline.comparisons.get(), 0);
        }
        for now in [count as f64, count as f64 + 16.0, 0.0] {
            timeline.comparisons.set(0);
            assert_eq!(timeline.state(BgaLayer::Base, now).alpha, 128);
            assert!(timeline.comparisons.get() <= count.ilog2() as usize + 2);
        }
    }
}
