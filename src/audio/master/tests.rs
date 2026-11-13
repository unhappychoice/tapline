use super::*;
use rodio::buffer::SamplesBuffer;

fn limit(data: Vec<f32>) -> Vec<f32> {
    Limiter::new(SamplesBuffer::new(1, 1000, data)).collect()
}

#[test]
fn quiet_signal_passes_through_unchanged() {
    let input = vec![0.5, -0.9, 0.2, 0.0];
    assert_eq!(limit(input.clone()), input);
}

#[test]
fn loud_peaks_are_held_at_the_ceiling() {
    let out = limit(vec![2.5, -3.0, 1.5, 2.0]);
    assert!(out.iter().all(|s| s.abs() <= CEILING + 1e-6), "{out:?}");
}

#[test]
fn gain_recovers_gradually_after_a_peak() {
    let mut input = vec![4.0];
    input.extend(std::iter::repeat_n(0.5, 2000));
    let out = limit(input);
    assert!(out[1] < 0.5);
    assert!(out[1] < out[200] && out[200] < out[1999]);
    assert!((out[1999] - 0.5).abs() < 0.01);
}

#[test]
fn limiter_preserves_source_format() {
    let source = Limiter::new(SamplesBuffer::new(2, 44_100, vec![0.0; 4]));
    assert_eq!((source.channels(), source.sample_rate()), (2, 44_100));
}
