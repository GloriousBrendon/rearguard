// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integer fixed-point maths for the probe signal.
//!
//! Values are Q30: `ONE` (2^30) stands for 1.0. Everything here is integer arithmetic
//! with Rust-defined semantics (arithmetic right shift, truncating division), so the
//! results are bit-identical on every platform and compiler. No floating point.

/// 1.0 in Q30.
pub(crate) const ONE: i64 = 1 << 30;

/// Taylor coefficients of `sin(pi/2 * x)` for x^1, x^3, ..., x^13, in Q30.
/// Maximum error on [0, 1] is below 5e-9 (see `quarter_sine_matches_f64`).
const SIN_COEFFS: [i64; 7] = [
    1_686_629_713,
    -693_598_668,
    85_569_306,
    -5_026_995,
    172_272,
    -3_864,
    61,
];

/// `sin(pi/2 * x)` for `x` in `[0, ONE]`, clamped to `[0, ONE]`.
fn quarter_sine(x: i64) -> i64 {
    debug_assert!((0..=ONE).contains(&x));
    let x2 = (x * x) >> 30;
    let mut p = SIN_COEFFS[SIN_COEFFS.len() - 1];
    for &c in SIN_COEFFS[..SIN_COEFFS.len() - 1].iter().rev() {
        p = c + ((p * x2) >> 30);
    }
    ((p * x) >> 30).clamp(0, ONE)
}

/// Sine of a phase where a full turn is 2^64, in Q30. The result is in `[-ONE, ONE]`.
/// Uses the top 32 bits of the phase.
pub(crate) fn sin_turns(phase: u64) -> i64 {
    let frac = ((phase >> 32) & (ONE as u64 - 1)) as i64;
    match phase >> 62 {
        0 => quarter_sine(frac),
        1 => quarter_sine(ONE - frac),
        2 => -quarter_sine(frac),
        _ => -quarter_sine(ONE - frac),
    }
}

/// Uniform cubic B-spline through four consecutive knots, at position `u` in
/// `[0, ONE)` between the middle two. Knots and result are Q30 in `[-ONE, ONE]`.
///
/// The basis weights are non-negative and sum to one, so the result never leaves the
/// range of the knots; the final clamp only absorbs rounding.
pub(crate) fn cubic_bspline(knots: &[i64; 4], u: i64) -> i64 {
    debug_assert!((0..ONE).contains(&u));
    let u2 = (u * u) >> 30;
    let u3 = (u2 * u) >> 30;
    let v = ONE - u;
    let v3 = (((v * v) >> 30) * v) >> 30;
    // Six times the standard basis weights.
    let weights = [
        v3,
        3 * u3 - 6 * u2 + 4 * ONE,
        -3 * u3 + 3 * u2 + 3 * u + ONE,
        u3,
    ];
    let sum: i128 = weights
        .iter()
        .zip(knots)
        .map(|(&w, &k)| i128::from(w) * i128::from(k))
        .sum();
    let value = sum / i128::from(6 * ONE);
    // |value| <= ONE plus rounding, so the cast cannot truncate.
    (value as i64).clamp(-ONE, ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_f64(q: i64) -> f64 {
        q as f64 / ONE as f64
    }

    #[test]
    fn quarter_sine_matches_f64() {
        let mut worst = 0.0f64;
        for i in 0..=100_000i64 {
            let x = ONE * i / 100_000;
            let expected = (core::f64::consts::FRAC_PI_2 * to_f64(x)).sin();
            worst = worst.max((to_f64(quarter_sine(x)) - expected).abs());
        }
        assert!(worst < 5e-9, "max error {worst}");
    }

    #[test]
    fn sine_hits_the_cardinal_points() {
        assert_eq!(sin_turns(0), 0);
        assert_eq!(sin_turns(1 << 62), ONE);
        assert_eq!(sin_turns(1 << 63), 0);
        assert_eq!(sin_turns(3 << 62), -ONE);
    }

    #[test]
    fn sine_matches_f64_over_a_full_turn() {
        for i in 0..4096u64 {
            let phase = i.wrapping_mul(u64::MAX / 4096);
            let turns = phase as f64 / 2f64.powi(64);
            let expected = (core::f64::consts::TAU * turns).sin();
            let error = (to_f64(sin_turns(phase)) - expected).abs();
            assert!(error < 1e-8, "phase {phase}: error {error}");
        }
    }

    #[test]
    fn bspline_of_constant_knots_is_that_constant() {
        for k in [-ONE, -12345, 0, 777, ONE] {
            for u in [0, 1, ONE / 3, ONE / 2, ONE - 1] {
                let value = cubic_bspline(&[k; 4], u);
                assert!((value - k).abs() <= 2, "k {k} u {u}: {value}");
            }
        }
    }

    #[test]
    fn bspline_is_continuous_across_knots() {
        // The end of one segment meets the start of the next.
        let knots = [ONE / 2, -ONE, ONE, -ONE / 4, ONE / 3];
        let end = cubic_bspline(&[knots[0], knots[1], knots[2], knots[3]], ONE - 1);
        let start = cubic_bspline(&[knots[1], knots[2], knots[3], knots[4]], 0);
        assert!((end - start).abs() < 16, "{end} vs {start}");
    }

    #[test]
    fn bspline_stays_within_extreme_knots() {
        for knots in [
            [ONE, ONE, ONE, ONE],
            [-ONE, -ONE, -ONE, -ONE],
            [ONE, -ONE, ONE, -ONE],
        ] {
            for i in 0..1000 {
                let value = cubic_bspline(&knots, ONE * i / 1000);
                assert!((-ONE..=ONE).contains(&value));
            }
        }
    }
}
