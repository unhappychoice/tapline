use crate::chart::{Chart, VolumeEvent, VolumeTarget};

pub(crate) struct PlaybackVolume {
    bgm: Vec<VolumeEvent>,
    key: Vec<VolumeEvent>,
    master: f32,
    #[cfg(test)]
    visits: std::cell::Cell<usize>,
    #[cfg(test)]
    pub(crate) last_query: std::cell::Cell<Option<(VolumeTarget, f64, f32)>>,
}

impl PlaybackVolume {
    pub(crate) fn new(chart: &Chart) -> Self {
        // Chart events are time-sorted; partitioning preserves same-time precedence.
        let (bgm, key) = chart
            .volume_events
            .iter()
            .copied()
            .partition(|event| event.target == VolumeTarget::Bgm);
        Self {
            #[cfg(test)]
            visits: std::cell::Cell::new(0),
            #[cfg(test)]
            last_query: std::cell::Cell::new(None),
            bgm,
            key,
            master: chart.vol_wav.unwrap_or(100) as f32 / 100.0,
        }
    }

    pub(crate) fn gain(&self, target: VolumeTarget, time_ms: f64) -> f32 {
        let events = match target {
            VolumeTarget::Bgm => &self.bgm,
            VolumeTarget::Key => &self.key,
        };
        #[cfg(test)]
        self.visits.set(0);
        let end = events.partition_point(|event| {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            event.time_ms <= time_ms
        });
        let volume = end.checked_sub(1).map_or(255, |i| events[i].volume);
        let gain = self.master * volume as f32 / 255.0;
        #[cfg(test)]
        self.last_query.set(Some((target, time_ms, gain)));
        gain
    }
}

#[cfg(test)]
mod tests;
