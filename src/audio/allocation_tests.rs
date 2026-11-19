use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static PEAK_ALLOC_SIZE: Cell<Option<usize>> = const { Cell::new(None) };
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation(size);
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn record_allocation(size: usize) {
    let _ = ALLOCATIONS.try_with(|count| {
        count.set(count.get().map(|value| value + 1));
    });
    let _ = PEAK_ALLOC_SIZE.try_with(|peak| {
        peak.set(peak.get().map(|value| value.max(size)));
    });
}

#[test]
fn gain_settings_and_playback_do_not_allocate_pcm() {
    [1, 2, 3, 6].into_iter().for_each(|channels| {
        [1, 44_100].into_iter().for_each(|frames| {
            let mut sample = DecodedSample::new(
                channels,
                44100,
                Arc::new(vec![0.5; frames * channels as usize]),
            );
            let settings = crate::sample_settings::SampleSettings {
                attenuation: -2000,
                ..Default::default()
            };
            ALLOCATIONS.with(|count| count.set(Some(0)));
            apply_settings(&mut sample, &settings);
            let source = sample_source(&sample, None);
            let output_channels = channels.max(2);
            let actual_channels = source.channels().get();
            let mut samples = 0;
            source.for_each(|value| {
                assert_eq!(value, 0.05);
                samples += 1;
            });
            let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());

            assert_eq!(allocations, 0, "channels={channels}, frames={frames}");
            assert_eq!(actual_channels, output_channels);
            assert_eq!(samples, frames * usize::from(output_channels));
        });
    });
}

#[test]
fn gains_preserve_pcm_and_restart_channel_parity_for_each_frame() {
    [1, 2, 3, 6].into_iter().for_each(|channels| {
        let original = Arc::new(vec![1.0, -0.5, 0.25, -1.0, 0.5, -0.25, 0.75]);
        let mut sample = DecodedSample::new(channels, 44100, original.clone());
        let settings = crate::sample_settings::SampleSettings {
            pan: 2000,
            volume: Some(50),
            ..Default::default()
        };
        let (left, right) = settings.gains();
        let expected: Vec<f32> = original
            .chunks(channels as usize)
            .flat_map(|frame| {
                if channels == 1 {
                    vec![frame[0] * left, frame[0] * right]
                } else {
                    frame
                        .iter()
                        .enumerate()
                        .map(|(i, value)| value * if i % 2 == 0 { left } else { right })
                        .collect()
                }
            })
            .collect();
        apply_settings(&mut sample, &settings);
        let source = sample_source(&sample, None);
        assert!(Arc::ptr_eq(&source.data, &original));
        assert_eq!(source.channels().get(), channels.max(2));
        assert_eq!(source.sample_rate().get(), 44100);
        assert_eq!(source.collect::<Vec<_>>(), expected);
        assert_eq!(*original, [1.0, -0.5, 0.25, -1.0, 0.5, -0.25, 0.75]);
    });
}

#[test]
fn capped_collection_rejects_unbounded_silence_without_allocating_for_its_full_length() {
    let max_samples = 1_000usize;
    // An endless stream of silence stands in for a crafted long-duration
    // (synthetic silence) sample: real decoders only read from the source as
    // `collect_capped` pulls items, so this exercises the exact mechanism a
    // compression-bomb file would hit, without needing gigabytes of real PCM.
    let unbounded_silence = std::iter::repeat(0.0_f32);

    ALLOCATIONS.with(|count| count.set(Some(0)));
    PEAK_ALLOC_SIZE.with(|peak| peak.set(Some(0)));
    let result = collect_capped(unbounded_silence, max_samples);
    let peak = PEAK_ALLOC_SIZE.with(|peak| peak.replace(None).unwrap());
    ALLOCATIONS.with(|count| count.set(None));

    assert!(result.is_err());
    // No single allocation should come anywhere close to the unbounded input;
    // it should stay on the order of the capped sample count.
    let max_expected_bytes = (max_samples + 1) * std::mem::size_of::<f32>() * 2;
    assert!(
        peak <= max_expected_bytes,
        "allocation of {peak} bytes exceeds the capped bound of {max_expected_bytes} bytes"
    );
}
