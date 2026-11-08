use super::*;
use std::{sync::mpsc, thread, time::Duration};

const BASE: u64 = 9_007_199_254_740_992; // Beyond exact f64 integer precision.
const BLOCK: usize = 480;
#[test]
fn whole_capture_packets_keep_qpc_start_boundary_and_separate_output_limit() {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    let mut first = pcm(12345, t(0));
    first.samples = (12345..17145).map(value).collect();
    first.flags.discontinuity = true;
    timeline.push_audio(g, first, t(1_100_000)).unwrap();
    let second = pcm(17145, t(1_000_000));
    timeline.push_audio(g, second, t(1_100_000)).unwrap();
    let s = timeline.snapshot();
    assert_eq!(s.audio_frames, 5280);
    assert_eq!(s.metrics.startup_discontinuities, 1);
    assert_eq!(MAX_PACKET_FRAMES, 960); // output allocation/reservation unchanged
    let mut later = pcm(17625, t(1_100_000));
    later.flags.discontinuity = true;
    assert_eq!(
        timeline.push_audio(g, later, t(1_200_000)),
        Err(Error::Discontinuity)
    );
    assert_eq!(timeline.snapshot().audio_frames, 0);
    timeline.stop().unwrap();
    let g = timeline.begin(timeline.snapshot().generation, 7).unwrap();
    let mut oversized = pcm(0, t(0));
    oversized.samples.resize(4801, 0.2);
    assert_eq!(
        timeline.push_audio(g, oversized, t(1_100_000)),
        Err(Error::Format)
    );
}
fn t(delta: u64) -> Time {
    Time(BASE + delta)
}
fn pixels() -> Arc<[u8]> {
    Arc::from(vec![73; NV12_BYTES])
}
fn value(index: u64) -> f32 {
    (index % 997) as f32 / 997.0
}
fn pcm(first: u64, time: Time) -> PcmPacket {
    PcmPacket {
        device: 7,
        source: Source::SelfMicrophone,
        first_sample: first,
        capture: time,
        rate: RATE,
        channels: 1,
        flags: PacketFlags::default(),
        samples: (first..first + BLOCK as u64).map(value).collect(),
    }
}
fn frame(id: u64, capture: Time, bytes: &Arc<[u8]>) -> CorrectedFrame {
    CorrectedFrame {
        id,
        capture,
        nv12: bytes.clone(),
    }
}
fn interval(g: u64, first: u64, now: Time) -> OutputInterval {
    let mut clock = DeviceClock::new(g, u64::from(RATE)).unwrap();
    clock
        .observe(first - BLOCK as u64, Time(now.0 - 100_000))
        .unwrap();
    clock.observe(first, now).unwrap();
    clock.interval(first, BLOCK, now).unwrap()
}
fn prepared() -> (Arc<Timeline>, u64) {
    let timeline = Arc::new(Timeline::default());
    let g = timeline.begin(0, 7).unwrap();
    for n in 0..5 {
        timeline
            .push_audio(g, pcm(n * BLOCK as u64, t(n * 100_000)), t(500_000))
            .unwrap();
    }
    timeline
        .push_video(g, frame(1, t(100_000), &pixels()), t(500_000))
        .unwrap();
    (timeline, g)
}

#[test]
fn explicit_clock_units_large_qpc_and_non_sample_device_frequency() {
    assert_eq!(
        Time::from_qpc_ticks(12_000_000, 24_000_000).unwrap().0,
        5_000_000
    );
    assert_eq!(Time::from_qpc_ticks(1, 0), Err(Error::Clock));
    assert_eq!(Time::from_qpc_ticks(u64::MAX, 1), Err(Error::Clock));
    // 10 MHz device positions are not 48 kHz sample counts.
    let mut clock = DeviceClock::new(1, 10_000_000).unwrap();
    clock.observe(900_000_000, t(0)).unwrap();
    clock.observe(900_100_000, t(100_000)).unwrap();
    let w = clock.interval(900_120_000, BLOCK, t(100_000)).unwrap();
    assert_eq!(w.first_sample_time(), t(120_000));
    assert!((w.sample_period_100ns() - 10_000_000.0 / 48_000.0).abs() < 1e-10);
    assert!(matches!(
        clock.interval(900_099_999, BLOCK, t(100_000)),
        Err(Error::Clock)
    ));
    assert!(matches!(
        clock.interval(901_100_001, BLOCK, t(100_000)),
        Err(Error::Clock)
    ));
    assert!(matches!(
        clock.interval(900_200_000, BLOCK, t(1_200_000)),
        Err(Error::Clock)
    ));
}

