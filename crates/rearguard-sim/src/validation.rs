// SPDX-License-Identifier: MIT OR Apache-2.0

//! Ground-truth checks of the models: does a class's aim error follow the drift?
//!
//! This is **not a detector.** It uses [`ShotTruth`], the simulator's exact record of
//! what the drift did to the view, to check that the models behave as intended
//! (closed-loop error ignores the drift, open-loop error follows it).
//!
//! For each shot, two quantities per axis (yaw and pitch):
//!
//! - `error`: the aim error `target - view` at the shot;
//! - `drift`: how much the drift changed that error, `nominal view - view`, where the
//!   nominal view replays the same inputs with no drift.
//!
//! Both are taken relative to a reference: for flick, the moment the target appeared
//! (so `drift` is what the drift did to this target's engagement); for spray, the first
//! shot of the burst (so a constant acquisition error cancels). An open-loop player
//! never sees the drift, so its error contains all of `drift`; a closed-loop player
//! sees and corrects it.
//!
//! The statistic is Pearson's r over all (error, drift) pairs. Its null distribution
//! comes from a permutation test that re-pairs each unit's errors with another unit's
//! drift (a unit is one flick target, or one full spray burst). That keeps each unit's
//! internal structure (spray errors are cumulative, so pairs within a burst are not
//! independent) while breaking any real link, so the null accounts for dependence
//! that a plain 1/sqrt(N) would ignore.

use crate::seed::SimRng;
use crate::world::{RECOIL_PATTERN_V1, Scenario, ShotTruth};

/// (error, drift) pairs of one unit: one flick target or one full spray burst.
pub type Unit = Vec<(f64, f64)>;

/// Splits a session's shots into units. Spray bursts shorter than a full magazine (the
/// session ended mid-burst) are dropped, so every spray unit has the same length.
#[must_use]
pub fn units(scenario: Scenario, truth: &[ShotTruth]) -> Vec<Unit> {
    match scenario {
        Scenario::Flick => truth
            .iter()
            .map(|t| {
                (0..2)
                    .map(|a| (t.error[a], t.drift_offset[a] - t.drift_offset_at_target[a]))
                    .collect()
            })
            .collect(),
        Scenario::Spray => {
            let mut out = Vec::new();
            for burst in truth.chunk_by(|a, b| a.target_index == b.target_index) {
                if burst.len() != RECOIL_PATTERN_V1.len() || burst[0].burst_shot != 0 {
                    continue;
                }
                let first = burst[0];
                let unit = burst[1..]
                    .iter()
                    .flat_map(|t| {
                        (0..2).map(move |a| {
                            (
                                t.error[a] - first.error[a],
                                t.drift_offset[a] - first.drift_offset[a],
                            )
                        })
                    })
                    .collect();
                out.push(unit);
            }
            out
        }
    }
}

/// Pearson's r of `pairs`; 0 if either side is constant.
#[must_use]
pub fn pearson<'a>(pairs: impl Iterator<Item = &'a (f64, f64)> + Clone) -> f64 {
    let (mut n, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for (x, y) in pairs.clone() {
        n += 1.0;
        sx += x;
        sy += y;
    }
    let (mx, my) = (sx / n, sy / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in pairs {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
        syy += (y - my) * (y - my);
    }
    if sxx == 0.0 || syy == 0.0 {
        return 0.0;
    }
    sxy / libm::sqrt(sxx * syy)
}

/// Result of [`drift_correlation`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriftCorrelation {
    /// Number of (error, drift) pairs.
    pub pairs: usize,
    /// Pearson's r between error and drift.
    pub r: f64,
    /// Standard deviation of r under the permutation null.
    pub null_sd: f64,
    /// `r / null_sd`.
    pub z: f64,
    /// Regression slope of error on drift: 1 means the error contains all of the
    /// drift, 0 none of it.
    pub slope: f64,
}

/// Correlation between error and drift over `units`, with a permutation null of
/// `rounds` re-pairings drawn from `rng`. Units must all have the same length.
#[must_use]
pub fn drift_correlation(units: &[Unit], rng: &mut SimRng, rounds: usize) -> DriftCorrelation {
    let all = || units.iter().flatten();
    let r = pearson(all());
    let (mut sxy, mut syy) = (0.0, 0.0);
    let n = all().count() as f64;
    let (mx, my) = (
        all().map(|p| p.0).sum::<f64>() / n,
        all().map(|p| p.1).sum::<f64>() / n,
    );
    for (x, y) in all() {
        sxy += (x - mx) * (y - my);
        syy += (y - my) * (y - my);
    }
    let slope = if syy > 0.0 { sxy / syy } else { 0.0 };

    let mut order: Vec<usize> = (0..units.len()).collect();
    let mut sum_sq = 0.0;
    let mut shuffled = Vec::with_capacity(all().count());
    for _ in 0..rounds {
        for i in (1..order.len()).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        shuffled.clear();
        for (unit, &other) in units.iter().zip(&order) {
            for (a, b) in unit.iter().zip(&units[other]) {
                shuffled.push((a.0, b.1));
            }
        }
        let null_r = pearson(shuffled.iter());
        sum_sq += null_r * null_r;
    }
    let null_sd = libm::sqrt(sum_sq / rounds as f64);
    DriftCorrelation {
        pairs: all().count(),
        r,
        null_sd,
        z: if null_sd > 0.0 { r / null_sd } else { 0.0 },
        slope,
    }
}
