//! Native-only A/V timeline with optional bounded original-voice sinc rendering.
//! Output adapters still account for their actual device delay. This is NOT an
//! installed virtual microphone or an end-to-end synchronization claim.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
mod clock;
pub use clock::{DeviceClock, OutputInterval};
mod render;
pub use render::{RenderStatus, RenderedWindow, SincRenderer};
#[cfg(test)]
mod tests;

pub const HNS_PER_SECOND: u64 = 10_000_000;
pub const RATE: u32 = 48_000;
pub const MAX_PACKET_FRAMES: usize = 960;
pub const MAX_AUDIO_FRAMES: usize = 9_600;
pub const MAX_VIDEO_FRAMES: usize = 8;
pub const NV12_BYTES: usize = 1280 * 720 * 3 / 2;
const DELAY: u64 = 400_000; // 40 ms experimental contract, not a product default.
const MAX_AGE: u64 = 2_500_000; // Preserve the existing 250 ms video safety lease.
const MAX_VIDEO_LAG: u64 = 500_000;
const FILTER_HISTORY: u64 = 30_000; // 3 ms covers the 128-sample sinc lookbehind.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time(u64);
impl Time {
    pub const fn from_qpc_100ns(value: u64) -> Self {
        Self(value)
    }
    pub const fn qpc_100ns(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Stale,
    Off,
    Clock,
    ClockNotReady,
    Source,
    Format,
    Discontinuity,
    AudioOverflow,
    OutputOrder,
    Sink,
    Render,
    Exhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Off,
    Running,
    Fault(Error),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    SelfMicrophone,
    SystemLoopback,
    OutputReturn,
}
#[derive(Default, Clone, Copy)]
pub struct PacketFlags {
    pub silent: bool,
    pub discontinuity: bool,
    pub timestamp_error: bool,
}

/// PCM is one native mono f32 stream. The adapter verifies physical endpoint
/// identity; a source enum alone cannot authenticate a real microphone.
pub struct PcmPacket {
    pub device: u64,
    pub source: Source,
    pub first_sample: u64,
    pub capture: Time,
    pub rate: u32,
    pub channels: u16,
    pub flags: PacketFlags,
    pub samples: Vec<f32>,
}
impl Drop for PcmPacket {
    fn drop(&mut self) {
        self.samples.fill(0.0);
    }
}

/// Only a corrected, validated NV12 frame is admitted by the native caller.
/// QPC capture time must be mapped by the capture adapter; v1 capture milliseconds
/// are never reinterpreted. Shared immutable storage avoids diagnostic frame copies.
pub struct CorrectedFrame {
    pub id: u64,
    pub capture: Time,
    pub nv12: Arc<[u8]>,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Metrics {
    pub admitted_audio: u64,
    pub retired_audio: u64,
    pub cleared_audio: u64,
    pub audio_peak: usize,
    pub video_peak: usize,
    pub rejected: u64,
    pub admitted_video: u64,
    pub dropped_video: u64,
    pub unique_video: u64,
    pub repeated_video: u64,
    pub output_frames: u64,
    pub silence_frames: u64,
    pub windows: u64,
    pub filter_underrun_windows: u64,
}
pub struct Snapshot {
    pub generation: u64,
    pub phase: Phase,
    pub audio_frames: usize,
    pub video_frames: usize,
    pub metrics: Metrics,
}

/// A fractional native input position. An output resampler must use a proper
/// interpolation/filter implementation and include its delay in synchronization.
#[derive(Clone, Copy, Debug)]
pub struct SourcePosition {
    pub sample: u64,
    pub fraction: f64,
}

pub struct Window<'a> {
    pub generation: u64,
    pub first_capture: Time,
    pub first_presentation: Time,
    pub frames: usize,
    pub video: &'a CorrectedFrame,
    packets: &'a VecDeque<PcmPacket>,
    period: clock::SamplePeriod,
    audio_origin: u64,
}
impl Window<'_> {
    pub fn position_at(&self, output_frame: usize) -> Option<SourcePosition> {
        if output_frame >= self.frames {
            return None;
        }
        position(self.packets, self.first_capture, self.period, output_frame)
    }
    pub fn sample(&self, index: u64) -> Option<f32> {
        self.packets.iter().find_map(|p| {
            index
                .checked_sub(p.first_sample)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| p.samples.get(i))
                .copied()
        })
    }
    /// Consumers may need lookbehind for resampling; never retain these slices.
    pub fn packets(&self) -> impl Iterator<Item = (u64, &[f32])> {
        self.packets
            .iter()
            .map(|p| (p.first_sample, p.samples.as_slice()))
    }
    /// Monotone cursor avoids rescanning all packets for every output sample.
    fn positions(&self) -> impl Iterator<Item = Option<SourcePosition>> + '_ {
        let mut packet = 0;
        (0..self.frames).map(move |frame| {
            position_from(self.packets, self.first_capture, self.period, frame, packet).map(
                |(value, index)| {
                    packet = index;
                    value
                },
            )
        })
    }
}

