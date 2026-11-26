/// Binary-searches a time-sorted slice for the index of the last entry whose time is at or
/// before `now_ms`, returning `None` when every entry is in the future (or the slice is empty).
pub(crate) fn latest_at_or_before<T>(
    events: &[T],
    now_ms: f64,
    time_ms: impl Fn(&T) -> f64,
    mut on_compare: impl FnMut(),
) -> Option<usize> {
    let end = events.partition_point(|event| {
        on_compare();
        time_ms(event) <= now_ms
    });
    end.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_last_index_at_or_before_now_and_none_when_all_future() {
        let events = [0.0, 100.0, 100.0, 200.0];
        let latest = |now| latest_at_or_before(&events, now, |&time_ms| time_ms, || {});
        assert_eq!(latest(-1.0), None);
        assert_eq!(latest(0.0), Some(0));
        assert_eq!(latest(99.0), Some(0));
        assert_eq!(latest(100.0), Some(2));
        assert_eq!(latest(200.0), Some(3));
        assert_eq!(latest_at_or_before::<f64>(&[], 0.0, |&t| t, || {}), None);
    }

    #[test]
    fn on_compare_runs_once_per_partition_point_probe() {
        let events: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let mut comparisons = 0;
        latest_at_or_before(&events, 50.0, |&time_ms| time_ms, || comparisons += 1);
        assert!(comparisons <= events.len().ilog2() as usize + 2);
    }
}
