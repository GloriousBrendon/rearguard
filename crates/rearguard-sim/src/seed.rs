// SPDX-License-Identifier: MIT OR Apache-2.0

//! The top-level simulation seed and the deterministic random streams derived from it.
//!
//! Every random draw in a simulation comes from a ChaCha20 stream keyed by the
//! [`SimSeed`] and numbered by purpose, so a seed reproduces every session exactly,
//! and one session's draws never shift another's.

use core::fmt;

use chacha20::ChaCha20Rng;
use chacha20::rand_core::{Rng, SeedableRng};
use rearguard_core::probe::RootSeed;
use zeroize::Zeroize;

/// Domain separation for the sim's ChaCha20 key.
const KEY_PREFIX: &[u8; 16] = b"rearguard-sim/v1";
/// Stream reserved for deriving the probe root seed.
const PROBE_ROOT_STREAM: u64 = u64::MAX;

/// The top-level seed of a simulation run. It determines the probe root seed, so it
/// is treated as a secret: redacted in `Debug`, no `Display`, wiped on drop.
///
/// It is test data for Rearguard's closed test environment, with 64 bits of entropy:
/// never use it to seed anything outside the simulator.
pub struct SimSeed(u64);

impl SimSeed {
    /// Wraps `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The deterministic random stream numbered `stream`.
    #[must_use]
    pub fn rng(&self, stream: u64) -> SimRng {
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(KEY_PREFIX);
        key[16..24].copy_from_slice(&self.0.to_le_bytes());
        let mut rng = ChaCha20Rng::from_seed(key);
        key.zeroize();
        rng.set_stream(stream);
        SimRng(rng, None)
    }

    /// The probe root seed for this run: the simulated server's secret.
    #[must_use]
    pub fn probe_root(&self) -> RootSeed {
        let mut bytes = [0u8; 32];
        self.rng(PROBE_ROOT_STREAM).0.fill_bytes(&mut bytes);
        RootSeed::from_bytes(&mut bytes)
    }
}

impl Drop for SimSeed {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SimSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimSeed(<redacted>)")
    }
}

/// A deterministic random stream. Floating-point draws use only IEEE 754 basic
/// operations and `libm`, so they are identical on every platform.
pub struct SimRng(ChaCha20Rng, Option<f64>);

impl SimRng {
    /// Uniform `u64`.
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    /// Uniform in `[0, 1)`, 53 bits.
    pub fn uniform(&mut self) -> f64 {
        (self.0.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    /// Standard normal (Box–Muller; each pair of uniforms gives two normals).
    pub fn normal(&mut self) -> f64 {
        if let Some(z) = self.1.take() {
            return z;
        }
        let u1 = 1.0 - self.uniform(); // (0, 1]
        let u2 = self.uniform();
        let radius = libm::sqrt(-2.0 * libm::log(u1));
        let (sin, cos) = libm::sincos(core::f64::consts::TAU * u2);
        self.1 = Some(radius * sin);
        radius * cos
    }

    /// Normal with the given mean and standard deviation.
    pub fn gauss(&mut self, mean: f64, sd: f64) -> f64 {
        mean + sd * self.normal()
    }

    /// Log-normal with the given mean and standard deviation of the result.
    pub fn lognormal(&mut self, mean: f64, sd: f64) -> f64 {
        let s2 = libm::log(1.0 + (sd * sd) / (mean * mean));
        let mu = libm::log(mean) - s2 / 2.0;
        libm::exp(mu + libm::sqrt(s2) * self.normal())
    }
}

impl fmt::Debug for SimRng {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimRng(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_reproducible_and_distinct() {
        let seed = SimSeed::new(42);
        let a: Vec<u64> = (0..4).map(|_| seed.rng(1).next_u64()).collect();
        assert!(
            a.windows(2).all(|w| w[0] == w[1]),
            "same stream, same draws"
        );
        let mut s1 = seed.rng(1);
        let mut s2 = seed.rng(2);
        let mut other = SimSeed::new(43).rng(1);
        let first = s1.next_u64();
        assert_ne!(first, s2.next_u64());
        assert_ne!(first, other.next_u64());
    }

    #[test]
    fn distributions_have_the_right_moments() {
        let mut rng = SimSeed::new(7).rng(0);
        let n = 200_000;
        let (mut s, mut s2, mut ln) = (0.0, 0.0, 0.0);
        for _ in 0..n {
            let x = rng.normal();
            s += x;
            s2 += x * x;
            ln += rng.lognormal(0.2, 0.04);
        }
        let n = f64::from(n);
        assert!((s / n).abs() < 0.01);
        assert!((s2 / n - 1.0).abs() < 0.01);
        assert!((ln / n - 0.2).abs() < 0.001);
        for _ in 0..1000 {
            let u = rng.uniform();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn seed_is_redacted() {
        let seed = SimSeed::new(0xDEAD_BEEF);
        let shown = format!("{seed:?} {:?}", seed.rng(0));
        assert!(!shown.contains("3735928559") && !shown.to_lowercase().contains("deadbeef"));
        assert!(shown.contains("<redacted>"));
    }

    #[test]
    fn probe_root_depends_on_the_seed() {
        use rearguard_core::probe::{ProbeConfig, ProbeGenerator, Stream};
        let drift = |seed: u64| {
            let root = SimSeed::new(seed).probe_root();
            let epoch = root.match_key(b"m").player_key(1).epoch_seed(0);
            let mut probe = ProbeGenerator::new(&epoch, &ProbeConfig::default()).unwrap();
            probe.drift(Stream::Sensitivity, 1234)
        };
        assert_eq!(drift(1), drift(1));
        assert_ne!(drift(1), drift(2));
    }
}
