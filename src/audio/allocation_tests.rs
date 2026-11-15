use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        count.set(count.get().map(|value| value + 1));
    });
}

#[test]
fn gain_allocations_are_constant_for_short_and_three_minute_samples() {
    [1, 2, 3, 6].into_iter().for_each(|channels| {
        [1, 44100 * 180].into_iter().for_each(|frames| {
            let mut sample = DecodedSample {
                channels,
                sample_rate: 44100,
                data: Arc::new(vec![0.5; frames * channels as usize]),
            };
            let settings = crate::sample_settings::SampleSettings {
                attenuation: -2000,
                ..Default::default()
            };
            ALLOCATIONS.with(|count| count.set(Some(0)));
            apply_settings(&mut sample, &settings);
            let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
            // One PCM buffer and one Arc, independent of frame count.
            assert_eq!(allocations, 2, "channels={channels}, frames={frames}");
            assert_eq!(sample.channels, channels.max(2));
            assert_eq!(sample.data.len(), frames * channels.max(2) as usize);
            assert!(sample.data.iter().all(|&value| value == 0.05));
        });
    });
}

#[test]
fn gains_preserve_pcm_and_restart_channel_parity_for_each_frame() {
    [1, 2, 3, 6].into_iter().for_each(|channels| {
        let original = Arc::new(vec![1.0, -0.5, 0.25, -1.0, 0.5, -0.25, 0.75]);
        let mut sample = DecodedSample {
            channels,
            sample_rate: 44100,
            data: original.clone(),
        };
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
        assert_eq!(*sample.data, expected);
        assert_eq!(sample.channels, channels.max(2));
        assert_eq!(sample.sample_rate, 44100);
        assert_eq!(*original, [1.0, -0.5, 0.25, -1.0, 0.5, -0.25, 0.75]);
    });
}