#[test]
fn discontinuous_stale_or_unbounded_rate_clocks_stay_faulted() {
    for (pos, time) in [(480, t(0)), (480, t(200_000)), (9_600, t(2_000_000))] {
        let mut clock = DeviceClock::new(1, 48_000).unwrap();
        clock.observe(0, t(0)).unwrap();
        assert!(matches!(
            clock.interval(0, BLOCK, t(0)),
            Err(Error::ClockNotReady)
        ));
        assert_eq!(clock.observe(pos, time), Err(Error::Clock));
        assert_eq!(clock.observe(480, t(100_000)), Err(Error::Clock));
        assert!(matches!(
            clock.interval(480, BLOCK, t(100_000)),
            Err(Error::Clock)
        ));
    }
}

#[test]
fn stationary_device_observations_do_not_refresh_the_clock_lease() {
    let mut clock = DeviceClock::new(1, RATE as u64).unwrap();
    clock.observe(0, t(0)).unwrap();
    clock.observe(0, t(50_000)).unwrap();
    assert!(matches!(
        clock.interval(0, BLOCK, t(50_000)),
        Err(Error::ClockNotReady)
    ));
    clock.observe(480, t(100_000)).unwrap();
    clock.observe(480, t(200_000)).unwrap();
    // The same position is legal but not new evidence that the endpoint advances.
    assert!(clock.interval(960, BLOCK, t(200_000)).is_ok());
    assert_eq!(clock.observe(480, t(1_100_001)), Err(Error::Clock));
    assert_eq!(clock.observe(5_760, t(1_200_000)), Err(Error::Clock));
}

#[test]
fn timestamp_quantization_allows_small_packets_but_not_real_discontinuities() {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    for n in 0..20 {
        let mut p = pcm(n, t(n * HNS_PER_SECOND / RATE as u64));
        p.samples.truncate(1);
        timeline.push_audio(g, p, t(10_000)).unwrap();
    }
    assert_eq!(timeline.snapshot().audio_frames, 20);
    let mut p = pcm(20, t(20 * HNS_PER_SECOND / RATE as u64 + 10));
    p.samples.truncate(1);
    assert_eq!(timeline.push_audio(g, p, t(10_000)), Err(Error::Clock));
    assert_eq!(timeline.snapshot().audio_frames, 0);
}

#[test]
fn original_pcm_and_corrected_video_share_the_capture_window() {
    let (timeline, g) = prepared();
    let ready = timeline
        .with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |window| {
            let w = window.unwrap();
            assert_eq!(w.first_capture, t(100_000));
            assert_eq!(w.first_presentation, t(500_000));
            assert_eq!(w.generation, g);
            assert_eq!(w.video.id, 1);
            assert!(w.video.nv12.iter().all(|v| *v == 73));
            for i in 0..BLOCK {
                let p = w.position_at(i).unwrap();
                assert_eq!(p.sample, 480 + i as u64);
                assert!(p.fraction.abs() < 1e-8);
                assert_eq!(w.sample(p.sample), Some(value(480 + i as u64)));
            }
            assert!(w.position_at(BLOCK).is_none());
            assert!(w.sample(u64::MAX).is_none());
            true
        })
        .unwrap();
    assert!(ready);
    let s = timeline.snapshot();
    assert_eq!(s.metrics.output_frames, 480);
    assert_eq!(s.metrics.unique_video, 1);
    assert!(s.audio_frames <= MAX_AUDIO_FRAMES);
}

