//! Native MTA joint owner. No capture/start webview command or virtual device
//! installation. Video queue admission gates original-voice commitment, not an
//! atomic transaction across two devices or proof of receiver synchronization.
mod barrier;
mod eye;
use crate::{
    audio::{self, AudioPair, NativeAudio, PinnedAudioLibrary},
    av_timeline::{
        self, CorrectedFrame, PcmPacket, RenderStatus, SincRenderer, Time, Timeline,
        MAX_PACKET_FRAMES,
    },
    timed_video::{self, Admission, NativeTimedVideo, PinnedTimedLibrary, TimedEndpoint},
};
pub use barrier::Error as BarrierError;
use eye::EyeRoute;
pub use eye::EyeStatus;
use std::{
    os::windows::io::{AsRawHandle, BorrowedHandle},
    path::PathBuf,
};
use windows_sys::Win32::{Foundation::*, System::Threading::WaitForMultipleObjects};

const FRAMES: usize = 480; // Fixed 10 ms mono pending window, no unbounded queue.
const GUARD: u64 = 20_000; // 2 ms; native device-position check remains authoritative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Permission,
    Off,
    Stale,
    Source,
    Deadline,
    Revoked,
    Wait,
    Eye(crate::Reason),
    Barrier(BarrierError),
    Audio(audio::Error),
    Video(timed_video::Error),
    Admission(Admission),
    Timeline(av_timeline::Error),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    WaitingVideo,
    PrimingAudio,
    Running,
    AwaitingAdmission,
    Stopped,
    Fault,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Staged,
    Committed,
    Stopped,
}
/// Pins, endpoint IDs and barrier path are trusted native configuration only.
/// The host MUST use one stable barrier per app profile/output authority, across
/// device changes and restarts. A fresh filename is NOT a recovery mechanism.
pub struct Config {
    pub audio: PinnedAudioLibrary,
    pub video: PinnedTimedLibrary,
    pub pair: AudioPair,
    pub channels: u32,
    pub selected_microphone: u64,
    pub barrier_path: PathBuf,
}
/// An idle, native-only candidate, not output readiness or authorization. No
/// pipe, endpoint client, timeline generation or durable exposure mark exists.
/// Keep on the MTA owner lane; activate rechecks the inventory before exposure.
pub struct Candidate {
    audio: NativeAudio,
    video: NativeTimedVideo,
    pair: AudioPair,
    channels: u32,
    selected_microphone: u64,
    barrier_path: PathBuf,
    renderer: SincRenderer,
}
#[derive(Clone, Copy, Debug)]
pub struct Cleanup {
    pub timeline: Result<u64, av_timeline::Error>,
    pub audio: Result<audio::Status, audio::Error>,
    pub video: Result<timed_video::Status, timed_video::Error>,
    pub eye: Option<Result<(), crate::Reason>>,
    /// Stays true until a future, independently validated recovery protocol.
    pub restart_blocked: bool,
}
impl Cleanup {
    /// Local cleanup only. Successful reset never proves external device drain.
    fn failure(&self) -> Option<Error> {
        if let Err(e) = self.timeline {
            return Some(Error::Timeline(e));
        }
        let audio = match self.audio {
            Ok(status) => status,
            Err(e) => return Some(Error::Audio(e)),
        };
        for hr in [audio.stop_error, audio.reset_error] {
            if hr < 0 {
                return Some(Error::Audio(audio::Error::Native(hr)));
            }
        }
        if !audio.revoked || audio.phase != 3 || !audio.client_reset {
            return Some(Error::Audio(audio::Error::Protocol));
        }
        match self.video {
            Err(e) => return Some(Error::Video(e)),
            // `pending` also denotes an unconsumed cancellation receipt. The
            // native stop joins the sender and local_clear proves payload
            // retirement; retaining that receipt is not queued media.
            Ok(status) if !status.local_clear => {
                return Some(Error::Video(timed_video::Error::Protocol));
            }
            _ => {}
        }
        self.eye.and_then(Result::err).map(Error::Eye)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub phase: Phase,
    pub generation: u64,
    pub committed_windows: u64,
    pub neutral_windows: u64,
    pub pending_samples: usize,
    pub failure: Option<Error>,
    pub cleanup: Option<Cleanup>,
    pub eye: Option<EyeStatus>,
}
#[derive(Clone, Copy, Debug)]
pub struct PrepareFailure {
    pub cause: Error,
    pub cleanup: Option<Cleanup>,
}
impl From<Error> for PrepareFailure {
    fn from(cause: Error) -> Self {
        Self {
            cause,
            cleanup: None,
        }
    }
}
struct Pending {
    ticket: Option<audio::Reservation>,
    flight: timed_video::Flight,
    samples: [f32; MAX_PACKET_FRAMES],
    frames: usize,
    presentation: Time,
    neutral: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.samples.fill(0.0);
    }
}

