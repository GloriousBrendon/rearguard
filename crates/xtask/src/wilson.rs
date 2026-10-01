// SPDX-License-Identifier: MIT OR Apache-2.0

//! Wilson score interval for a binomial proportion.
//!
//! For `k` flagged sessions out of `n`, with `p = k / n` and `z` the standard normal
//! quantile for the confidence level (95%: `z = 1.959963984540054`):
//!
//! ```text
//! centre = (p + z²/2n) / (1 + z²/n)
//! half   = z · sqrt(p(1 − p)/n + z²/4n²) / (1 + z²/n)
//! interval = [centre − half, centre + half]
//! ```
//!
//! No continuity correction. Unlike the normal-approximation ("Wald") interval it stays
//! inside [0, 1] and is not empty at `k = 0` or `k = n`, which is where false-positive
//! rates live. Reference: E. B. Wilson (1927); values checked against R. G. Newcombe,
//! "Two-sided confidence intervals for the single proportion", Statistics in Medicine
//! 17 (1998), Table I, method 3.
//!
//! The interval treats the `n` sessions as independent trials with a fixed threshold.
//! It therefore holds for sessions that played no part in setting the threshold, and
//! does not include the uncertainty of the threshold itself.

/// Standard normal quantile at 0.975: a two-sided 95% interval.
pub const Z_95: f64 = 1.959_963_984_540_054;

/// The 95% Wilson score interval `(low, high)` for `k` successes in `n` trials, or
/// `None` when there are no trials.
#[must_use]
pub fn interval(k: u64, n: u64) -> Option<(f64, f64)> {
    if n == 0 || k > n {
        return None;
    }
    let (k, n) = (k as f64, n as f64);
    let z2 = Z_95 * Z_95;
    let p = k / n;
    let denominator = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denominator;
    let half = Z_95 * libm::sqrt(p * (1.0 - p) / n + z2 / (4.0 * n * n)) / denominator;
    // The ends are exact at k = 0 and k = n; rounding must not move them.
    let low = if k == 0.0 {
        0.0
    } else {
        (centre - half).max(0.0)
    };
    let high = if k == n {
        1.0
    } else {
        (centre + half).min(1.0)
    };
    Some((low, high))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Newcombe (1998), Table I, method 3 (Wilson score, no continuity correction),
    /// printed to four decimals.
    #[test]
    fn matches_newcombe_1998_table_1() {
        for (k, n, low, high) in [
            (81, 263, 0.2553, 0.3662),
            (15, 148, 0.0624, 0.1605),
            (0, 20, 0.0000, 0.1611),
            (1, 29, 0.0061, 0.1718),
        ] {
            let (l, h) = interval(k, n).unwrap();
            assert!((l - low).abs() < 5e-5, "{k}/{n}: low {l} vs {low}");
            assert!((h - high).abs() < 5e-5, "{k}/{n}: high {h} vs {high}");
        }
    }

    /// Further values, from the closed form evaluated independently (Python, double
    /// precision), at the sample sizes this evaluation uses.
    #[test]
    fn matches_independent_values() {
        for (k, n, low, high) in [
            (5, 10, 0.236_593, 0.763_407),
            (29, 29, 0.883_030, 1.0),
            (1, 5_000, 0.000_035, 0.001_132),
            (0, 5_000, 0.0, 0.000_768),
        ] {
            let (l, h) = interval(k, n).unwrap();
            assert!((l - low).abs() < 5e-7, "{k}/{n}: low {l} vs {low}");
            assert!((h - high).abs() < 5e-7, "{k}/{n}: high {h} vs {high}");
        }
    }

    #[test]
    fn ends_are_exact_and_the_interval_is_symmetric() {
        assert_eq!(interval(0, 7).unwrap().0, 0.0);
        assert_eq!(interval(7, 7).unwrap().1, 1.0);
        assert_eq!(interval(0, 0), None);
        assert_eq!(interval(3, 2), None);
        for (k, n) in [(0, 9), (2, 9), (4, 9), (17, 300)] {
            let (l, h) = interval(k, n).unwrap();
            let (l2, h2) = interval(n - k, n).unwrap();
            assert!((l - (1.0 - h2)).abs() < 1e-12 && (h - (1.0 - l2)).abs() < 1e-12);
            let p = k as f64 / n as f64;
            assert!(l <= p && p <= h);
        }
    }
}