#[test]
fn measured_first_window_lag_crossing_suppresses_video_and_voice_without_retiming() {
    // Diagnostic CI 42463733676 measured a 455112 x 100 ns source/video gap.
    // Another 6 ms of processing/setup crosses the fixed 50 ms lag limit even
    // though the frame remains younger than the 250 ms absolute safety lease.
    // Reproduce both cases deterministically, independent of runner wall time.
    for (extra, expected) in [(0, true), (60_000, false)] {
        let timeline = Timeline::default();
        let g = timeline.begin(0, 7).unwrap();
        let presentation = t(855_112 + extra);
        for n in 0..9 {
            timeline.push_audio(g, pcm(n * BLOCK as u64, t(n * 100_000)), presentation).unwrap();
        }
        timeline.push_video(g, frame(81, t(0), &pixels()), presentation).unwrap();
        let ready = timeline.with_window(g, interval(g, 2_400, presentation), presentation, |view| {
            assert_eq!(view.is_some(), expected, "available media must still obey the lag boundary");
            if let Some(w) = view {
                assert_eq!(w.video.capture, t(0), "original capture may not be refreshed");
                assert_eq!(w.first_capture, t(455_112));
                assert!(w.position_at(BLOCK - 1).is_some());
            }
            true
        }).unwrap();
        assert_eq!(ready, expected);
        let s = timeline.snapshot();
        assert_eq!(s.metrics.output_frames, if expected {480} else {0});
        assert_eq!(s.metrics.silence_frames, if expected {0} else {480});
    }
}

fn clock_time(sample: u64, ppm: i64) -> Time {
    Time(
        BASE + (u128::from(sample) * HNS_PER_SECOND as u128 * (1_000_000 + ppm) as u128
            / (RATE as u128 * 1_000_000)) as u64,
    )
}

#[test]
fn ten_minutes_of_two_drifting_clocks_remain_bounded_and_match_original_samples() {
    // +600 ppm capture period and -400 ppm output period: a fixed sample-index
    // mapping drifts by about 600 ms here. Simulated time, no sleep or devices.
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    let mut output_clock = DeviceClock::new(g, RATE as u64).unwrap();
    output_clock.observe(0, t(0)).unwrap();
    let bytes = pixels();
    let (mut input, mut video_id, mut video_pts) = (0_u64, 1_u64, 0_u64);
    let (mut ready_count, mut peak_error) = (0, 0.0_f64);
    for tick in 1..=60_000_u64 {
        let out_sample = tick * BLOCK as u64;
        let now = clock_time(out_sample, -400);
        while clock_time(input + BLOCK as u64, 600) <= now {
            timeline
                .push_audio(g, pcm(input, clock_time(input, 600)), now)
                .unwrap();
            input += BLOCK as u64;
        }
        // Completed corrected frames have varying 5/12/20 ms processing delays.
        while BASE + video_pts + [50_000, 120_000, 200_000][video_id as usize % 3] <= now.0 {
            timeline
                .push_video(g, frame(video_id, t(video_pts), &bytes), now)
                .unwrap();
            video_id += 1;
            video_pts = (video_id - 1) * HNS_PER_SECOND / 30;
        }
        output_clock.observe(out_sample, now).unwrap();
        let window = output_clock.interval(out_sample, BLOCK, now).unwrap();
        let ready = timeline
            .with_window(g, window, now, |view| {
                if let Some(w) = view {
                    ready_count += 1;
                    for i in [0, BLOCK / 2, BLOCK - 1] {
                        let p = w.position_at(i).unwrap();
                        // Independent oracle from the synthetic capture oscillator.
                        let desired = (w.first_capture.0 - BASE) as f64
                            + i as f64 * HNS_PER_SECOND as f64 / RATE as f64 * 0.9996;
                        let expected = desired * RATE as f64 / HNS_PER_SECOND as f64 / 1.0006;
                        peak_error =
                            peak_error.max((p.sample as f64 + p.fraction - expected).abs());
                        assert_eq!(w.sample(p.sample), Some(value(p.sample)));
                        assert!(w.sample(p.sample + 1).is_some());
                    }
                    assert!(w.first_capture.0 - w.video.capture.0 <= HNS_PER_SECOND / 30 + 1);
                }
                true
            })
            .unwrap();
        if tick >= 10 {
            assert!(ready, "unexpected silence at {tick}");
        }
    }
    assert!(ready_count > 59_990);
    assert!(peak_error < 0.02, "fractional mapping error {peak_error}");
    let s = timeline.snapshot();
    assert!(s.metrics.audio_peak <= MAX_AUDIO_FRAMES);
    assert!(s.metrics.video_peak <= MAX_VIDEO_FRAMES);
    assert_eq!(s.phase, Phase::Running);
    assert_eq!(
        s.metrics.admitted_audio,
        s.metrics.retired_audio + s.audio_frames as u64
    );
    println!("AV-E1 simulated 600 seconds: {ready_count} ready windows, max source-position error {peak_error:.6} samples, PCM peak {} frames, video peak {}", s.metrics.audio_peak, s.metrics.video_peak);
}

