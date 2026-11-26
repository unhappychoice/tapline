use crate::chart::{BgaEvent, BgaLayer};

#[derive(Debug, PartialEq)]
pub(super) struct Event {
    pub(super) identity: usize,
    pub(super) time_ms: f64,
    pub(super) bmp_id: u32,
}

pub(super) struct Timeline<T = Event> {
    layers: [Vec<(f64, T)>; 4],
    #[cfg(test)]
    pub(super) comparisons: std::cell::Cell<usize>,
}

impl<T> Default for Timeline<T> {
    fn default() -> Self {
        Self {
            layers: Default::default(),
            #[cfg(test)]
            comparisons: Default::default(),
        }
    }
}

impl<T> Timeline<T> {
    pub(super) fn push(&mut self, layer: BgaLayer, time_ms: f64, value: T) {
        self.layers[slot(layer)].push((time_ms, value));
    }

    pub(super) fn sort_by_time(&mut self) {
        self.layers.iter_mut().for_each(|events| {
            // Keep chart order for events at the same time.
            events.sort_by(|a, b| a.0.total_cmp(&b.0));
        });
    }

    pub(super) fn is_empty(&self) -> bool {
        self.layers.iter().all(Vec::is_empty)
    }

    pub(super) fn latest(&self, layer: BgaLayer, now_ms: f64) -> Option<&T> {
        let events = &self.layers[slot(layer)];
        let index = crate::search::latest_at_or_before(
            events,
            now_ms,
            |(time_ms, _)| *time_ms,
            || {
                #[cfg(test)]
                self.comparisons.set(self.comparisons.get() + 1);
            },
        )?;
        Some(&events[index].1)
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

impl Timeline<Event> {
    pub(super) fn new(events: &[BgaEvent]) -> Self {
        let mut timeline = Self::default();
        events.iter().enumerate().for_each(|(identity, event)| {
            timeline.push(
                event.layer,
                event.time_ms,
                Event {
                    identity,
                    time_ms: event.time_ms,
                    bmp_id: event.bmp_id,
                },
            );
        });
        timeline.sort_by_time();
        timeline
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
        let timeline = Timeline::<Event>::new(&events);
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
            let timeline = Timeline::<Event>::new(&events);
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