fn position(
    packets: &VecDeque<PcmPacket>,
    first: Time,
    period: clock::SamplePeriod,
    frame: usize,
) -> Option<SourcePosition> {
    position_from(packets, first, period, frame, 0).map(|(value, _)| value)
}

fn position_from(
    packets: &VecDeque<PcmPacket>,
    first: Time,
    period: clock::SamplePeriod,
    frame: usize,
    start: usize,
) -> Option<(SourcePosition, usize)> {
    let offset = period.numerator.checked_mul(frame as u128)?;
    // Integer time differences and rational periods preserve exact sample
    // boundaries, including QPC values too large for an f64 integer.
    for i in start..packets.len().saturating_sub(1) {
        let p = &packets[i];
        let next = &packets[i + 1];
        let gap = u128::from(first.0.abs_diff(p.capture.0)).checked_mul(period.denominator)?;
        let delta = if first >= p.capture {
            offset.checked_add(gap)
        } else {
            offset.checked_sub(gap)
        };
        let Some(delta) = delta else {
            continue;
        };
        let span =
            u128::from(next.capture.0.checked_sub(p.capture.0)?).checked_mul(period.denominator)?;
        if delta < span {
            let numerator = delta.checked_mul(p.samples.len() as u128)?;
            let whole = u64::try_from(numerator / span).ok()?;
            return Some((
                SourcePosition {
                    sample: p.first_sample.checked_add(whole)?,
                    fraction: (numerator % span) as f64 / span as f64,
                },
                i,
            ));
        }
    }
    // The last packet has no following clock anchor; do not extrapolate it.
    None
}

struct State {
    generation: u64,
    phase: Phase,
    device: u64,
    audio: VecDeque<PcmPacket>,
    video: VecDeque<CorrectedFrame>,
    audio_frames: usize,
    audio_origin: Option<u64>,
    output_end: Option<(u128, u64)>,
    presented_capture: Option<Time>,
    last_video: u64,
    metrics: Metrics,
}
impl State {
    fn clear(&mut self) {
        self.metrics.cleared_audio += self.audio_frames as u64;
        self.metrics.dropped_video +=
            self.video.iter().filter(|v| v.id > self.last_video).count() as u64;
        self.audio.clear();
        self.video.clear();
        self.audio_frames = 0;
        self.audio_origin = None;
        self.output_end = None;
        self.presented_capture = None;
        self.last_video = 0;
    }
    fn fail(&mut self, error: Error) -> Error {
        self.clear();
        self.phase = Phase::Fault(error);
        error
    }
    fn admit(&mut self, generation: u64) -> Result<(), Error> {
        if generation != self.generation {
            self.metrics.rejected += 1;
            return Err(Error::Stale);
        }
        if self.phase != Phase::Running {
            return Err(Error::Off);
        }
        Ok(())
    }
}

