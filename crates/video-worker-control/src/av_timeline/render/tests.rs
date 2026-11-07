use super::*;
use std::f64::consts::TAU;
use std::time::Instant;

const BASE: u64 = 9_007_199_254_740_992;
const BLOCK: usize = 480;
fn t(ticks: u64) -> Time {
    Time(BASE + ticks)
}
fn pts(sample: u64, ppm: u64) -> Time {
    t(
        (sample as u128 * HNS_PER_SECOND as u128 * (1_000_000 + ppm) as u128
            / (RATE as u128 * 1_000_000)) as u64,
    )
}
fn picture(id: u64, capture: Time) -> CorrectedFrame {
    CorrectedFrame {
        id,
        capture,
        nv12: Arc::from(vec![73; NV12_BYTES]),
    }
}
fn packet(first: u64, size: usize, capture: Time, sample: impl Fn(u64) -> f32) -> PcmPacket {
    PcmPacket {
        device: 7,
        source: Source::SelfMicrophone,
        first_sample: first,
        capture,
        rate: RATE,
        channels: 1,
        flags: PacketFlags::default(),
        samples: (first..first + size as u64).map(sample).collect(),
    }
}
fn interval(g: u64, position: u64, now: Time, frames: usize) -> OutputInterval {
    let mut clock = DeviceClock::new(g, RATE as u64).unwrap();
    clock
        .observe(position - BLOCK as u64, Time(now.0 - 100_000))
        .unwrap();
    clock.observe(position, now).unwrap();
    clock.interval(position, frames, now).unwrap()
}
fn prepared(size: usize, offset: u64, sample: impl Fn(u64) -> f32) -> (Timeline, u64) {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    for first in (0..3_360).step_by(size) {
        timeline
            .push_audio(
                g,
                packet(first, size, Time(pts(first, 0).0 + offset), &sample),
                t(800_000),
            )
            .unwrap();
    }
    timeline
        .push_video(g, picture(1, t(400_000)), t(800_000))
        .unwrap();
    (timeline, g)
}
fn sine(index: f64, frequency: f64) -> f32 {
    (0.4 * (TAU * frequency * index / RATE as f64).sin()) as f32
}

#[test]
fn sinc_preserves_dc_and_voice_band_and_rejects_near_nyquist() {
    let renderer = SincRenderer::default();
    assert_eq!(renderer.lookahead_frames(), 128);
    let (timeline, g) = prepared(480, 37, |_| 0.25);
    assert!(timeline
        .with_rendered_window(
            g,
            interval(g, 3840, t(800_000), BLOCK),
            t(800_000),
            &renderer,
            |w| {
                assert!(w.audio.iter().all(|v| (*v - 0.25).abs() < 0.0001));
                true
            }
        )
        .unwrap());
    for hz in [1000.0, 8000.0, 18000.0, 23900.0] {
        let (timeline, g) = prepared(480, 37, |i| sine(i as f64, hz));
        let mut output = Vec::new();
        assert!(timeline
            .with_rendered_window(
                g,
                interval(g, 3840, t(800_000), BLOCK),
                t(800_000),
                &renderer,
                |w| {
                    assert_eq!(w.status, RenderStatus::Media);
                    assert_eq!(w.video.unwrap().id, 1);
                    assert_eq!(w.capture, t(400_000));
                    assert_eq!(w.presentation, t(800_000));
                    output.extend_from_slice(w.audio);
                    true
                }
            )
            .unwrap());
        let mut power = 0.0;
        let mut error = 0.0;
        let mut reference = 0.0;
        for (i, value) in output.iter().enumerate() {
            let expected = sine(
                (400_000.0 - 37.0) * RATE as f64 / HNS_PER_SECOND as f64 + i as f64,
                hz,
            ) as f64;
            power += f64::from(*value).powi(2);
            error += (f64::from(*value) - expected).powi(2);
            reference += expected.powi(2);
        }
        let relative_error = (error / reference).sqrt();
        let gain_db = 10.0 * (power / reference).log10();
        println!("sinc {hz} Hz: relative RMS error {relative_error:.8}, gain {gain_db:.3} dB");
        if hz < 20_000.0 {
            assert!(relative_error < 0.001, "{hz}: {relative_error}");
        } else {
            assert!(gain_db < -60.0, "stopband {gain_db}");
        }
    }
}

