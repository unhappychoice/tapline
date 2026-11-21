use crate::chart::{BgaEvent, BgaLayer};

#[derive(Debug, PartialEq)]
pub(super) struct Event {
    pub(super) identity: usize,
    pub(super) time_ms: f64,
    pub(super) bmp_id: u32,
}

#[derive(Default)]
pub(super) struct Timeline {
    layers: [Vec<Event>; 4],
    #[cfg(test)]
    comparisons: std::cell::Cell<usize>,
}

impl Timeline {
    pub(super) fn new(events: &[BgaEvent]) -> Self {
        let mut timeline = Self::default();
        events.iter().enumerate().for_each(|(identity, event)| {
            timeline.layers[slot(event.layer)].push(Event {
                identity,
                time_ms: event.time_ms,
                bmp_id: event.bmp_id,
            });
        });
        timeline.layers.iter_mut().for_each(|events| {
            // Keep chart order for events at the same time.
            events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        });
        timeline
    }

    pub(super) fn is_empty(&self) -> bool {
        self.layers.iter().all(Vec::is_empty)
    }

    pub(super) fn latest(&self, layer: BgaLayer, now_ms: f64) -> Option<&Event> {
        let events = &self.layers[slot(layer)];
        let end = events.partition_point(|event| {
            #[cfg(test)]
            self.comparisons.set(self.comparisons.get() + 1);
            event.time_ms <= now_ms
        });
        end.checked_sub(1).map(|index| &events[index])
    }

    #[cfg(test)]
    pub(super) fn comparisons(&self) -> usize {
        self.comparisons.get()
    }

    #[cfg(test)]
    pub(super) fn reset_comparisons(&self) {
        self.comparisons.set(0);
    }
}

pub(super) fn slot(layer: BgaLayer) -> usize {
    match layer {
        BgaLayer::Base => 0,
        BgaLayer::Overlay => 1,
        BgaLayer::Poor => 2,
        BgaLayer::Overlay2 => 3,
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
            (50.0, BgaLayer::Overlay2),
            (200.0, BgaLayer::Overlay2),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (time_ms, layer))| BgaEvent {
            time_ms,
            layer,
            bmp_id: index as u32 + 1,
        })
        .collect::<Vec<_>>();
        let timeline = Timeline::new(&events);
        let latest = |layer, time| timeline.latest(layer, time).map(|event| event.bmp_id);
        for (layer, first, last) in [
            (BgaLayer::Base, 3, 4),
            (BgaLayer::Overlay, 2, 6),
            (BgaLayer::Poor, 1, 5),
            (BgaLayer::Overlay2, 7, 8),
        ] {
            assert_eq!(latest(layer, -1.0), None);
            assert_eq!(latest(layer, 200.0), Some(events[last].bmp_id));
            assert_eq!(
                latest(layer, 100.0),
                Some(events[if layer == BgaLayer::Base { 4 } else { first }].bmp_id)
            );
            assert_eq!(
                latest(layer, events[first].time_ms),
                Some(events[first].bmp_id)
            );
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
            for layer in [BgaLayer::Overlay, BgaLayer::Poor, BgaLayer::Overlay2] {
                assert_eq!(timeline.latest(layer, 0.0), None);
                assert_eq!(timeline.comparisons.get(), 0);
            }
            for _ in 0..2 {
                timeline.comparisons.set(0);
                assert_eq!(
                    timeline
                        .latest(BgaLayer::Base, 0.0)
                        .map(|event| event.identity),
                    Some(0)
                );
                assert!(timeline.comparisons.get() <= count.ilog2() as usize + 2);
            }
        }
    }
}
