//! Absolute-time sinc rendering. Kernel is prepared once outside the callback;
//! original PCM exists only in the timeline and this bounded, scoped scratch.
use super::*;
use rubato::{
    sinc_interpolator::{ScalarInterpolator, SincInterpolator},
    WindowFunction,
};

const TAPS: usize = 256;
const PHASES: usize = 256;
const HALF: usize = TAPS / 2;
const INPUT_CAPACITY: usize = MAX_PACKET_FRAMES + TAPS + 32;
// Guard filter changes against silently exceeding the timeline's retained past.
const _: () = assert!((HALF as u64 * HNS_PER_SECOND).div_ceil(RATE as u64) + 100 < FILTER_HISTORY);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderStatus {
    Media,
    MissingTimelineInput,
    MissingFilterInput,
}

/// One atomic borrowed handoff: never independently accept audio and video.
/// None video means neutral output, never original camera or the previous frame.
pub struct RenderedWindow<'a> {
    pub generation: u64,
    pub capture: Time,
    pub presentation: Time,
    pub audio: &'a [f32],
    pub video: Option<&'a CorrectedFrame>,
    pub status: RenderStatus,
}

/// Prepared 48 kHz drift-compensation filter, not a generic format converter.
/// No media or cumulative phase is retained. Its centered 128-sample lookahead
/// must fit inside the timeline buffer; it does not silently shift output PTS.
pub struct SincRenderer {
    kernel: ScalarInterpolator<f64>,
}
impl Default for SincRenderer {
    fn default() -> Self {
        Self {
            kernel: ScalarInterpolator::new(TAPS, PHASES, 0.94, WindowFunction::BlackmanHarris2),
        }
    }
}

struct Scratch {
    input: [f64; INPUT_CAPACITY],
    output: [f32; MAX_PACKET_FRAMES],
}
impl Scratch {
    fn new() -> Self {
        Self {
            input: [0.0; INPUT_CAPACITY],
            output: [0.0; MAX_PACKET_FRAMES],
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        self.input.fill(0.0);
        self.output.fill(0.0);
    }
}

impl SincRenderer {
    pub const fn lookahead_frames(&self) -> usize {
        HALF
    }

    fn render(&self, window: &Window<'_>, scratch: &mut Scratch) -> Result<bool, Error> {
        let first = window.position_at(0).ok_or(Error::Render)?;
        let last = window.position_at(window.frames - 1).ok_or(Error::Render)?;
        let start = i128::from(first.sample) - HALF as i128;
        let end = i128::from(last.sample) + HALF as i128 + 1; // exclusive
        let length = usize::try_from(end - start).map_err(|_| Error::Render)?;
        // Rubato also requires one unused trailing element. No unbounded copy or
        // extrapolation if the measured rate lies outside this prepared support.
        if length + 1 > INPUT_CAPACITY {
            return Err(Error::Render);
        }
        let mut cursor = start.max(i128::from(window.audio_origin));
        for (offset, samples) in window.packets() {
            let from = i128::from(offset).max(cursor);
            let to = (i128::from(offset) + samples.len() as i128).min(end);
            if to <= from {
                continue;
            }
            if from != cursor {
                return Ok(false);
            }
            let dest = (from - start) as usize;
            let src = (from - i128::from(offset)) as usize;
            for (out, value) in scratch.input[dest..dest + (to - from) as usize]
                .iter_mut()
                .zip(&samples[src..src + (to - from) as usize])
            {
                *out = f64::from(*value);
            }
            cursor = to;
            if cursor == end {
                break;
            }
        }
        if cursor < end {
            return Ok(false);
        }
        // Only pre-stream history is zero-extended. Missing retired or future
        // samples are an underrun; they are never fabricated or repeated.
        for (frame, position) in window.positions().enumerate() {
            let position = position.ok_or(Error::Render)?;
            let index = usize::try_from(i128::from(position.sample) - HALF as i128 - start)
                .map_err(|_| Error::Render)?;
            let phase = position.fraction * PHASES as f64;
            let lower = phase.floor() as usize;
            if lower >= PHASES || index + TAPS + 1 >= INPUT_CAPACITY {
                return Err(Error::Render);
            }
            // Rubato 0.15's subindex k is centered at HALF-1+(k+1)/PHASES.
            // Compensate that origin explicitly, rather than accepting a one-
            // sample filter delay. Tests use analytic waves and an impulse oracle.
            let left = if lower == 0 {
                self.kernel
                    .get_sinc_interpolated(&scratch.input, index, PHASES - 1)
            } else {
                self.kernel
                    .get_sinc_interpolated(&scratch.input, index + 1, lower - 1)
            };
            let right = self
                .kernel
                .get_sinc_interpolated(&scratch.input, index + 1, lower);
            let value = left + (right - left) * (phase - lower as f64);
            if !value.is_finite() {
                return Err(Error::Render);
            }
            scratch.output[frame] = value as f32;
        }
        Ok(true)
    }
}

impl Timeline {
    /// The consumer must atomically admit both lanes to a bounded native output
    /// owner, or reject the whole handoff. It must not block or call this timeline.
    /// Actual endpoint draining/revocation remains mandatory outside this core.
    pub fn with_rendered_window<F>(
        &self,
        generation: u64,
        output: OutputInterval,
        now: Time,
        renderer: &SincRenderer,
        consume: F,
    ) -> Result<bool, Error>
    where
        F: FnOnce(RenderedWindow<'_>) -> bool,
    {
        let frames = output.frames;
        let presentation = output.time;
        let capture = Time(presentation.0.saturating_sub(DELAY));
        self.dispatch(generation, output, now, |view| {
            let mut scratch = Scratch::new();
            let status = match view.as_ref() {
                Some(window) if renderer.render(window, &mut scratch)? => RenderStatus::Media,
                Some(_) => RenderStatus::MissingFilterInput,
                None => RenderStatus::MissingTimelineInput,
            };
            let media = status == RenderStatus::Media;
            if !media {
                scratch.output.fill(0.0);
            }
            let output = RenderedWindow {
                generation,
                capture,
                presentation,
                audio: &scratch.output[..frames],
                video: if media {
                    view.as_ref().map(|w| w.video)
                } else {
                    None
                },
                status,
            };
            if !consume(output) {
                return Err(Error::Sink);
            }
            Ok(media)
        })
    }
}

#[cfg(test)]
mod tests;