#[test]
fn wrong_source_format_and_capture_discontinuities_fail_both_lanes_closed() {
    for case in 0..8 {
        let (timeline, g) = prepared();
        let mut p = pcm(5 * BLOCK as u64, t(500_000));
        let expected = match case {
            0 => {
                p.source = Source::SystemLoopback;
                Error::Source
            }
            1 => {
                p.source = Source::OutputReturn;
                Error::Source
            }
            2 => {
                p.device = 8;
                Error::Source
            }
            3 => {
                p.rate = 44_100;
                Error::Format
            }
            4 => {
                p.first_sample += 1;
                Error::Discontinuity
            }
            5 => {
                p.flags.timestamp_error = true;
                Error::Clock
            }
            6 => {
                p.flags.discontinuity = true;
                Error::Discontinuity
            }
            _ => {
                p.samples[1] = f32::NAN;
                Error::Format
            }
        };
        assert_eq!(timeline.push_audio(g, p, t(800_000)), Err(expected));
        let s = timeline.snapshot();
        assert_eq!(s.phase, Phase::Fault(expected));
        assert_eq!(s.audio_frames, 0);
        assert_eq!(s.video_frames, 0);
        assert!(timeline
            .with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |_| panic!(
                "faulted handoff"
            ))
            .is_err());
    }
}

#[test]
fn capture_silence_is_zero_pcm_and_does_not_reuse_a_previous_packet() {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    for n in 0..4 {
        let mut p = pcm(n * BLOCK as u64, t(n * 100_000));
        p.flags.silent = true;
        p.samples.fill(f32::NAN);
        timeline.push_audio(g, p, t(500_000)).unwrap();
    }
    timeline
        .push_video(g, frame(1, t(100_000), &pixels()), t(500_000))
        .unwrap();
    assert!(timeline
        .with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |view| {
            let w = view.unwrap();
            assert!(w.packets().all(|(_, v)| v.iter().all(|s| *s == 0.0)));
            true
        })
        .unwrap());
}

#[test]
fn unconsumed_pcm_hits_a_hard_limit_without_discarding_normal_voice_to_catch_up() {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    for n in 0..20 {
        timeline
            .push_audio(
                g,
                pcm(n * BLOCK as u64, t(n * 100_000)),
                t(n * 100_000 + 100_000),
            )
            .unwrap();
    }
    assert_eq!(timeline.snapshot().audio_frames, MAX_AUDIO_FRAMES);
    assert_eq!(
        timeline.push_audio(g, pcm(20 * BLOCK as u64, t(2_000_000)), t(2_100_000)),
        Err(Error::AudioOverflow)
    );
    let s = timeline.snapshot();
    assert_eq!(s.phase, Phase::Fault(Error::AudioOverflow));
    assert_eq!(s.metrics.cleared_audio, MAX_AUDIO_FRAMES as u64);
    assert_eq!(s.audio_frames, 0);
    assert!(timeline.begin(g, 7).is_err());
    let stopped = timeline.stop().unwrap();
    assert!(timeline.begin(stopped, 7).is_ok());
}

