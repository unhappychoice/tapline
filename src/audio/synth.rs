use super::{
    sample_source, synth_ping, DecodedSample, SharedSamples, HIT_SYNTH_DURATION_MS, LANE_PITCHES,
};
use std::sync::{Arc, LazyLock};

pub(super) fn hit_source(lane: usize) -> SharedSamples {
    sample_source(&HIT_SAMPLES[lane % LANE_PITCHES.len()], None)
}

static HIT_SAMPLES: LazyLock<[DecodedSample; LANE_PITCHES.len()]> = LazyLock::new(|| {
    LANE_PITCHES.map(|freq| DecodedSample {
        channels: 1,
        sample_rate: 44_100,
        data: Arc::new(synth_ping(freq, HIT_SYNTH_DURATION_MS, 44_100)),
    })
});

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    #[test]
    fn repeated_hits_reuse_pcm_and_preserve_every_lane_waveform() {
        LANE_PITCHES.iter().enumerate().for_each(|(lane, &freq)| {
            let first = hit_source(lane);
            let repeated = hit_source(lane);
            let wrapped = hit_source(lane + LANE_PITCHES.len());
            assert!(Arc::ptr_eq(&first.data, &repeated.data));
            assert!(Arc::ptr_eq(&first.data, &wrapped.data));
            assert_eq!(first.channels().get(), 1);
            assert_eq!(first.sample_rate().get(), 44_100);
            assert_eq!(first.end, 3528);
            assert_eq!(
                first.collect::<Vec<_>>(),
                synth_ping(freq, HIT_SYNTH_DURATION_MS, 44_100)
            );
        });
    }

    #[test]
    fn overlapping_hits_have_independent_positions_and_gains() {
        let expected = synth_ping(LANE_PITCHES[0], HIT_SYNTH_DURATION_MS, 44_100);
        let mut first = hit_source(0).amplify(0.25);
        assert_eq!(first.next(), Some(expected[0] * 0.25));
        let mut second = hit_source(0).amplify(0.75);
        assert_eq!(second.next(), Some(expected[0] * 0.75));
        assert_eq!(second.next(), Some(expected[1] * 0.75));
        assert_eq!(first.next(), Some(expected[1] * 0.25));
        assert_eq!(first.count(), expected.len() - 2);
        assert_eq!(second.count(), expected.len() - 2);
        assert_eq!(hit_source(0).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn gameplay_hits_enqueue_shared_pcm_without_an_audio_device() {
        let (mixer, mut output) = rodio::mixer::mixer(rodio::nz!(2), rodio::nz!(44_100));
        let bus = super::super::master::master_bus(&mixer);
        let bank = super::super::SampleBank {
            bus: Some(bus),
            ..super::super::SampleBank::silent()
        };
        assert!(output.by_ref().take(2048).all(|sample| sample == 0.0));
        let source = hit_source(6);
        bank.play_hit_with_gain(6, None, false, 0.25);
        bank.play_hit_with_gain(6, Some(123), true, 0.5);
        assert!(Arc::strong_count(&source.data) >= 4);
        source.for_each(|sample| {
            let expected = sample * 0.25 + sample * 0.5;
            assert_eq!(output.next(), Some(expected));
            assert_eq!(output.next(), Some(expected));
        });
        assert!(output.take(2048).all(|sample| sample == 0.0));
    }
}