#[test]
fn impulse_and_video_tags_do_not_acquire_an_unreported_filter_delay() {
    let renderer = SincRenderer::default();
    for offset in [0, 37] {
        let (timeline, g) = prepared(480, offset, |i| if i == 2000 { 1.0 } else { 0.0 });
        timeline.with_rendered_window(g, interval(g, 3840, t(800_000), BLOCK), t(800_000), &renderer, |w| {
            let peak = w.audio.iter().enumerate().max_by(|a,b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap();
            assert!(peak.1.abs() > 0.85);
            let expected = 2000.0 - (400_000.0 - offset as f64) * RATE as f64 / HNS_PER_SECOND as f64;
            assert!((peak.0 as f64 - expected).abs() <= 1.0);
            assert_eq!(w.video.unwrap().capture, w.capture); assert_eq!(w.presentation.0 - w.capture.0, DELAY);
            println!("impulse offset={offset}: peak={}, expected={expected:.6}; declared buffer={} ticks", peak.0, DELAY);
            true
        }).unwrap();
    }
}

#[test]
fn input_packet_boundaries_and_output_splits_do_not_change_filtered_pcm() {
    let renderer = SincRenderer::default();
    let mut reference = Vec::new();
    for (size, split) in [(480, false), (48, false), (48, true)] {
        let (timeline, g) = prepared(size, 0, |i| sine(i as f64, 8000.0));
        let mut result = Vec::new();
        let parts = if split { 2 } else { 1 };
        let frames = BLOCK / parts;
        for part in 0..parts {
            let now = t(800_000 + part as u64 * 50_000);
            timeline
                .with_rendered_window(
                    g,
                    interval(g, 3840 + part as u64 * 240, now, frames),
                    now,
                    &renderer,
                    |w| {
                        assert_eq!(w.status, RenderStatus::Media);
                        result.extend_from_slice(w.audio);
                        true
                    },
                )
                .unwrap();
        }
        if reference.is_empty() {
            reference = result;
        } else {
            assert_eq!(reference, result);
        }
    }
}

#[test]
fn filter_lookahead_underrun_silences_both_lanes_and_cannot_be_replayed() {
    let renderer = SincRenderer::default();
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    for first in (0..2448).step_by(48) {
        timeline
            .push_audio(g, packet(first, 48, pts(first, 0), |_| 0.4), t(800_000))
            .unwrap();
    }
    timeline
        .push_video(g, picture(1, t(400_000)), t(800_000))
        .unwrap();
    assert!(!timeline
        .with_rendered_window(
            g,
            interval(g, 3840, t(800_000), BLOCK),
            t(800_000),
            &renderer,
            |w| {
                assert_eq!(w.status, RenderStatus::MissingFilterInput);
                assert!(w.video.is_none());
                assert!(w.audio.iter().all(|v| *v == 0.0));
                true
            }
        )
        .unwrap());
    let s = timeline.snapshot();
    assert_eq!(s.metrics.filter_underrun_windows, 1);
    assert_eq!(s.metrics.unique_video, 0);
    assert_eq!(s.metrics.output_frames, 0);
    for first in (2448..3840).step_by(48) {
        timeline
            .push_audio(g, packet(first, 48, pts(first, 0), |_| 0.4), t(900_000))
            .unwrap();
    }
    assert!(timeline
        .with_rendered_window(
            g,
            interval(g, 4320, t(900_000), BLOCK),
            t(900_000),
            &renderer,
            |w| {
                assert_eq!(w.capture, t(500_000));
                assert_eq!(w.status, RenderStatus::Media);
                true
            }
        )
        .unwrap());
}

#[test]
fn ten_seconds_of_drift_renders_voice_at_common_times_and_retains_filter_history() {
    let renderer = SincRenderer::default();
    let started = Instant::now();
    // Compare 1 ms and 10 ms input blocks, including packet retirement.
    for size in [48_u64, 480] {
        let timeline = Timeline::default();
        let g = timeline.begin(0, 7).unwrap();
        let mut clock = DeviceClock::new(g, RATE as u64).unwrap();
        clock.observe(0, t(0)).unwrap();
        let (mut input, mut video, mut ready) = (0, 0, 0);
        let (mut error, mut power) = (0.0, 0.0);
        for tick in 1..=1000_u64 {
            let now = pts(tick * BLOCK as u64, 400);
            while pts(input + size, 600) <= now {
                timeline
                    .push_audio(
                        g,
                        packet(input, size as usize, pts(input, 600), |i| {
                            sine(i as f64, 1000.0)
                        }),
                        now,
                    )
                    .unwrap();
                input += size;
            }
            if tick % 3 == 0 {
                video += 1;
                timeline
                    .push_video(g, picture(video, Time(now.0 - 100_000)), now)
                    .unwrap();
            }
            clock.observe(tick * BLOCK as u64, now).unwrap();
            let interval = clock.interval(tick * BLOCK as u64, BLOCK, now).unwrap();
            let delivered = timeline
                .with_rendered_window(g, interval, now, &renderer, |w| {
                    if w.status == RenderStatus::Media {
                        ready += 1;
                        for (i, v) in w.audio.iter().enumerate() {
                            let desired = (w.capture.0 - BASE) as f64
                                + i as f64 * HNS_PER_SECOND as f64 / RATE as f64 * 1.0004;
                            let expected = sine(
                                desired * RATE as f64 / HNS_PER_SECOND as f64 / 1.0006,
                                1000.0,
                            ) as f64;
                            error += (f64::from(*v) - expected).powi(2);
                            power += expected.powi(2);
                        }
                    } else {
                        assert!(w.audio.iter().all(|v| *v == 0.0));
                        assert!(w.video.is_none());
                    }
                    true
                })
                .unwrap();
            if tick >= 10 {
                assert!(
                    delivered,
                    "unexpected neutral at tick {tick}, input size {size}"
                );
            }
        }
        let rms = (error / power).sqrt();
        assert!(rms < 0.001, "drift RMS {rms}");
        assert!(ready > 990);
        let s = timeline.snapshot();
        assert!(s.metrics.audio_peak <= MAX_AUDIO_FRAMES);
        assert_eq!(s.phase, Phase::Running);
        println!("rendered 10 simulated seconds: input packet={size}, ready={ready}, relative RMS={rms:.8}, PCM peak={}",s.metrics.audio_peak);
    }
    println!("debug/release wall time for two deterministic render scenarios: {:?}; not an endpoint latency", started.elapsed());
}

#[test]
fn rejected_output_and_three_restarts_never_reuse_old_filtered_voice() {
    let renderer = SincRenderer::default();
    let (timeline, mut g) = prepared(480, 0, |_| 0.5);
    assert_eq!(
        timeline.with_rendered_window(
            g,
            interval(g, 3840, t(800_000), BLOCK),
            t(800_000),
            &renderer,
            |_| false
        ),
        Err(Error::Sink)
    );
    assert_eq!(timeline.snapshot().audio_frames, 0);
    for _ in 0..3 {
        let old = g;
        let stopped = timeline.stop().unwrap();
        g = timeline.begin(stopped, 7).unwrap();
        assert_eq!(
            timeline.with_rendered_window(
                old,
                interval(old, 3840, t(800_000), BLOCK),
                t(800_000),
                &renderer,
                |_| panic!("old render")
            ),
            Err(Error::Stale)
        );
        for first in (0..3360).step_by(480) {
            timeline
                .push_audio(g, packet(first, 480, pts(first, 0), |_| 0.0), t(800_000))
                .unwrap();
        }
        timeline
            .push_video(g, picture(1, t(400_000)), t(800_000))
            .unwrap();
        assert!(timeline
            .with_rendered_window(
                g,
                interval(g, 3840, t(800_000), BLOCK),
                t(800_000),
                &renderer,
                |w| {
                    assert_eq!(w.status, RenderStatus::Media);
                    assert!(w.audio.iter().all(|v| *v == 0.0));
                    true
                }
            )
            .unwrap());
    }
}

#[test]
fn one_millisecond_output_periods_keep_enough_history_after_retirement() {
    let renderer = SincRenderer::default();
    let (timeline, g) = prepared(48, 0, |i| sine(i as f64, 1000.0));
    for tick in 0..20 {
        let now = t(800_000 + tick * 10_000);
        assert!(timeline
            .with_rendered_window(
                g,
                interval(g, 3840 + tick * 48, now, 48),
                now,
                &renderer,
                |w| {
                    assert_eq!(w.status, RenderStatus::Media);
                    for (i, v) in w.audio.iter().enumerate() {
                        assert!(
                            (*v - sine((1920 + tick * 48) as f64 + i as f64, 1000.0)).abs()
                                < 0.0001
                        );
                    }
                    true
                }
            )
            .unwrap());
    }
    assert_eq!(timeline.snapshot().metrics.filter_underrun_windows, 0);
}

#[test]
fn stream_start_zero_extends_only_history_before_the_first_capture() {
    let renderer = SincRenderer::default();
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    for first in (0..1920).step_by(480) {
        timeline
            .push_audio(
                g,
                packet(
                    first,
                    480,
                    pts(first, 0),
                    |i| if i == 0 { 1.0 } else { 0.0 },
                ),
                t(400_000),
            )
            .unwrap();
    }
    timeline
        .push_video(g, picture(1, t(0)), t(400_000))
        .unwrap();
    assert!(timeline
        .with_rendered_window(
            g,
            interval(g, 1920, t(400_000), BLOCK),
            t(400_000),
            &renderer,
            |w| {
                assert!(w.audio[0] > 0.9);
                assert_eq!(w.capture, t(0));
                true
            }
        )
        .unwrap());
}
#[test]
fn full_scale_original_input_does_not_violate_the_native_output_range() {
    let renderer = SincRenderer::default();
    let (timeline, g) = prepared(480, 37, |i| if i % 48 < 24 { 1.0 } else { -1.0 });
    assert!(timeline
        .with_rendered_window(
            g,
            interval(g, 3840, t(800_000), BLOCK),
            t(800_000),
            &renderer,
            |w| {
                let peak = w.audio.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
                println!(
                    "full-scale filtered output peak={peak:.8}, saturated_samples={}",
                    w.saturated_samples
                );
                assert!(
                    peak <= 1.0,
                    "valid full-scale input must not fault the native sink"
                );
                assert!(w.saturated_samples > 0, "overshoot remains observable");
                true
            }
        )
        .unwrap());
    assert!(timeline.snapshot().metrics.saturated_samples > 0);
}
