use crate::chart::{Chart, ScrollEvent};

/// Immutable index built once when gameplay loads a chart. Events must be time-sorted.
pub struct ScrollMap {
    events: Vec<ScrollEvent>,
    coordinates: Vec<f64>,
    ranges: Vec<(f64, f64)>,
    #[cfg(test)]
    visits: std::cell::Cell<usize>,
}

impl ScrollMap {
    pub fn new(events: &[ScrollEvent]) -> Self {
        let mut time = 0.0;
        let mut coordinate = 0.0;
        let mut rate = 1.0;
        let coordinates: Vec<_> = events
            .iter()
            .map(|event| {
                coordinate += (event.time_ms - time) * rate;
                time = event.time_ms;
                rate = event.multiplier;
                coordinate
            })
            .collect();
        let mut ranges = vec![(f64::INFINITY, f64::NEG_INFINITY); events.len() * 2];
        coordinates.iter().enumerate().for_each(|(i, &value)| {
            ranges[events.len() + i] = (value, value);
        });
        for i in (1..events.len()).rev() {
            ranges[i] = merge(ranges[i * 2], ranges[i * 2 + 1]);
        }
        Self {
            events: events.to_vec(),
            coordinates,
            ranges,
            #[cfg(test)]
            visits: std::cell::Cell::new(0),
        }
    }

    pub fn coordinate(&self, time: f64) -> f64 {
        let index = self.index(time, true);
        index.checked_sub(1).map_or(time, |i| {
            self.coordinates[i] + (time - self.events[i].time_ms) * self.events[i].multiplier
        })
    }

    /// Query endpoints and internal event extrema in O(log S) time.
    pub fn range(&self, start: f64, end: f64) -> (f64, f64) {
        let a = self.coordinate(start);
        let b = self.coordinate(end);
        let mut range = (a.min(b), a.max(b));
        let mut left = self.index(start, true) + self.events.len();
        let mut right = self.index(end, false) + self.events.len();
        while left < right {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            if left % 2 == 1 {
                range = merge(range, self.ranges[left]);
                left += 1;
            }
            if right % 2 == 1 {
                right -= 1;
                range = merge(range, self.ranges[right]);
            }
            left /= 2;
            right /= 2;
        }
        range
    }

    pub fn frame(&self, chart: &Chart, now: f64) -> ScrollFrame<'_> {
        let index = chart.speed.partition_point(|event| event.time_ms <= now);
        let speed = index
            .checked_sub(1)
            .map_or(1.0, |i| chart.speed[i].multiplier);
        ScrollFrame {
            map: self,
            now: self.coordinate(now),
            speed,
        }
    }
    fn index(&self, time: f64, inclusive: bool) -> usize {
        self.events.partition_point(|event| {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            event.time_ms < time || (inclusive && event.time_ms == time)
        })
    }
}

pub struct ScrollFrame<'a> {
    map: &'a ScrollMap,
    now: f64,
    speed: f64,
}

impl ScrollFrame<'_> {
    pub fn distance(&self, time: f64) -> f64 {
        self.transform(self.map.coordinate(time))
    }

    pub fn range(&self, start: f64, end: f64) -> (f64, f64) {
        let (min, max) = self.map.range(start, end);
        let a = self.transform(min);
        let b = self.transform(max);
        (a.min(b), a.max(b))
    }

    fn transform(&self, coordinate: f64) -> f64 {
        (coordinate - self.now) * self.speed
    }
}

fn merge(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0.min(b.0), a.1.max(b.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_ranges_match_integrated_extrema() {
        let chart = Chart {
            scroll: vec![
                ScrollEvent {
                    time_ms: -100.0,
                    multiplier: 0.5,
                },
                ScrollEvent {
                    time_ms: 100.0,
                    multiplier: 0.0,
                },
                ScrollEvent {
                    time_ms: 200.0,
                    multiplier: -2.0,
                },
                ScrollEvent {
                    time_ms: 200.0,
                    multiplier: -1.0,
                },
                ScrollEvent {
                    time_ms: 400.0,
                    multiplier: 2.0,
                },
            ],
            speed: vec![
                ScrollEvent {
                    time_ms: 100.0,
                    multiplier: 0.0,
                },
                ScrollEvent {
                    time_ms: 200.0,
                    multiplier: -2.0,
                },
                ScrollEvent {
                    time_ms: 400.0,
                    multiplier: 0.5,
                },
            ],
            ..Chart::default()
        };
        let map = ScrollMap::new(&chart.scroll);
        for now in [-200.0, 100.0, 250.0, 500.0] {
            let frame = map.frame(&chart, now);
            for start in [-200.0, 100.0, 200.0, 300.0, 500.0] {
                for end in [start, start + 100.0, 600.0] {
                    let expected = [start, end]
                        .into_iter()
                        .chain(
                            chart
                                .scroll
                                .iter()
                                .filter(|e| e.time_ms > start && e.time_ms < end)
                                .map(|e| e.time_ms),
                        )
                        .map(|time| chart.scroll_distance(time, now))
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |range, value| {
                            merge(range, (value, value))
                        });
                    assert_eq!(frame.range(start, end), expected);
                    assert_eq!(frame.distance(start), chart.scroll_distance(start, now));
                }
            }
        }
    }

    #[test]
    fn empty_index_preserves_default_scroll() {
        let chart = Chart::default();
        let map = ScrollMap::new(&[]);
        let frame = map.frame(&chart, -100.0);
        assert_eq!(frame.distance(100.0), 200.0);
        assert_eq!(frame.range(100.0, 200.0), (200.0, 300.0));
    }

    #[test]
    fn long_note_queries_visit_logarithmically_many_entries() {
        for count in [100usize, 1000, 10_000] {
            let events: Vec<_> = (0..count)
                .map(|i| ScrollEvent {
                    time_ms: i as f64,
                    multiplier: if i % 2 == 0 { -1.0 } else { 1.0 },
                })
                .collect();
            let map = ScrollMap::new(&events);
            let chart = Chart::default();
            let frame = map.frame(&chart, 0.0);
            map.visits.set(0);
            frame.range(-1.0, count as f64 + 1.0);
            let bound = 5 * (count.ilog2() as usize + 2);
            assert!(
                map.visits.get() <= bound,
                "{} events: {} visits",
                count,
                map.visits.get()
            );
            map.visits.set(0);
            frame.distance(-1.0);
            assert!(map.visits.get() <= count.ilog2() as usize + 2);
        }
    }
}