/// One native session, default off. Borrowed handoff holds the same generation
/// lock as Stop. The callback must be short, nonblocking, and must not reenter.
/// Device teardown/watchdogs remain the endpoint owner's separate responsibility.
pub struct Timeline {
    inner: Mutex<State>,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            inner: Mutex::new(State {
                generation: 0,
                phase: Phase::Off,
                device: 0,
                audio: VecDeque::new(),
                video: VecDeque::new(),
                audio_frames: 0,
                audio_origin: None,
                output_end: None,
                presented_capture: None,
                last_video: 0,
                metrics: Metrics::default(),
            }),
        }
    }
}
impl Timeline {
    pub fn snapshot(&self) -> Snapshot {
        let s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            generation: s.generation,
            phase: s.phase,
            audio_frames: s.audio_frames,
            video_frames: s.video.len(),
            metrics: s.metrics,
        }
    }
    /// Call only after explicit native intent and separate endpoint preparation.
    pub fn begin(&self, expected_generation: u64, selected_microphone: u64) -> Result<u64, Error> {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if s.generation != expected_generation {
            return Err(Error::Stale);
        }
        if s.phase != Phase::Off {
            return Err(Error::Off);
        }
        if selected_microphone == 0 {
            return Err(Error::Source);
        }
        s.generation = s.generation.checked_add(1).ok_or(Error::Exhausted)?;
        s.clear();
        s.device = selected_microphone;
        s.phase = Phase::Running;
        Ok(s.generation)
    }
    pub fn stop(&self) -> Result<u64, Error> {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.clear();
        s.phase = Phase::Off;
        s.device = 0;
        s.generation = s
            .generation
            .checked_add(1)
            .ok_or_else(|| s.fail(Error::Exhausted))?;
        Ok(s.generation)
    }
    /// Faults both timeline lanes; the owner must revoke both real endpoints too.
    pub fn fault(&self, generation: u64, error: Error) -> Result<(), Error> {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.admit(generation)?;
        Err(s.fail(error))
    }
    pub fn push_audio(&self, generation: u64, mut p: PcmPacket, now: Time) -> Result<(), Error> {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.admit(generation)?;
        if p.device != s.device || p.source != Source::SelfMicrophone {
            return Err(s.fail(Error::Source));
        }
        if p.rate != RATE
            || p.channels != 1
            || p.samples.is_empty()
            || p.samples.len() > MAX_PACKET_FRAMES
            || p.first_sample.checked_add(p.samples.len() as u64).is_none()
        {
            return Err(s.fail(Error::Format));
        }
        if p.flags.discontinuity {
            return Err(s.fail(Error::Discontinuity));
        }
        if p.flags.timestamp_error || p.capture.0 > now.0 || now.0 - p.capture.0 > MAX_AGE {
            return Err(s.fail(Error::Clock));
        }
        if p.flags.silent {
            p.samples.fill(0.0);
        } else if p.samples.iter().any(|v| !v.is_finite() || v.abs() > 1.0) {
            return Err(s.fail(Error::Format));
        }
        if let Some(previous) = s.audio.back() {
            if p.first_sample != previous.first_sample + previous.samples.len() as u64 {
                return Err(s.fail(Error::Discontinuity));
            }
            let dt = p.capture.0.checked_sub(previous.capture.0).unwrap_or(0);
            if dt == 0 || !clock::valid_rate(previous.samples.len() as u64, dt, u64::from(RATE)) {
                return Err(s.fail(Error::Clock));
            }
        }
        if s.audio_frames + p.samples.len() > MAX_AUDIO_FRAMES {
            return Err(s.fail(Error::AudioOverflow));
        }
        s.audio_frames += p.samples.len();
        s.audio_origin.get_or_insert(p.first_sample);
        s.metrics.admitted_audio += p.samples.len() as u64;
        s.metrics.audio_peak = s.metrics.audio_peak.max(s.audio_frames);
        s.audio.push_back(p);
        Ok(())
    }
    /// Returns false for expired/replayed video; PCM is never replaced this way.
    pub fn push_video(
        &self,
        generation: u64,
        frame: CorrectedFrame,
        now: Time,
    ) -> Result<bool, Error> {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.admit(generation)?;
        if frame.nv12.len() != NV12_BYTES || frame.id == 0 {
            return Err(s.fail(Error::Format));
        }
        if frame.capture.0 > now.0
            || now.0 - frame.capture.0 > MAX_AGE
            || frame.id <= s.last_video
            || s.video
                .iter()
                .any(|v| v.id == frame.id || v.capture == frame.capture)
            || s.presented_capture.is_some_and(|t| frame.capture < t)
        {
            s.metrics.dropped_video += 1;
            return Ok(false);
        }
        let index = s
            .video
            .iter()
            .position(|v| v.capture > frame.capture)
            .unwrap_or(s.video.len());
        // Capture order and frame identity must agree even for out-of-order completion.
        if (index > 0 && s.video[index - 1].id >= frame.id)
            || (index < s.video.len() && s.video[index].id <= frame.id)
        {
            s.metrics.dropped_video += 1;
            return Ok(false);
        }
        s.video.insert(index, frame);
        s.metrics.admitted_video += 1;
        if s.video.len() > MAX_VIDEO_FRAMES {
            s.video.pop_front();
            s.metrics.dropped_video += 1;
        }
        s.metrics.video_peak = s.metrics.video_peak.max(s.video.len());
        Ok(true)
    }
    /// None explicitly requests silence + neutral video. A successful callback
    /// means handoff only, NOT device consumption or meeting receipt. Consumers
    /// cannot borrow data beyond this callback, and Stop shares this lock.
    pub fn with_window<F>(
        &self,
        generation: u64,
        output: OutputInterval,
        now: Time,
        consume: F,
    ) -> Result<bool, Error>
    where
        F: FnOnce(Option<Window<'_>>) -> bool,
    {
        self.dispatch(generation, output, now, |view| {
            let ready = view.is_some();
            if consume(view) {
                Ok(ready)
            } else {
                Err(Error::Sink)
            }
        })
    }

    // The callback reports whether media or neutral/silence was actually handed
    // off. A filter underrun must not count as successful audio/video delivery.
    fn dispatch<F>(
        &self,
        generation: u64,
        output: OutputInterval,
        now: Time,
        consume: F,
    ) -> Result<bool, Error>
    where
        F: FnOnce(Option<Window<'_>>) -> Result<bool, Error>,
    {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.admit(generation)?;
        if output.generation != generation {
            s.metrics.rejected += 1;
            return Err(Error::Stale);
        }
        if now.0 < output.observed.0
            || now.0 - output.observed.0 > HNS_PER_SECOND / 10
            || output.time < now
        {
            return Err(s.fail(Error::Clock));
        }
        if s.output_end
            .is_some_and(|(end, f)| f != output.frequency || output.start < end)
        {
            return Err(s.fail(Error::OutputOrder));
        }
        let first = Time(output.time.0.saturating_sub(DELAY));
        if s.presented_capture.is_some_and(|t| first < t) {
            return Err(s.fail(Error::Clock));
        }
        // Retire past video, but keep the most recent at/before this source time.
        while s.video.len() > 1 && s.video[1].capture <= first {
            let old = s.video.pop_front().unwrap();
            if old.id > s.last_video {
                s.metrics.dropped_video += 1;
            }
        }
        let video = s.video.front().filter(|v| {
            v.capture <= first
                && first.0 - v.capture.0 <= MAX_VIDEO_LAG
                && now.0 >= v.capture.0
                && now.0 - v.capture.0 <= MAX_AGE
        });
        let ready = output.time.0 >= DELAY
            && video.is_some()
            && position(&s.audio, first, output.period, 0).is_some()
            && position(&s.audio, first, output.period, output.frames - 1).is_some();
        let id = if ready { video.unwrap().id } else { 0 };
        let view = if ready {
            Some(Window {
                generation,
                first_capture: first,
                first_presentation: output.time,
                frames: output.frames,
                video: video.unwrap(),
                packets: &s.audio,
                period: output.period,
                audio_origin: s.audio_origin.unwrap(),
            })
        } else {
            None
        };
        let delivered =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| consume(view))) {
                Ok(Ok(delivered)) if !delivered || ready => delivered,
                Ok(Err(error)) => return Err(s.fail(error)),
                _ => return Err(s.fail(Error::Sink)),
            };
        s.output_end = Some((output.end, output.frequency));
        s.presented_capture = Some(first);
        s.metrics.windows += 1;
        if delivered {
            s.metrics.output_frames += output.frames as u64;
            if id == s.last_video {
                s.metrics.repeated_video += 1;
            } else {
                s.metrics.unique_video += 1;
                s.last_video = id;
            }
        } else {
            s.metrics.silence_frames += output.frames as u64;
            if ready {
                s.metrics.filter_underrun_windows += 1;
            }
        }
        // Retain two anchors and lookbehind. A passed deadline is never replayed,
        // even if its sink used silence. retired_audio is not a delivery counter.
        while s.audio.len() > 2 && s.audio[1].capture.0 <= first.0.saturating_sub(FILTER_HISTORY) {
            let p = s.audio.pop_front().unwrap();
            s.audio_frames -= p.samples.len();
            s.metrics.retired_audio += p.samples.len() as u64;
        }
        Ok(delivered)
    }
}
