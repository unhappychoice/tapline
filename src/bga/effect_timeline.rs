use super::{timeline::slot, Effect, Extensions};
use crate::chart::BgaLayer;

pub(super) type Timeline = super::timeline::Timeline<State>;

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
            timeline.push(layer, event.time_ms, state);
        });
        timeline
    }

    pub(super) fn state(&self, layer: BgaLayer, now_ms: f64) -> State {
        self.latest(layer, now_ms).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
