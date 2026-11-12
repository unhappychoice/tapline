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
}

impl SwitchPlayback {
    pub fn press(&mut self, chart: &crate::chart::Chart, lane: usize, now: f64) {
        let id = chart.bga_extensions.events.iter().rev().find_map(|event| {
            if event.time_ms > now {
                return None;
            }
            match event.effect {
                Effect::Switch(id) => Some(id),
                _ => None,
            }
        });
        let Some(id) = id else {
            return;
        };
        let Some(switch) = chart.bga_extensions.switches.get(&id) else {
            return;
        };
        let channel = crate::bms::bga_switch_lane(&switch.channel, chart.lane_count);
        if channel == Some(lane) {
            self.active = Some((id, lane, now));
            self.held = true;
        }
    }
}
