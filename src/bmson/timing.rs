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
        let mut bpms: Vec<_> = data.bpm_events.iter().collect();
        let mut stops: Vec<_> = data.stop_events.iter().collect();
        bpms.sort_by_key(|e| e.y);
        stops.sort_by_key(|e| e.y);
        Self::from_events(
            bpms.into_iter(),
            stops.into_iter(),
            lead,
            60_000.0 / data.info.init_bpm / resolution,
            resolution,
        )
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

    fn from_events<'a>(
        bpms: impl Iterator<Item = &'a super::model::Bpm>,
        stops: impl Iterator<Item = &'a super::model::Stop>,
        lead: f64,
        mut step: f64,
        resolution: f64,
    ) -> Result<Self> {
        let (mut bpms, mut stops) = (bpms.peekable(), stops.peekable());
        let mut points = Vec::new();
        let (mut previous, mut time, mut y) = (0, lead, 0);
        loop {
            time += (y - previous) as f64 * step;
            let arrival = time;
            while let Some(event) = bpms.next_if(|e| e.y == y) {
                step = 60_000.0 / event.bpm / resolution;
            }
            let mut stop_time = 0.0;
            while let Some(event) = stops.next_if(|e| e.y == y) {
                stop_time += event.duration as f64 * step;
            }
            time += stop_time;
            ensure!(time.is_finite() && step.is_finite(), "Timing overflow");
            points.push((y, arrival, time, step));
            previous = y;
            match bpms
                .peek()
                .map(|e| e.y)
                .into_iter()
                .chain(stops.peek().map(|e| e.y))
                .min()
            {
                Some(next) => y = next,
                None => break,
            }
        }
        Ok(Self { points })
    }
}

#[cfg(test)]
#[path = "timing_tests.rs"]
mod tests;
