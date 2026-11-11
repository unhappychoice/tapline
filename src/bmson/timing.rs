use super::model::Bmson;
use anyhow::{ensure, Result};

pub struct Timeline {
    points: Vec<(u32, f64, f64, f64)>,
}

impl Timeline {
    pub fn new(data: &Bmson, lead: f64) -> Result<Self> {
        ensure!(lead.is_finite() && lead >= 0.0, "Invalid lead-in");
        ensure!(data.info.init_bpm > 0.0, "Invalid initial BPM");
        ensure!(data.bpm_events.iter().all(|e| e.bpm > 0.0), "Invalid BPM");
        let resolution = data.info.resolution.filter(|&r| r > 0).unwrap_or(240) as f64;
        let mut pulses = vec![0];
        pulses.extend(data.bpm_events.iter().map(|e| e.y));
        pulses.extend(data.stop_events.iter().map(|e| e.y));
        pulses.sort_unstable();
        pulses.dedup();
        let mut points = Vec::new();
        let (mut previous, mut time, mut step) =
            (0, lead, 60_000.0 / data.info.init_bpm / resolution);
        for y in pulses {
            time += (y - previous) as f64 * step;
            let arrival = time;
            if let Some(event) = data.bpm_events.iter().rev().find(|e| e.y == y) {
                step = 60_000.0 / event.bpm / resolution;
            }
            time += data
                .stop_events
                .iter()
                .filter(|e| e.y == y)
                .map(|e| e.duration as f64 * step)
                .sum::<f64>();
            ensure!(time.is_finite() && step.is_finite(), "Timing overflow");
            points.push((y, arrival, time, step));
            previous = y;
        }
        Ok(Self { points })
    }

    pub fn time(&self, pulse: u64) -> f64 {
        let i = self.points.partition_point(|p| u64::from(p.0) <= pulse) - 1;
        let (y, arrival, departure, step) = self.points[i];
        if pulse == u64::from(y) {
            arrival
        } else {
            departure + (pulse - u64::from(y)) as f64 * step
        }
    }
}
