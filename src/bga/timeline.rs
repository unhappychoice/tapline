use crate::chart::{BgaEvent, BgaLayer};

#[derive(Default)]
pub(super) struct Timeline {
    layers: [Vec<(f64, usize)>; 3],
    #[cfg(test)]
    comparisons: std::cell::Cell<usize>,
}

impl Timeline {
    pub(super) fn new(events: &[BgaEvent]) -> Self {
        let mut timeline = Self::default();
        events.iter().enumerate().for_each(|(index, event)| {
            timeline.layers[slot(event.layer)].push((event.time_ms, index));
        });
        timeline.layers.iter_mut().for_each(|events| {
            events.sort_by(|a, b| a.0.total_cmp(&b.0));
        });
        timeline
    }

    pub(super) fn latest(&self, layer: BgaLayer, now_ms: f64) -> Option<usize> {
        let events = &self.layers[slot(layer)];
        let end = events.partition_point(|(time_ms, _)| {
            #[cfg(test)]
            self.comparisons.set(self.comparisons.get() + 1);
            *time_ms <= now_ms
        });
        end.checked_sub(1).map(|index| events[index].1)
    }
}

pub(super) fn slot(layer: BgaLayer) -> usize {
    match layer {
        BgaLayer::Base => 0,
        BgaLayer::Overlay => 1,
        BgaLayer::Poor => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_each_layer_at_boundaries_and_when_seeking_backwards() {
        let events = [
            (100.0, BgaLayer::Base),
            (0.0, BgaLayer::Poor),
            (100.0, BgaLayer::Overlay),
            (0.0, BgaLayer::Base),
            (100.0, BgaLayer::Base),
            (200.0, BgaLayer::Poor),
            (200.0, BgaLayer::Overlay),
        ]
        .map(|(time_ms, layer)| BgaEvent {
            time_ms,
            layer,
            bmp_id: 1,
        });
        let timeline = Timeline::new(&events);
        for (layer, first, last) in [
            (BgaLayer::Base, 3, 4),
            (BgaLayer::Overlay, 2, 6),
            (BgaLayer::Poor, 1, 5),
        ] {
            assert_eq!(timeline.latest(layer, -1.0), None);
            assert_eq!(timeline.latest(layer, 200.0), Some(last));
            assert_eq!(
                timeline.latest(layer, 100.0),
                Some(if layer == BgaLayer::Base { 4 } else { first })
            );
            assert_eq!(timeline.latest(layer, events[first].time_ms), Some(first));
        }
    }

    #[test]
    fn missing_layers_do_no_comparisons_and_future_events_cost_logarithmic_work() {
        for count in [1_usize, 100, 100_000] {
            let events: Vec<_> = (0..count)
                .map(|index| BgaEvent {
                    time_ms: index as f64 * 1000.0,
                    layer: BgaLayer::Base,
                    bmp_id: 1,
                })
                .collect();
            let timeline = Timeline::new(&events);
            for layer in [BgaLayer::Overlay, BgaLayer::Poor] {
                assert_eq!(timeline.latest(layer, 0.0), None);
                assert_eq!(timeline.comparisons.get(), 0);
            }
            for _ in 0..2 {
                timeline.comparisons.set(0);
                assert_eq!(timeline.latest(BgaLayer::Base, 0.0), Some(0));
                assert!(timeline.comparisons.get() <= count.ilog2() as usize + 2);
            }
        }
    }
}
