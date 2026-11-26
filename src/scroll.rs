use crate::chart::{Chart, ScrollEvent};

/// Immutable index built once when gameplay loads a chart. Events must be time-sorted.
pub struct ScrollMap {
    segments: Vec<ScrollSegment>,
    ranges: Vec<(f64, f64)>,
    #[cfg(test)]
    visits: std::cell::Cell<usize>,
}

impl ScrollMap {
    pub fn new(events: &[ScrollEvent]) -> Self {
        let segments: Vec<_> = integrate(events.iter()).collect();
        let mut ranges = vec![(f64::INFINITY, f64::NEG_INFINITY); events.len() * 2];
        segments.iter().enumerate().for_each(|(i, segment)| {
            ranges[events.len() + i] = (segment.coordinate, segment.coordinate);
        });
        for i in (1..events.len()).rev() {
            ranges[i] = merge(ranges[i * 2], ranges[i * 2 + 1]);
        }
        Self {
            segments,
            ranges,
            #[cfg(test)]
            visits: std::cell::Cell::new(0),
        }
    }

    pub fn coordinate(&self, time: f64) -> f64 {
        self.segments[..self.index(time, true)]
            .last()
            .map_or(time, |segment| segment.coordinate_at(time))
    }

    /// Query endpoints and internal event extrema in O(log S) time.
    pub fn range(&self, start: f64, end: f64) -> (f64, f64) {
        let a = self.coordinate(start);
        let b = self.coordinate(end);
        let mut range = (a.min(b), a.max(b));
        let mut left = self.index(start, true) + self.segments.len();
        let mut right = self.index(end, false) + self.segments.len();
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
        ScrollFrame {
            map: self,
            now: self.coordinate(now),
            speed: speed_at(chart, now),
        }
    }

    fn index(&self, time: f64, inclusive: bool) -> usize {
        self.segments.partition_point(|segment| {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            segment.time < time || (inclusive && segment.time == time)
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

    pub(crate) fn transform(&self, coordinate: f64) -> f64 {
        scaled_distance(coordinate, self.now, self.speed)
    }
}

/// Chart loaders call this before accepting scroll events: integrating a finite
/// multiplier against an already-huge coordinate can still overflow to infinity,
/// after which no later reset can recover a usable position. `end_time_ms` is the
/// chart's full playable duration: the last scroll rate stays in effect until then,
/// so a single extreme rate with no later reset must be checked at that bound too.
pub(crate) fn coordinates_are_finite(events: &[ScrollEvent], end_time_ms: f64) -> bool {
    let segments: Vec<_> = integrate(events.iter()).collect();
    segments.iter().all(|segment| segment.coordinate.is_finite())
        && segments
            .last()
            .is_none_or(|segment| segment.coordinate_at(end_time_ms).is_finite())
}

pub(crate) fn distance(chart: &Chart, time: f64, now: f64) -> f64 {
    let coordinate = |time| {
        integrate(
            chart
                .scroll
                .iter()
                .take_while(|event| event.time_ms <= time),
        )
        .last()
        .map_or(time, |segment| segment.coordinate_at(time))
    };
    scaled_distance(coordinate(time), coordinate(now), speed_at(chart, now))
}

#[derive(Clone, Copy)]
struct ScrollSegment {
    time: f64,
    coordinate: f64,
    rate: f64,
}

impl ScrollSegment {
    fn coordinate_at(&self, time: f64) -> f64 {
        self.coordinate + (time - self.time) * self.rate
    }
}

fn integrate<'a>(
    events: impl Iterator<Item = &'a ScrollEvent> + 'a,
) -> impl Iterator<Item = ScrollSegment> + 'a {
    let mut segment = ScrollSegment {
        time: 0.0,
        coordinate: 0.0,
        rate: 1.0,
    };
    events.map(move |event| {
        segment = ScrollSegment {
            time: event.time_ms,
            coordinate: segment.coordinate_at(event.time_ms),
            rate: event.multiplier,
        };
        segment
    })
}

fn speed_at(chart: &Chart, now: f64) -> f64 {
    let index = chart.speed.partition_point(|event| event.time_ms <= now);
    chart.speed[..index]
        .last()
        .map_or(1.0, |event| event.multiplier)
}

fn scaled_distance(coordinate: f64, now: f64, speed: f64) -> f64 {
    (coordinate - now) * speed
}

fn merge(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0.min(b.0), a.1.max(b.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simultaneous_bms_scroll_changes_have_no_distance_between_them() {
        let chart = crate::bms::load_bytes(
            std::path::Path::new("simultaneous.bms"),
            b"#BPM 120\n#SCROLL01 1e20\n#SCROLL02 1\n#000SC:0001\n#000SC:0002\n",
            0.0,
            0,
        )
        .unwrap();
        assert_eq!(chart.scroll.len(), 2);
        assert_eq!(chart.scroll[0].time_ms, 1000.0);
        assert_eq!(chart.scroll[1].time_ms, 1000.0);
        assert_eq!(chart.scroll[0].multiplier, 1e20);
        assert_eq!(chart.scroll[1].multiplier, 1.0);

        let map = ScrollMap::new(&chart.scroll);
        assert_eq!(chart.scroll_distance(1500.0, 1000.0), 500.0);
        assert_eq!(map.frame(&chart, 1000.0).distance(1500.0), 500.0);
    }

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
                    multiplier: 3.0,
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
        for (now, time, expected) in [
            (-200.0, 300.0, 100.0),
            (100.0, 300.0, 0.0),
            (200.0, 300.0, 200.0),
            (400.0, 500.0, 100.0),
        ] {
            assert_eq!(chart.scroll_distance(time, now), expected);
            assert_eq!(map.frame(&chart, now).distance(time), expected);
        }

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
    fn coordinates_are_finite_detects_overflow_from_a_finite_multiplier() {
        let events = vec![
            ScrollEvent {
                time_ms: 0.0,
                multiplier: 1e308,
            },
            ScrollEvent {
                time_ms: 2000.0,
                multiplier: 1.0,
            },
        ];
        assert!(!coordinates_are_finite(&events, 5000.0));
    }

    #[test]
    fn coordinates_are_finite_detects_an_unreset_extreme_rate_at_a_later_time() {
        let events = vec![ScrollEvent {
            time_ms: 0.0,
            multiplier: 1e308,
        }];
        assert!(!coordinates_are_finite(&events, 2000.0));
    }

    #[test]
    fn coordinates_are_finite_accepts_ordinary_rates() {
        let events = vec![
            ScrollEvent {
                time_ms: 0.0,
                multiplier: 2.0,
            },
            ScrollEvent {
                time_ms: 1000.0,
                multiplier: -1.0,
            },
        ];
        assert!(coordinates_are_finite(&events, 5000.0));
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
