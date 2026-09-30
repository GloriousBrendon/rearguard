// SPDX-License-Identifier: MIT OR Apache-2.0

//! Input probe: a secret, server-seeded drift applied to mouse sensitivity and recoil.
//!
//! The drift is a pure function of an [`EpochSeed`] and an epoch-local tick, so client
//! and server compute the same values with no per-frame communication. It is computed
//! with integer arithmetic only and is bit-identical on every platform.
//!
//! A *tick* is the time index. The default shapes assume 1 tick = 1 ms, measured from
//! the start of the epoch; use [`EpochSchedule::locate`] to turn a match tick into an
//! epoch number and epoch-local tick.
//!
//! ```
//! use rearguard_core::probe::{ProbeConfig, ProbeGenerator, RootSeed, Stream};
//!
//! // Server: derive this player's seed for epoch 0 and deliver it to their client.
//! let root = RootSeed::from_bytes(&mut [42; 32]);
//! let seed = root.match_key(b"match-1").player_key(7).epoch_seed(0);
//!
//! // Client and server: same seed, same config, same drift.
//! let config = ProbeConfig::default();
//! let mut probe = ProbeGenerator::new(&seed, &config).unwrap();
//! let drift = probe.drift(Stream::Sensitivity, 1_500);
//! assert!(drift.ppb().unsigned_abs() <= config.amplitude.ppm() * 1_000);
//! let sensitivity = 2.5 * drift.multiplier();
//! # let _ = sensitivity;
//! ```

mod fixed;
mod keys;
mod signal;

#[cfg(test)]
mod tests;

use core::fmt;
use core::num::{NonZeroU32, NonZeroU64};

pub use keys::{EpochSeed, MatchKey, PlayerKey, RootSeed};
pub use signal::MAX_SINUSOIDS;
use signal::StreamSignal;

/// Peak drift, in parts per million of the nominal value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Amplitude(u32);

impl Amplitude {
    /// 0.5%: the default.
    pub const DEFAULT: Self = Self(5_000);
    /// 2%: the hard maximum. Larger drifts risk being felt by players.
    pub const MAX: Self = Self(20_000);

    /// # Errors
    /// [`ProbeError::AmplitudeTooLarge`] if `ppm` exceeds [`Amplitude::MAX`].
    pub const fn from_ppm(ppm: u32) -> Result<Self, ProbeError> {
        if ppm > Self::MAX.0 {
            return Err(ProbeError::AmplitudeTooLarge { ppm });
        }
        Ok(Self(ppm))
    }

    /// Peak drift in parts per million.
    #[must_use]
    pub const fn ppm(self) -> u32 {
        self.0
    }
}

impl Default for Amplitude {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How the drift varies over time. See `docs/probe-signal-shapes.md` for the comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignalShape {
    /// Independent random knots joined by a cubic B-spline. Recommended.
    BandLimitedNoise {
        /// Ticks between knots. Sets the bandwidth: the spectrum falls 3 dB at about
        /// `0.31 / knot_interval`.
        knot_interval_ticks: NonZeroU32,
    },
    /// A sum of sinusoids with secret, incommensurate frequencies and phases.
    SinusoidSum {
        /// Number of sinusoids, 1 to [`MAX_SINUSOIDS`].
        components: u8,
        /// Shortest period in ticks; at least 2.
        min_period_ticks: u32,
        /// Longest period in ticks; at least `min_period_ticks`.
        max_period_ticks: u32,
    },
}

impl SignalShape {
    /// Knots every 500 ticks (0.5 s at 1 ms ticks): energy mostly below ~0.6 Hz.
    pub const DEFAULT_NOISE: Self = Self::BandLimitedNoise {
        knot_interval_ticks: NonZeroU32::new(500).expect("500 is non-zero"),
    };
    /// Four sinusoids with periods of 1.6 s to 10 s (0.1 to 0.625 Hz at 1 ms ticks).
    pub const DEFAULT_SINUSOIDS: Self = Self::SinusoidSum {
        components: 4,
        min_period_ticks: 1_600,
        max_period_ticks: 10_000,
    };

    fn validate(&self) -> Result<(), ProbeError> {
        match *self {
            Self::BandLimitedNoise { .. } => Ok(()),
            Self::SinusoidSum {
                components,
                min_period_ticks,
                max_period_ticks,
            } => {
                if !(1..=MAX_SINUSOIDS).contains(&components) {
                    return Err(ProbeError::InvalidComponentCount { components });
                }
                if min_period_ticks < 2 || min_period_ticks > max_period_ticks {
                    return Err(ProbeError::InvalidPeriods {
                        min_period_ticks,
                        max_period_ticks,
                    });
                }
                Ok(())
            }
        }
    }

    /// Domain-separation tag for the stream key, so shapes never share keys.
    fn tag(&self) -> u8 {
        match self {
            Self::BandLimitedNoise { .. } => 0,
            Self::SinusoidSum { .. } => 1,
        }
    }
}

impl Default for SignalShape {
    fn default() -> Self {
        Self::DEFAULT_NOISE
    }
}

/// How match time is split into epochs, each with its own [`EpochSeed`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EpochSchedule {
    length_ticks: Option<NonZeroU64>,
}