#[test]
fn out_of_order_corrected_video_is_sorted_and_replays_or_overflow_are_counted() {
    let (timeline, g) = prepared();
    let bytes = pixels();
    assert!(timeline
        .push_video(g, frame(3, t(300_000), &bytes), t(600_000))
        .unwrap());
    assert!(timeline
        .push_video(g, frame(2, t(200_000), &bytes), t(600_000))
        .unwrap());
    assert!(!timeline
        .push_video(g, frame(2, t(200_000), &bytes), t(600_000))
        .unwrap());
    assert!(timeline
        .with_window(g, interval(g, 3_360, t(700_000)), t(700_000), |w| {
            assert_eq!(w.unwrap().video.id, 3);
            true
        })
        .unwrap());
    assert!(!timeline
        .push_video(g, frame(2, t(200_000), &bytes), t(800_000))
        .unwrap());
    for id in 4..20 {
        timeline
            .push_video(g, frame(id, t(id * 100_000), &bytes), t(id * 100_000))
            .unwrap();
    }
    let s = timeline.snapshot();
    assert_eq!(s.video_frames, MAX_VIDEO_FRAMES);
    assert!(s.metrics.dropped_video >= 10);
    assert_eq!(s.metrics.unique_video, 1);
}

#[test]
fn missing_audio_or_video_requests_neutral_silence_and_never_replays_output_positions() {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    let mut calls = 0;
    assert!(!timeline
        .with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |w| {
            assert!(w.is_none());
            calls += 1;
            true
        })
        .unwrap());
    assert_eq!(calls, 1);
    assert_eq!(timeline.snapshot().metrics.silence_frames, 480);
    assert_eq!(
        timeline.with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |_| panic!(
            "replayed output"
        )),
        Err(Error::OutputOrder)
    );
    assert_eq!(timeline.snapshot().phase, Phase::Fault(Error::OutputOrder));
}

#[test]
fn one_missing_lane_suppresses_both_and_late_media_does_not_replay_a_missed_window() {
    for audio_first in [true, false] {
        let timeline = Timeline::default();
        let g = timeline.begin(0, 7).unwrap();
        let audio = || {
            for n in 0..5 {
                timeline
                    .push_audio(g, pcm(n * BLOCK as u64, t(n * 100_000)), t(500_000))
                    .unwrap();
            }
        };
        let video = || {
            timeline
                .push_video(g, frame(1, t(100_000), &pixels()), t(500_000))
                .unwrap();
        };
        if audio_first {
            audio();
        } else {
            video();
        }
        assert!(!timeline
            .with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |w| {
                assert!(w.is_none());
                true
            })
            .unwrap());
        if audio_first {
            video();
        } else {
            audio();
        }
        assert!(timeline
            .with_window(g, interval(g, 2_880, t(600_000)), t(600_000), |view| {
                let w = view.unwrap();
                assert_eq!(w.first_capture, t(200_000));
                assert_eq!(w.position_at(0).unwrap().sample, 960);
                true
            })
            .unwrap());
        let s = timeline.snapshot();
        assert_eq!(s.metrics.silence_frames, 480);
        assert_eq!(s.metrics.output_frames, 480);
    }
}

#[test]
fn stop_and_three_restarts_clear_both_queues_and_isolate_old_callbacks() {
    let timeline = Timeline::default();
    assert_eq!(timeline.snapshot().phase, Phase::Off);
    let mut old = 0;
    for _ in 0..3 {
        let g = timeline.begin(timeline.snapshot().generation, 7).unwrap();
        timeline.push_audio(g, pcm(0, t(0)), t(100_000)).unwrap();
        assert_eq!(
            timeline.push_audio(old, pcm(480, t(100_000)), t(200_000)),
            Err(Error::Stale)
        );
        assert_eq!(timeline.snapshot().audio_frames, BLOCK);
        timeline
            .push_video(g, frame(1, t(0), &pixels()), t(100_000))
            .unwrap();
        timeline.stop().unwrap();
        let s = timeline.snapshot();
        assert_eq!(s.phase, Phase::Off);
        assert_eq!(s.audio_frames + s.video_frames, 0);
        assert_eq!(
            timeline.with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |_| panic!(
                "late generation"
            )),
            Err(Error::Stale)
        );
        assert!(timeline.begin(g, 7).is_err());
        old = g;
    }
}

