// SPDX-License-Identifier: MIT OR Apache-2.0

//! The evaluation's configuration: a JSON file with every field required and unknown
//! fields refused. It is written, unchanged in meaning, into every result file.

use rearguard_core::detect::{DetectorConfig, KappaTest};
use rearguard_core::probe::Amplitude;
use serde::{Deserialize, Serialize};

/// Detector statistics, in `Report::statistics` order.
pub const STATISTICS: [&str; 4] = ["error", "steps", "change", "spray"];

/// The scenarios the evaluation covers.
pub const SCENARIOS: [&str; 2] = ["flick", "spray"];

/// What to evaluate, and how.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalConfig {
    /// Detector parameters that are not thresholds. The flag scores are what the
    /// evaluation calibrates.
    pub detector: DetectorParams,
    /// False-positive rate the thresholds aim for, per session of `horizon_s`, shared
    /// between a scenario's statistics.
    pub fpr_target: f64,
    /// Session length evaluated, seconds. Simulated sessions are run for exactly this
    /// long; recorded sessions of any other planned length are left out.
    pub horizon_s: f64,
    /// Drift amplitudes simulated, ppm.
    pub amplitudes_ppm: Vec<u32>,
    /// Simulated sessions.
    pub sim: SimPlan,
    /// How human participants are divided between calibration and evaluation.
    pub human_split: HumanSplit,
}

/// The detector's null hypotheses and sample limits (see `rearguard_core::detect`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectorParams {
    /// Fire-time statistic: kappa bound, 1/deg.
    pub kappa_bound: f64,
    /// Pairs needed before a statistic reports a score.
    pub min_pairs: u64,
    /// Window length for per-window evidence, milliseconds.
    pub window_ms: u64,
    /// Shortest per-frame step the step-response statistic uses, counts.
    pub min_step_counts: f64,
    /// Drift-change statistic (flick): kappa bound, or `null` to leave it out.
    pub change_kappa_bound: Option<f64>,
    /// Per-shot spray statistic: kappa bound, or `null` to leave it out.
    pub spray_kappa_bound: Option<f64>,
}

/// How many sessions to simulate per scenario (humans) or class (cheats), at every
/// amplitude.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimPlan {
    /// Simulated humans the `sim` thresholds are calibrated on.
    pub calibration_humans: u32,
    /// Simulated humans, never used for calibration, the false-positive rate is
    /// measured on.
    pub held_out_humans: u32,
    /// Sessions per cheat class.
    pub cheats: u32,
}

/// The split of human participants. A participant is in the calibration set when a
/// hash of the salt and their id falls below `calibration_fraction`, so nobody moves
/// between the sets when participants are added. Keep the salt for the life of a study.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanSplit {
    /// Expected share of participants in the calibration set, strictly between 0 and 1.
    pub calibration_fraction: f64,
    /// Names the split.
    pub salt: String,
}

impl EvalConfig {
    /// Parses and checks a configuration.
    ///
    /// # Errors
    /// Unknown or missing fields, or a value out of range.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let config: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        let fail = |what: &str| Err(what.to_owned());
        if !(self.fpr_target > 0.0 && self.fpr_target < 1.0) {
            return fail("fpr_target must be between 0 and 1");
        }
        if !(self.horizon_s.is_finite() && self.horizon_s > 0.0 && self.horizon_s <= 3_600.0) {
            return fail("horizon_s must be between 0 and 3600");
        }
        if self.amplitudes_ppm.is_empty()
            || !self.amplitudes_ppm.windows(2).all(|w| w[0] < w[1])
            || self
                .amplitudes_ppm
                .iter()
                .any(|a| *a == 0 || Amplitude::from_ppm(*a).is_err())
        {
            return fail("amplitudes_ppm must be ascending, distinct, above 0 and within range");
        }
        if self.sim.calibration_humans == 0 || self.sim.held_out_humans == 0 || self.sim.cheats == 0
        {
            return fail("sim counts must be at least 1");
        }
        let f = self.human_split.calibration_fraction;
        if !(f > 0.0 && f < 1.0) {
            return fail("human_split.calibration_fraction must be strictly between 0 and 1");
        }
        // The detector checks its own ranges; ask it.
        let probe = rearguard_core::probe::ProbeConfig::default();
        let seed = rearguard_core::probe::EpochSeed::from_bytes(&mut [0; 32]);
        rearguard_core::detect::Detector::new(&seed, &probe, self.detector.config(None))
            .map(|_| ())
            .map_err(|e| format!("detector: {e}"))
    }

    /// The configuration as one line of JSON, for result files.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("plain data serialises")
    }
}