/// Own on one native MTA lane. All waits and device writes occur outside the
/// timeline lock. The host must service wait/advance and its stop signal; this
/// owner is not a substitute for endpoint crash watchdogs or real drain proofs.
pub struct JointOutput {
    audio: NativeAudio,
    video: NativeTimedVideo,
    timeline: Timeline,
    renderer: SincRenderer,
    _barrier: barrier::Barrier,
    generation: u64,
    phase: Phase,
    video_ticket: Option<timed_video::Reservation>,
    pending: Option<Pending>,
    committed: u64,
    neutral: u64,
    failure: Option<Error>,
    cleanup: Option<Cleanup>,
    prime_deadline: Option<u64>,
    eye: Option<EyeRoute>,
}
impl Candidate {
    /// Only reads native component pins and current endpoint metadata. Failure
    /// leaves the profile untouched and can be retried with a corrected choice.
    pub fn inspect(config: Config) -> Result<Self, Error> {
        if config.selected_microphone == 0 || !(1..=2).contains(&config.channels) {
            return Err(Error::Source);
        }
        let mut audio = NativeAudio::load(config.audio).map_err(Error::Audio)?;
        let video = NativeTimedVideo::load(config.video).map_err(Error::Video)?;
        audio
            .couple_video(video.cancellation())
            .map_err(Error::Audio)?;
        audio.inspect_pair(&config.pair).map_err(Error::Audio)?;
        let renderer = SincRenderer::default();
        Ok(Self {
            audio,
            video,
            pair: config.pair,
            channels: config.channels,
            selected_microphone: config.selected_microphone,
            barrier_path: config.barrier_path,
            renderer,
        })
    }
    /// Explicit intent covers BOTH outputs. Revalidate before arming; actual
    /// prepare checks again for hot-unplug/races. Once arming begins, failures
    /// retain the barrier because acquired resources may have been exposed.
    pub fn activate(
        self,
        explicit_intent: bool,
    ) -> Result<(JointOutput, TimedEndpoint), PrepareFailure> {
        if !explicit_intent {
            return Err(Error::Permission.into());
        }
        self.audio.inspect_pair(&self.pair).map_err(Error::Audio)?;
        let Self {
            audio,
            video,
            pair,
            channels,
            selected_microphone,
            barrier_path,
            renderer,
        } = self;
        let timeline = Timeline::default();
        let generation = timeline
            .begin(0, selected_microphone)
            .map_err(Error::Timeline)?;
        let mut barrier = barrier::Barrier::acquire(&barrier_path).map_err(Error::Barrier)?;
        barrier.arm(generation).map_err(Error::Barrier)?;
        let mut owner = JointOutput {
            audio,
            video,
            timeline,
            renderer,
            _barrier: barrier,
            generation,
            phase: Phase::WaitingVideo,
            video_ticket: None,
            pending: None,
            committed: 0,
            neutral: 0,
            failure: None,
            cleanup: None,
            prime_deadline: None,
            eye: None,
        };
        if let Err(e) = owner.audio.prepare(&pair, channels, generation) {
            let cause = owner.fail(Error::Audio(e));
            return Err(PrepareFailure {
                cause,
                cleanup: owner.cleanup,
            });
        }
        let endpoint = match owner.video.open(generation, true) {
            Ok(e) => e,
            Err(e) => {
                let cause = owner.fail(Error::Video(e));
                return Err(PrepareFailure {
                    cause,
                    cleanup: owner.cleanup,
                });
            }
        };
        Ok((owner, endpoint))
    }
}
impl JointOutput {
    /// Convenience for an immediate explicit start. A failed read-only preflight
    /// does not arm the durable barrier or require fabricated drain evidence.
    pub fn prepare(
        config: Config,
        explicit_intent: bool,
    ) -> Result<(Self, TimedEndpoint), PrepareFailure> {
        if !explicit_intent {
            return Err(Error::Permission.into());
        }
        Candidate::inspect(config)?.activate(true)
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: self.phase,
            generation: self.generation,
            committed_windows: self.committed,
            neutral_windows: self.neutral,
            pending_samples: self.pending.as_ref().map_or(0, |p| p.frames),
            failure: self.failure,
            cleanup: self.cleanup,
            eye: self.eye.as_ref().map(EyeRoute::snapshot),
        }
    }
    /// Bind once, before output starts. The reader moves to a dedicated native
    /// lane; subsequent manual video submission cannot bypass this source.
    pub fn bind_corrections(&mut self, reader: crate::NativeCorrections) -> Result<(), Error> {
        self.active()?;
        if self.eye.is_some()
            || self.phase != Phase::WaitingVideo
            || self.video_ticket.is_some()
            || self.timeline.snapshot().metrics.admitted_video != 0
        {
            return Err(Error::Source);
        }
        match EyeRoute::start(reader, self.video.cancellation()) {
            Ok(route) => {
                self.eye = Some(route);
                Ok(())
            }
            Err(e) => Err(self.fail(Error::Eye(e))),
        }
    }
    fn active(&self) -> Result<(), Error> {
        if self.cleanup.is_some() {
            Err(Error::Off)
        } else {
            Ok(())
        }
    }
    pub fn push_audio(&mut self, generation: u64, packet: PcmPacket) -> Result<(), Error> {
        self.active()?;
        if generation != self.generation {
            return Err(Error::Stale);
        }
        let result = audio::native_now().map_err(Error::Audio).and_then(|now| {
            self.timeline
                .push_audio(generation, packet, now)
                .map_err(Error::Timeline)
        });
        result.map_err(|e| self.fail(e))
    }
    pub fn push_video(&mut self, generation: u64, frame: CorrectedFrame) -> Result<bool, Error> {
        self.active()?;
        if generation != self.generation {
            return Err(Error::Stale);
        }
        if self.eye.is_some() {
            return Err(self.fail(Error::Source));
        }
        let result = audio::native_now().map_err(Error::Audio).and_then(|now| {
            self.timeline
                .push_video(generation, frame, now)
                .map_err(Error::Timeline)
        });
        result.map_err(|e| self.fail(e))
    }
    /// One bounded state transition, never a spin/wait under the timeline lock.
    pub fn advance(&mut self) -> Result<Progress, Error> {
        self.active()?;
        let result = (|| {
            let permit = if let Some(route) = &self.eye {
                let Some(permit) = route.admit().map_err(Error::Eye)? else {
                    return Ok(Progress::Waiting);
                };
                if let Some(frame) = route.take().map_err(Error::Eye)? {
                    self.timeline
                        .push_video(
                            self.generation,
                            frame,
                            audio::native_now().map_err(Error::Audio)?,
                        )
                        .map_err(Error::Timeline)?;
                }
                permit.check().map_err(Error::Eye)?;
                Some(permit)
            } else {
                None
            };
            let result = self.advance_inner();
            drop(permit); // Never report owner cleanup while its admission lives.
            result
        })();
        match result {
            Ok(p) => Ok(p),
            Err(e) => Err(self.fail(e)),
        }
    }
    fn advance_inner(&mut self) -> Result<Progress, Error> {
        if self.audio.revocation().revoked() {
            return Err(Error::Revoked);
        }
        if let Some(deadline) = self.prime_deadline {
            if audio::native_now().map_err(Error::Audio)?.qpc_100ns() >= deadline {
                return Err(Error::Deadline);
            }
        }
        if let Some(pending) = self.pending.as_mut() {
            // A delayed receipt must not revive an expired audio reservation.
            let now = audio::native_now().map_err(Error::Audio)?;
            if now.qpc_100ns().saturating_add(GUARD) >= pending.presentation.qpc_100ns() {
                return Err(Error::Deadline);
            }
            match self.video.take(&mut pending.flight).map_err(Error::Video)? {
                None => return Ok(Progress::Waiting),
                Some(Admission::Admitted) => {}
                Some(other) => return Err(Error::Admission(other)),
            }
            // Native commit re-observes the device clock/guard and can still
            // fail after video admission. Such a partial outcome stops BOTH.
            let mut pending = self.pending.take().unwrap();
            if let Some(route) = &self.eye {
                route.check().map_err(Error::Eye)?;
            }
            self.audio
                .commit(
                    pending.ticket.take().unwrap(),
                    &pending.samples[..pending.frames],
                )
                .map_err(Error::Audio)?;
            self.committed += 1;
            self.neutral += u64::from(pending.neutral);
            self.phase = Phase::Running;
            return Ok(Progress::Committed);
        }
        if self.video_ticket.is_none() {
            self.video_ticket = self.video.reserve().map_err(Error::Video)?;
            if self.video_ticket.is_none() {
                return Ok(Progress::Waiting);
            }
        }
        if self.phase == Phase::WaitingVideo {
            let start = audio::native_now().map_err(Error::Audio)?.qpc_100ns();
            if let Some(route) = &self.eye {
                route.check().map_err(Error::Eye)?;
            }
            self.audio.start(true).map_err(Error::Audio)?;
            self.prime_deadline = Some(start.saturating_add(200_000 - GUARD));
            self.phase = Phase::PrimingAudio;
        }
        let Some(mut ticket) = self.audio.reserve(FRAMES).map_err(Error::Audio)? else {
            return Ok(Progress::Waiting);
        };
        let interval = ticket
            .take_interval()
            .ok_or(Error::Audio(audio::Error::Protocol))?;
        self.prime_deadline = None;
        let presentation = interval.first_sample_time();
        let now = audio::native_now().map_err(Error::Audio)?;
        if now.qpc_100ns().saturating_add(GUARD) >= presentation.qpc_100ns() {
            return Err(Error::Deadline);
        }
        let video_ticket = self.video_ticket.take().unwrap();
        let mut samples = [0.0; MAX_PACKET_FRAMES];
        let mut staged = None;
        let mut neutral = true;
        let rendered = self.timeline.with_rendered_window(
            self.generation,
            interval,
            now,
            &self.renderer,
            |w| {
                // Only the bounded copies occur here, with no pipe I/O, device
                // commit, join, timeline reentry or independent lane acceptance.
                samples[..FRAMES].copy_from_slice(w.audio);
                neutral = w.status != RenderStatus::Media;
                staged = Some(self.video.stage(video_ticket, &w));
                staged.as_ref().unwrap().is_ok()
            },
        );
        if let Some(Err(e)) = staged.as_ref() {
            samples.fill(0.0);
            return Err(Error::Video(*e));
        }
        if let Err(e) = rendered {
            samples.fill(0.0);
            return Err(Error::Timeline(e));
        }
        self.pending = Some(Pending {
            ticket: Some(ticket),
            flight: staged.unwrap().unwrap(),
            samples,
            frames: FRAMES,
            presentation,
            neutral,
        });
        samples.fill(0.0);
        self.phase = Phase::AwaitingAdmission;
        Ok(Progress::Staged)
    }
    /// Wait for caller stop, video receipt/fault, or audio readiness. A bounded
    /// timeout services deadlines/disconnects even if an endpoint stops signaling.
    /// Stop is first in the wait set and never commits another pending window.
    pub fn wait(&mut self, stop: BorrowedHandle<'_>) -> Result<Progress, Error> {
        self.active()?;
        let result = (|| {
            let mut handles = [
                stop.as_raw_handle(),
                self.video.event().map_err(Error::Video)?.as_raw_handle(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ];
            let mut count = 2;
            if let Some(event) = self.audio.ready_event().map_err(Error::Audio)? {
                handles[2] = event.as_raw_handle();
                count = 3;
            }
            if let Some(route) = &self.eye {
                handles[count as usize] = route.event().as_raw_handle();
                count += 1;
            }
            let result = unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, 5) };
            if result == WAIT_OBJECT_0 {
                return Ok(true);
            }
            if result == WAIT_TIMEOUT || (result > WAIT_OBJECT_0 && result < WAIT_OBJECT_0 + count)
            {
                Ok(false)
            } else {
                Err(Error::Wait)
            }
        })();
        match result {
            Ok(true) => {
                self.stop();
                self.failure.map_or(Ok(Progress::Stopped), Err)
            }
            Ok(false) => self.advance(),
            Err(e) => Err(self.fail(e)),
        }
    }
    fn fail(&mut self, error: Error) -> Error {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
        self.stop();
        self.phase = Phase::Fault;
        error
    }
    pub fn stop(&mut self) -> Cleanup {
        if let Some(result) = self.cleanup {
            return result;
        }
        // Fast video revocation before cleanup. No device wait while timeline
        // is locked; preserve all three results instead of returning early.
        if let Some(route) = &self.eye {
            route.request_stop();
        }
        self.video.cancel();
        let timeline = self.timeline.stop();
        self.pending = None;
        self.video_ticket = None;
        self.prime_deadline = None;
        let audio = self.audio.stop();
        let video = self.video.stop();
        let eye = self.eye.as_mut().map(EyeRoute::retire);
        let result = Cleanup {
            timeline,
            audio,
            video,
            eye,
            restart_blocked: true,
        };
        self.cleanup = Some(result);
        if self.failure.is_none() {
            self.failure = result.failure();
        }
        self.phase = if self.failure.is_some() {
            Phase::Fault
        } else {
            Phase::Stopped
        };
        result
    }
}
impl Drop for JointOutput {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