#[test]
fn old_clock_interval_cannot_be_relabelled_with_the_new_generation() {
    let (timeline, g) = prepared();
    let old = interval(g, 2_400, t(500_000));
    let stopped = timeline.stop().unwrap();
    let current = timeline.begin(stopped, 7).unwrap();
    assert_eq!(
        timeline.with_window(current, old, t(500_000), |_| panic!("old clock reused")),
        Err(Error::Stale)
    );
    assert_eq!(timeline.snapshot().phase, Phase::Running);
    assert!(!timeline
        .with_window(
            current,
            interval(current, 2_400, t(500_000)),
            t(500_000),
            |w| {
                assert!(w.is_none());
                true
            }
        )
        .unwrap());
}

#[test]
fn rational_positions_cross_capture_packet_boundaries_without_moving_an_exact_sample_back() {
    let (timeline, g) = prepared();
    // Source time starts at 15 ms; all 480 exact samples span two input packets.
    assert!(timeline
        .with_window(g, interval(g, 2_640, t(550_000)), t(550_000), |view| {
            let w = view.unwrap();
            for i in 0..BLOCK {
                let p = w.position_at(i).unwrap();
                assert_eq!(p.sample, 720 + i as u64);
                assert_eq!(p.fraction, 0.0);
                assert_eq!(w.sample(p.sample), Some(value(p.sample)));
            }
            true
        })
        .unwrap());
}

#[test]
fn rejected_or_panicking_consumer_faults_and_clears_pending_media() {
    for panic in [false, true] {
        let (timeline, g) = prepared();
        assert_eq!(
            timeline.with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |_| {
                if panic {
                    panic!("synthetic sink panic");
                }
                false
            }),
            Err(Error::Sink)
        );
        let s = timeline.snapshot();
        assert_eq!(s.phase, Phase::Fault(Error::Sink));
        assert_eq!(s.audio_frames + s.video_frames, 0);
    }
}

#[test]
fn stop_serializes_with_the_borrowed_handoff_and_returns_no_old_window() {
    let (timeline, g) = prepared();
    let worker = timeline.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let handoff = thread::spawn(move || {
        worker.with_window(g, interval(g, 2_400, t(500_000)), t(500_000), |w| {
            assert!(w.is_some());
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            true
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let worker = timeline.clone();
    let (stopped_tx, stopped_rx) = mpsc::channel();
    let stop = thread::spawn(move || {
        worker.stop().unwrap();
        stopped_tx.send(()).unwrap();
    });
    assert!(stopped_rx.recv_timeout(Duration::from_millis(30)).is_err());
    release_tx.send(()).unwrap();
    assert_eq!(handoff.join().unwrap(), Ok(true));
    stopped_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    stop.join().unwrap();
    assert_eq!(
        timeline.with_window(g, interval(g, 2_880, t(600_000)), t(600_000), |_| panic!(
            "stopped handoff"
        )),
        Err(Error::Stale)
    );
}
#[test]
fn submitted_frame_clock_mapping_preserves_fractional_device_positions() {
    let mut clock = DeviceClock::new(1, 44_100).unwrap();
    clock.observe(0, Time::from_qpc_100ns(0)).unwrap();
    assert!(!clock.is_ready());
    clock.observe(441, Time::from_qpc_100ns(100_000)).unwrap();
    assert!(clock.is_ready());
    let now = Time::from_qpc_100ns(100_000);
    let first = clock.interval_from_frame(481, 479, now).unwrap();
    let next = clock.interval_from_frame(960, 479, now).unwrap();
    assert_eq!(first.first_sample_time().qpc_100ns(), 100_208);
    assert_eq!(
        first.end, next.start,
        "sample intervals must meet exactly in rational device units"
    );
    assert_eq!(first.start, 481u128 * 44_100);
    let prematurely_rounded = clock.interval(481 * 44_100 / 48_000, 479, now).unwrap();
    assert_eq!(prematurely_rounded.first_sample_time().qpc_100ns(), 100_000);
    assert_ne!(first.start, prematurely_rounded.start);
    assert!(
        clock.interval_from_frame(479, 1, now).is_err(),
        "no past output"
    );
    assert!(
        clock.interval_from_frame(u64::MAX, 960, now).is_err(),
        "checked range"
    );
}