impl DetectorParams {
    /// Indices into [`STATISTICS`] of the statistics a scenario's detector runs: the
    /// fire-time and step-response statistics, plus the drift-change statistic (flick)
    /// or the per-shot spray statistic (spray) when configured.
    #[must_use]
    pub fn applicable(&self, scenario: &str) -> Vec<usize> {
        let mut stats = vec![0, 1];
        match scenario {
            "flick" if self.change_kappa_bound.is_some() => stats.push(2),
            "spray" if self.spray_kappa_bound.is_some() => stats.push(3),
            _ => {}
        }
        stats
    }

    /// A detector configuration with these parameters and the given flag scores
    /// ([`STATISTICS`] order). Without flag scores, none that a session can reach: the
    /// evaluation reads scores, and compares them with thresholds itself.
    #[must_use]
    pub fn config(&self, flag_scores: Option<[f64; 4]>) -> DetectorConfig {
        let flags = flag_scores.unwrap_or([f64::MAX; 4]);
        let test = |bound: Option<f64>, flag_score: f64| {
            bound.map(|kappa_bound| KappaTest {
                kappa_bound,
                flag_score,
            })
        };
        DetectorConfig {
            kappa_bound: self.kappa_bound,
            error_flag_score: flags[0],
            steps_flag_score: flags[1],
            min_pairs: self.min_pairs,
            window_ms: self.window_ms,
            min_step_counts: self.min_step_counts,
            change: test(self.change_kappa_bound, flags[2]),
            spray: test(self.spray_kappa_bound, flags[3]),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn default_json() -> &'static str {
        include_str!("../eval/eval.json")
    }

    /// A configuration small enough for tests.
    pub(crate) fn small() -> EvalConfig {
        let mut config = EvalConfig::from_json(default_json()).unwrap();
        config.horizon_s = 10.0;
        config.amplitudes_ppm = vec![10_000, 20_000];
        config.fpr_target = 0.05;
        config.sim = SimPlan {
            calibration_humans: 12,
            held_out_humans: 10,
            cheats: 3,
        };
        config
    }

    #[test]
    fn the_default_configuration_is_valid_and_round_trips() {
        let config = EvalConfig::from_json(default_json()).unwrap();
        assert_eq!(config.detector.applicable("flick"), vec![0, 1, 2]);
        assert_eq!(config.detector.applicable("spray"), vec![0, 1, 3]);
        assert_eq!(EvalConfig::from_json(&config.to_json()).unwrap(), config);
        assert!(!config.to_json().contains('\n'));
    }

    #[test]
    fn bad_configurations_are_refused() {
        for (from, to) in [
            ("\"fpr_target\": 0.001", "\"fpr_target\": 0"),
            ("\"horizon_s\": 60.0", "\"horizon_s\": -1"),
            ("[2500, 5000, 10000, 20000]", "[5000, 2500]"),
            ("[2500, 5000, 10000, 20000]", "[0, 2500]"),
            ("[2500, 5000, 10000, 20000]", "[30000]"),
            ("\"cheats\": 500", "\"cheats\": 0"),
            (
                "\"calibration_fraction\": 0.5",
                "\"calibration_fraction\": 1.0",
            ),
            ("\"min_pairs\": 20", "\"min_pairs\": 2"),
            ("\"min_pairs\": 20", "\"min_pairs\": 20, \"extra\": 1"),
            ("\"salt\": \"rearguard-eval-1.12\"", "\"salt\": 7"),
        ] {
            let text = default_json().replace(from, to);
            assert_ne!(text, default_json(), "{from}");
            assert!(EvalConfig::from_json(&text).is_err(), "{to}");
        }
    }

    #[test]
    fn statistics_left_out_are_not_applicable() {
        let mut params = small().detector;
        params.change_kappa_bound = None;
        params.spray_kappa_bound = None;
        assert_eq!(params.applicable("flick"), vec![0, 1]);
        assert_eq!(params.applicable("spray"), vec![0, 1]);
        let config = params.config(Some([1.0, 2.0, 3.0, 4.0]));
        assert!(config.change.is_none() && config.spray.is_none());
        assert_eq!(
            (config.error_flag_score, config.steps_flag_score),
            (1.0, 2.0)
        );
    }
}
