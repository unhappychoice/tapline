use super::{timeline::slot, Effect, Extensions};
use crate::chart::BgaLayer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct State {
    pub alpha: u8,
    pub key: Option<[u8; 3]>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            alpha: 255,
            key: None,
        }
    }
}

#[derive(Default)]
pub(super) struct Timeline {
    layers: [Vec<(f64, State)>; 4],
    #[cfg(test)]
    comparisons: std::cell::Cell<usize>,
}

impl Timeline {
    pub(super) fn new(ext: &Extensions) -> Self {
        let mut timeline = Self::default();
        let mut states = [State::default(); 4];
        ext.events.iter().for_each(|event| {
            let (layer, alpha, key) = match event.effect {
                Effect::Opacity(layer, alpha) => (layer, alpha, states[slot(layer)].key),
                Effect::Color(layer, id) => match ext.argb.get(&id) {
                    Some(&[a, r, g, b]) => (layer, a, Some([r, g, b])),
                    None => return,
                },
                Effect::Switch(_) => return,
            };
            let state = State { alpha, key };
            states[slot(layer)] = state;
            timeline.layers[slot(layer)].push((event.time_ms, state));
        });
        timeline
    }

    pub(super) fn state(&self, layer: BgaLayer, now_ms: f64) -> State {
        let events = &self.layers[slot(layer)];
        let end = events.partition_point(|(time, _)| {
            #[cfg(test)]
            self.comparisons.set(self.comparisons.get() + 1);
            *time <= now_ms
        });
        end.checked_sub(1).map(|i| events[i].1).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
