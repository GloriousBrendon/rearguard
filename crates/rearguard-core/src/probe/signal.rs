//! The two candidate signal shapes, each producing a unit signal in Q30 `[-ONE, ONE]`.

use core::fmt;

use chacha20::ChaCha20Rng;
use chacha20::rand_core::{Rng, SeedableRng};
use zeroize::Zeroize;

use super::SignalShape;
use super::fixed::{ONE, cubic_bspline, sin_turns};
use crate::secret::SecretKey;

/// Most sinusoids a [`SignalShape::SinusoidSum`] may use.
pub const MAX_SINUSOIDS: u8 = 8;

/// ChaCha20 seeded from a stream key, positioned explicitly for every read.
fn stream_rng(key: &SecretKey) -> ChaCha20Rng {
    let mut seed = *key.expose_secret();
    let rng = ChaCha20Rng::from_seed(seed);
    seed.zeroize();
    rng
}

/// Unit signal for one stream: a pure function of (stream key, tick).
pub(crate) enum StreamSignal {
    Noise(Box<NoiseStream>),
    Sinusoids(SinusoidBank),
}

impl StreamSignal {
    /// `shape` must already be validated.
    pub(crate) fn new(key: &SecretKey, shape: &SignalShape) -> Self {
        match *shape {
            SignalShape::BandLimitedNoise {
                knot_interval_ticks,
            } => Self::Noise(Box::new(NoiseStream {
                rng: stream_rng(key),
                interval: u64::from(knot_interval_ticks.get()),
                window: None,
            })),
            SignalShape::SinusoidSum {
                components,
                min_period_ticks,
                max_period_ticks,
            } => Self::Sinusoids(SinusoidBank::new(
                key,
                components,
                min_period_ticks,
                max_period_ticks,
            )),
        }
    }

    pub(crate) fn unit(&mut self, tick: u64) -> i64 {
        match self {
            Self::Noise(noise) => noise.unit(tick),
            Self::Sinusoids(bank) => bank.unit(tick),
        }
    }
}

impl fmt::Debug for StreamSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Noise(_) => f.write_str("Noise(<redacted>)"),
            Self::Sinusoids(_) => f.write_str("Sinusoids(<redacted>)"),
        }
    }
}

/// Band-limited noise: independent uniform knots every `interval` ticks, joined by a
/// uniform cubic B-spline (C2-smooth, low-pass, bounded by the knots).
///
/// Knot `j` is the `j`-th 64-bit word pair of the ChaCha20 keystream, so any knot is
/// reachable in O(1) and the value at a tick does not depend on earlier calls.
pub(crate) struct NoiseStream {
    rng: ChaCha20Rng,
    interval: u64,
    /// The last knot window used, as a cache for sequential ticks.
    window: Option<(u64, [i64; 4])>,
}

impl NoiseStream {
    fn unit(&mut self, tick: u64) -> i64 {
        let segment = tick / self.interval;
        // (tick % interval) < 2^32, so the shift cannot overflow.
        let u = (((tick % self.interval) << 30) / self.interval) as i64;
        let knots = self.knots(segment);
        cubic_bspline(&knots, u)
    }

    /// Knots `segment .. segment + 4`, each uniform in `[-ONE, ONE)`.
    fn knots(&mut self, segment: u64) -> [i64; 4] {
        if let Some((cached, knots)) = self.window
            && cached == segment
        {
            return knots;
        }
        self.rng.set_word_pos(u128::from(segment) * 2);
        let mut knots = [0i64; 4];
        for knot in &mut knots {
            *knot = (self.rng.next_u64() >> 33) as i64 - ONE;
        }
        self.window = Some((segment, knots));
        knots
    }
}

impl Drop for NoiseStream {
    fn drop(&mut self) {
        if let Some((segment, knots)) = &mut self.window {
            segment.zeroize();
            knots.zeroize();
        }
    }
}

/// Sum of `n` sinusoids with secret frequencies (uniform in the configured band) and
/// secret phases, each weighted `1/n`. Frequencies are 64-bit random, so in practice
/// they are incommensurate and the sum never repeats.
pub(crate) struct SinusoidBank {
    /// Phase advance per tick; a full turn is 2^64.
    step: [u64; MAX_SINUSOIDS as usize],
    phase: [u64; MAX_SINUSOIDS as usize],
    len: usize,
}

impl SinusoidBank {
    fn new(key: &SecretKey, components: u8, min_period: u32, max_period: u32) -> Self {
        let turn = 1u128 << 64;
        // min_period >= 2, so the fastest step is at most half a turn.
        let slowest = turn / u128::from(max_period);
        let span = turn / u128::from(min_period) - slowest;
        let mut rng = stream_rng(key);
        let mut bank = Self {
            step: [0; MAX_SINUSOIDS as usize],
            phase: [0; MAX_SINUSOIDS as usize],
            len: usize::from(components),
        };
        for i in 0..bank.len {
            let r = u128::from(rng.next_u64());
            bank.step[i] = (slowest + ((r * span) >> 64)) as u64;
            bank.phase[i] = rng.next_u64();
        }
        bank
    }

    fn unit(&self, tick: u64) -> i64 {
        let sum: i64 = (0..self.len)
            .map(|i| sin_turns(self.phase[i].wrapping_add(self.step[i].wrapping_mul(tick))))
            .sum();
        sum / self.len as i64
    }
}

impl Drop for SinusoidBank {
    fn drop(&mut self) {
        self.step.zeroize();
        self.phase.zeroize();
    }
}
