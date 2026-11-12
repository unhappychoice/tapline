use super::super::model::{Bmson, Bpm, Stop};
use super::Timeline;
use std::cell::Cell;

fn chart() -> Bmson {
    serde_json::from_str(
        r#"{"version":"1.0.0","info":{"title":"Timing","artist":"Test","init_bpm":120},"sound_channels":[]}"#,
    )
    .unwrap()
}

#[test]
fn unsorted_events_preserve_duplicate_order_and_stop_arrival() {
    let mut data = chart();
    data.bpm_events = vec![
        Bpm { y: 480, bpm: 240.0 },
        Bpm { y: 240, bpm: 60.0 },
        Bpm { y: 0, bpm: 60.0 },
        Bpm { y: 240, bpm: 120.0 },
    ];
    data.stop_events = vec![
        Stop {
            y: 240,
            duration: 120,
        },
        Stop {
            y: 0,
            duration: 240,
        },
        Stop {
            y: 480,
            duration: 240,
        },
        Stop {
            y: 240,
            duration: 240,
        },
    ];
    let timeline = Timeline::new(&data, 100.0).unwrap();
    assert_eq!(timeline.time(0), 100.0);
    assert_eq!(timeline.time(120), 1600.0);
    assert_eq!(timeline.time(240), 2100.0);
    assert_eq!(timeline.time(360), 3100.0);
    assert_eq!(timeline.time(480), 3350.0);
    assert_eq!(timeline.time(720), 3850.0);
}

#[test]
fn distinct_events_are_consumed_once_as_input_doubles() {
    for count in [10_000, 20_000] {
        let bpms: Vec<_> = (1..=count)
            .map(|y| Bpm {
                y: y * 2,
                bpm: 120.0,
            })
            .collect();
        let stops: Vec<_> = (1..=count)
            .map(|y| Stop {
                y: y * 2 - 1,
                duration: 240,
            })
            .collect();
        let visits = Cell::new(0);
        let timeline = Timeline::from_events(
            bpms.iter().inspect(|_| visits.set(visits.get() + 1)),
            stops.iter().inspect(|_| visits.set(visits.get() + 1)),
            0.0,
            60_000.0 / 120.0 / 240.0,
            240.0,
        )
        .unwrap();
        assert_eq!(visits.get(), 2 * count);
        assert_eq!(timeline.points.len(), (2 * count + 1) as usize);
        let expected = f64::from(count) * (500.0 + 1000.0 / 240.0);
        assert!((timeline.time(u64::from(2 * count)) - expected).abs() < 0.001);
    }
}

#[test]
fn many_unsorted_stops_use_the_initial_bpm() {
    let mut data = chart();
    data.stop_events = (1..=10_000)
        .rev()
        .map(|y| Stop { y, duration: 240 })
        .collect();
    let timeline = Timeline::new(&data, 0.0).unwrap();
    assert_eq!(timeline.points.len(), 10_001);
    let expected = 10_001.0 * 500.0 / 240.0 + 10_000.0 * 500.0;
    assert!((timeline.time(10_001) - expected).abs() < 0.001);
}
