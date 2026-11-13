use crate::chart::BgaLayer;
use std::collections::HashMap;

#[derive(Default)]
pub struct Extensions {
    pub crops: HashMap<u32, Crop>,
    pub argb: HashMap<u32, [u8; 4]>,
    pub switches: HashMap<u32, Switch>,
    pub events: Vec<EffectEvent>,
    pub poor_mode: u8,
}

pub struct Crop {
    pub image: u32,
    pub rect: [i32; 6],
}

pub struct Switch {
    pub frame_ms: u32,
    pub duration_ms: u32,
    pub channel: String,
    pub lane: Option<usize>,
    pub looping: bool,
    pub argb: [u8; 4],
    pub pattern: Vec<u32>,
}

pub struct EffectEvent {
    pub time_ms: f64,
    pub effect: Effect,
}

pub enum Effect {
    Opacity(BgaLayer, u8),
    Color(BgaLayer, u32),
    Switch(u32),
}

#[derive(Default)]
pub struct SwitchPlayback {
    pub active: Option<(u32, usize, f64)>,
    pub held: bool,
    events: Vec<(f64, u32)>,
    #[cfg(test)]
    comparisons: usize,
}

impl SwitchPlayback {
    /// Extension events must be time-sorted and stay fixed during playback.
    pub fn new(ext: &Extensions) -> Self {
        Self {
            events: ext
                .events
                .iter()
                .filter_map(|event| match event.effect {
                    Effect::Switch(id) => Some((event.time_ms, id)),
                    _ => None,
                })
                .collect(),
            ..Self::default()
        }
    }

    pub fn press(&mut self, chart: &crate::chart::Chart, lane: usize, now: f64) {
        let end = self.events.partition_point(|(time, _)| {
            #[cfg(test)]
            {
                self.comparisons += 1;
            }
            *time <= now
        });
        let Some(index) = end.checked_sub(1) else {
            return;
        };
        let id = self.events[index].1;
        let Some(switch) = chart.bga_extensions.switches.get(&id) else {
            return;
        };
        if switch.lane == Some(lane) {
            self.active = Some((id, lane, now));
            self.held = true;
        }
    }
}

#[cfg(test)]
mod tests;
