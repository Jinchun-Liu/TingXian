//! Correlate a device's position units with QPC, without treating units as samples.
use super::{Error, Time, HNS_PER_SECOND, RATE};

const MAX_OBSERVATION_GAP: u64 = HNS_PER_SECOND / 10;
const MAX_PPM: u128 = 2_000;

/// Explicit 100 ns QPC domain. Not GetTickCount64, UTC, or arbitrary MF PTS.
impl Time {
    pub fn from_qpc_ticks(ticks: u64, frequency: u64) -> Result<Self, Error> {
        if frequency == 0 {
            return Err(Error::Clock);
        }
        u64::try_from(u128::from(ticks) * u128::from(HNS_PER_SECOND) / u128::from(frequency))
            .map(Self)
            .map_err(|_| Error::Clock)
    }
}

#[derive(Clone, Copy)]
struct Reading {
    position: u64,
    time: Time,
}

/// One instance per opened device stream. Invalid observations permanently fault
/// this instance; a new session must create a new clock after device preparation.
pub struct DeviceClock {
    generation: u64,
    frequency: u64,
    last: Option<Reading>,
    delta: Option<(u64, u64)>,
    failed: bool,
}

/// A bounded output interval, constructed only by a validated device clock.
pub struct OutputInterval {
    pub(super) generation: u64,
    pub(super) time: Time,
    pub(super) period: SamplePeriod,
    pub(super) frames: usize,
    pub(super) start: u128,
    pub(super) end: u128,
    pub(super) frequency: u64,
    pub(super) observed: Time,
}
/// Keep the measured sample period rational until the final interpolation
/// fraction. Floating point flooring can select the preceding sample even when
/// capture and output clocks align exactly.
#[derive(Clone, Copy)]
pub(super) struct SamplePeriod {
    pub numerator: u128,
    pub denominator: u128,
}
impl OutputInterval {
    pub fn first_sample_time(&self) -> Time {
        self.time
    }
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn sample_period_100ns(&self) -> f64 {
        self.period.numerator as f64 / self.period.denominator as f64
    }
}

impl DeviceClock {
    pub fn is_ready(&self) -> bool {
        !self.failed && self.delta.is_some()
    }
    pub fn new(generation: u64, position_frequency: u64) -> Result<Self, Error> {
        if generation == 0 || position_frequency == 0 {
            return Err(Error::Clock);
        }
        Ok(Self {
            generation,
            frequency: position_frequency,
            last: None,
            delta: None,
            failed: false,
        })
    }
    pub fn observe(&mut self, position: u64, qpc: Time) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Clock);
        }
        let next = Reading {
            position,
            time: qpc,
        };
        if let Some(last) = self.last {
            // GetPosition may return the same position again (including initial
            // zero). Ignore it without refreshing the advancing-clock lease.
            if position == last.position
                && qpc.0 >= last.time.0
                && qpc.0 - last.time.0 <= MAX_OBSERVATION_GAP
            {
                return Ok(());
            }
            let dp = position.checked_sub(last.position).filter(|v| *v != 0);
            let dt = qpc
                .0
                .checked_sub(last.time.0)
                .filter(|v| *v != 0 && *v <= MAX_OBSERVATION_GAP);
            match (dp, dt) {
                (Some(dp), Some(dt)) if valid_rate(dp, dt, self.frequency) => {
                    self.delta = Some((dp, dt))
                }
                _ => {
                    self.failed = true;
                    self.delta = None;
                    return Err(Error::Clock);
                }
            }
        }
        self.last = Some(next);
        Ok(())
    }
    /// `position` is in GetFrequency units, not necessarily sample frames. The
    /// endpoint adapter must supply its actual next write position, never wall time.
    pub fn interval(
        &self,
        position: u64,
        frames: usize,
        now: Time,
    ) -> Result<OutputInterval, Error> {
        self.interval_scaled(u128::from(position) * u128::from(RATE), frames, now)
    }

    /// Render clients count submitted sample frames. Preserve a fractional
    /// device position when frequency is not divisible by the sample rate.
    /// Flooring frame * frequency / RATE first silently loses timing precision.
    pub fn interval_from_frame(
        &self,
        first_frame: u64,
        frames: usize,
        now: Time,
    ) -> Result<OutputInterval, Error> {
        self.interval_scaled(
            u128::from(first_frame) * u128::from(self.frequency),
            frames,
            now,
        )
    }

    fn interval_scaled(
        &self,
        start: u128,
        frames: usize,
        now: Time,
    ) -> Result<OutputInterval, Error> {
        if self.failed || frames == 0 || frames > super::MAX_PACKET_FRAMES {
            return Err(Error::Clock);
        }
        let (dp, dt) = self.delta.ok_or(Error::ClockNotReady)?;
        let last = self.last.ok_or(Error::ClockNotReady)?;
        let age = now
            .0
            .checked_sub(last.time.0)
            .filter(|v| *v <= MAX_OBSERVATION_GAP)
            .ok_or(Error::Clock)?;
        let offset = start
            .checked_sub(u128::from(last.position) * u128::from(RATE))
            .ok_or(Error::Clock)?;
        let delay = offset.checked_mul(u128::from(dt)).ok_or(Error::Clock)?
            / (u128::from(dp) * u128::from(RATE));
        if delay > u128::from(MAX_OBSERVATION_GAP) {
            return Err(Error::Clock);
        }
        let time = last.time.0.checked_add(delay as u64).ok_or(Error::Clock)?;
        // Old observations are not permission to schedule into the past.
        if time < now.0 || age > MAX_OBSERVATION_GAP {
            return Err(Error::Clock);
        }
        let period = SamplePeriod {
            numerator: u128::from(dt) * u128::from(self.frequency),
            denominator: u128::from(dp) * u128::from(RATE),
        };
        let duration =
            u64::try_from((frames as u128 * period.numerator).div_ceil(period.denominator))
                .map_err(|_| Error::Clock)?;
        if time.checked_add(duration).ok_or(Error::Clock)? - now.0 > MAX_OBSERVATION_GAP {
            return Err(Error::Clock);
        }
        Ok(OutputInterval {
            generation: self.generation,
            time: Time(time),
            period,
            frames,
            start,
            end: start
                .checked_add(frames as u128 * u128::from(self.frequency))
                .ok_or(Error::Clock)?,
            frequency: self.frequency,
            observed: last.time,
        })
    }
}

pub(super) fn valid_rate(position_delta: u64, time_delta: u64, frequency: u64) -> bool {
    let measured = u128::from(time_delta) * u128::from(frequency);
    let nominal = u128::from(position_delta) * u128::from(HNS_PER_SECOND);
    // Divide before multiplying to keep validation safe even for hostile u64s.
    // Two 100 ns timestamp readings can differ by one quantization tick. Without
    // this allowance valid tiny capture packets can falsely appear out of range.
    measured.abs_diff(nominal) <= nominal / 1_000_000 * MAX_PPM + u128::from(frequency)
}