impl EpochSchedule {
    /// One epoch (number 0) for the whole match: the v0 behaviour.
    pub const SINGLE: Self = Self { length_ticks: None };

    /// Epochs of `length_ticks` each, numbered from 0.
    #[must_use]
    pub const fn fixed(length_ticks: NonZeroU64) -> Self {
        Self {
            length_ticks: Some(length_ticks),
        }
    }

    /// Epoch length in ticks, or `None` for a single epoch.
    #[must_use]
    pub const fn length_ticks(self) -> Option<NonZeroU64> {
        self.length_ticks
    }

    /// The epoch containing match tick `tick`, and the tick within that epoch.
    #[must_use]
    pub const fn locate(self, tick: u64) -> EpochTime {
        match self.length_ticks {
            None => EpochTime { epoch: 0, tick },
            Some(length) => EpochTime {
                epoch: tick / length.get(),
                tick: tick % length.get(),
            },
        }
    }
}

/// A point in time as (epoch number, tick within the epoch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EpochTime {
    /// Epoch number, for [`PlayerKey::epoch_seed`].
    pub epoch: u64,
    /// Tick within the epoch, for [`ProbeGenerator::drift`].
    pub tick: u64,
}

/// Probe configuration. Client and server must use the same one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ProbeConfig {
    /// Peak drift.
    pub amplitude: Amplitude,
    /// Signal shape.
    pub shape: SignalShape,
    /// Epoch schedule.
    pub epochs: EpochSchedule,
}

/// The quantity a drift applies to. Each has an independent signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stream {
    /// Mouse sensitivity.
    Sensitivity,
    /// Recoil scale.
    Recoil,
}

impl Stream {
    fn tag(self) -> u8 {
        match self {
            Self::Sensitivity => 0,
            Self::Recoil => 1,
        }
    }
}

/// A drift value: the nominal quantity is multiplied by `1 + ppb / 10^9`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Drift {
    ppb: i32,
}

impl Drift {
    /// Drift in parts per billion. Never exceeds the configured amplitude.
    #[must_use]
    pub const fn ppb(self) -> i32 {
        self.ppb
    }

    /// The factor to apply, `1 + ppb / 10^9`. Two correctly rounded IEEE 754
    /// operations on exact inputs, so identical on every platform.
    #[must_use]
    pub fn multiplier(self) -> f64 {
        1.0 + f64::from(self.ppb) / 1e9
    }
}

/// Invalid probe configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// Amplitude above [`Amplitude::MAX`].
    AmplitudeTooLarge {
        /// The requested amplitude.
        ppm: u32,
    },
    /// Sinusoid count outside 1 to [`MAX_SINUSOIDS`].
    InvalidComponentCount {
        /// The requested count.
        components: u8,
    },
    /// Sinusoid periods not satisfying `2 <= min <= max`.
    InvalidPeriods {
        /// The requested shortest period.
        min_period_ticks: u32,
        /// The requested longest period.
        max_period_ticks: u32,
    },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::AmplitudeTooLarge { ppm } => write!(
                f,
                "amplitude {ppm} ppm exceeds the maximum of {} ppm",
                Amplitude::MAX.0
            ),
            Self::InvalidComponentCount { components } => write!(
                f,
                "sinusoid count {components} is outside 1..={MAX_SINUSOIDS}"
            ),
            Self::InvalidPeriods {
                min_period_ticks,
                max_period_ticks,
            } => write!(
                f,
                "sinusoid periods {min_period_ticks}..={max_period_ticks} ticks are invalid \
                 (need 2 <= min <= max)"
            ),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Computes one player's drift for one epoch.
///
/// Holds derived secrets, which are redacted in `Debug` and zeroized on drop. Takes
/// `&mut self` only to cache the current noise segment: the output at a tick never
/// depends on which ticks were asked for before.
#[derive(Debug)]
pub struct ProbeGenerator {
    amplitude_ppb: i64,
    sensitivity: StreamSignal,
    recoil: StreamSignal,
}

impl ProbeGenerator {
    /// # Errors
    /// [`ProbeError`] if the signal shape is invalid. (An [`Amplitude`] is valid by
    /// construction.)
    pub fn new(seed: &EpochSeed, config: &ProbeConfig) -> Result<Self, ProbeError> {
        config.shape.validate()?;
        let signal = |stream: Stream| {
            let key = seed.stream_key(stream.tag(), config.shape.tag());
            StreamSignal::new(&key, &config.shape)
        };
        Ok(Self {
            amplitude_ppb: i64::from(config.amplitude.ppm()) * 1_000,
            sensitivity: signal(Stream::Sensitivity),
            recoil: signal(Stream::Recoil),
        })
    }

    /// The drift for `stream` at epoch-local tick `tick`.
    pub fn drift(&mut self, stream: Stream, tick: u64) -> Drift {
        let unit = match stream {
            Stream::Sensitivity => self.sensitivity.unit(tick),
            Stream::Recoil => self.recoil.unit(tick),
        };
        let ppb =
            (unit * self.amplitude_ppb / fixed::ONE).clamp(-self.amplitude_ppb, self.amplitude_ppb);
        Drift {
            // |ppb| <= 20_000_000, so it fits.
            ppb: ppb as i32,
        }
    }
}
