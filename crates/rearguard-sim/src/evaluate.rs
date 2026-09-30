// SPDX-License-Identifier: MIT OR Apache-2.0

//! Offline evaluation of the input-probe detector (`rearguard_core::detect`) on
//! simulated players.
//!
//! Every session is simulated for the longest observation time, streamed through a
//! detector exactly as a server would (telemetry plus the player's epoch seed), and
//! scored at each shorter observation time along the way. Thresholds are calibrated on
//! simulated humans only, at a chosen false-positive rate; detection rates, their
//! bootstrap confidence intervals, and times to detection follow.
//!
//! All numbers are about the simulator's models, not about real players (see
//! `README.md`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use rearguard_core::detect::{Detector, DetectorConfig, Evidence, Report};
use rearguard_core::probe::{Amplitude, ProbeConfig, RootSeed};
use rearguard_core::telemetry::Record;

use crate::seed::{SimRng, SimSeed};
use crate::session::{Class, ModelParams, SessionSpec, run_session};
use crate::world::Scenario;

/// One simulated player's detector output.
#[derive(Clone, Debug)]
pub struct Trace {
    /// Session evidence at each observation time (the state before the first record
    /// at or after that time).
    pub snapshots: Vec<Report>,
    /// Highest fire-time score reached up to each observation time.
    pub max_error: Vec<f64>,
    /// Highest step-response score reached up to each observation time.
    pub max_steps: Vec<f64>,
    /// Every point where a running maximum score rose while still below
    /// [`PATH_SCORE_CAP`]: (shots, engagements, fire-time score, step-response score).
    /// Enough to find the first crossing of any threshold below the cap. Kept only when
    /// requested (cheats), for time to detection.
    pub path: Vec<(u32, u32, f32, f32)>,
}

/// Scores above this never need a recorded path point: every calibrated threshold is
/// far below it.
pub const PATH_SCORE_CAP: f64 = 5_000.0;

/// Simulates one session and streams it through a detector.
#[must_use]
pub fn trace_session(
    seed: &SimSeed,
    root: &RootSeed,
    spec: SessionSpec,
    config: &DetectorConfig,
    observe_ms: &[u64],
    keep_path: bool,
) -> Trace {
    let session = run_session(seed, root, spec);
    // The server's view: the player's epoch seed from its own key hierarchy.
    let epoch = root
        .match_key(session.match_id.as_bytes())
        .player_key(session.player_id)
        .epoch_seed(0);
    let probe = ProbeConfig {
        amplitude: spec.amplitude,
        ..ProbeConfig::default()
    };
    let mut detector = Detector::new(&epoch, &probe, config.clone()).expect("valid config");
    let Some(Record::Header(header)) = session.records.first() else {
        unreachable!("sessions start with a header")
    };
    let start_us = header.probe_start_us;
    let mut trace = Trace {
        snapshots: Vec::with_capacity(observe_ms.len()),
        max_error: Vec::with_capacity(observe_ms.len()),
        max_steps: Vec::with_capacity(observe_ms.len()),
        path: Vec::new(),
    };
    let (mut max_error, mut max_steps) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut next = 0;
    for record in &session.records {
        while next < observe_ms.len() && record.ts_us() >= start_us + observe_ms[next] * 1_000 {
            trace.snapshots.push(detector.session());
            trace.max_error.push(max_error);
            trace.max_steps.push(max_steps);
            next += 1;
        }
        detector
            .feed(std::slice::from_ref(record))
            .expect("simulated telemetry is valid");
        if !matches!(record, Record::Move(_)) {
            let r = detector.session();
            let rose = (r.error.score > max_error && max_error < PATH_SCORE_CAP)
                || (r.steps.score > max_steps && max_steps < PATH_SCORE_CAP);
            max_error = max_error.max(r.error.score);
            max_steps = max_steps.max(r.steps.score);
            if keep_path && rose {
                trace.path.push((
                    r.shots as u32,
                    r.engagements as u32,
                    r.error.score as f32,
                    r.steps.score as f32,
                ));
            }
        }
    }
    while next < observe_ms.len() {
        trace.snapshots.push(detector.session());
        trace.max_error.push(max_error);
        trace.max_steps.push(max_steps);
        next += 1;
    }
    trace
}

/// Runs `jobs` in parallel on `threads` threads; results come back in job order, so
/// the output does not depend on scheduling.
pub fn parallel_map<T: Sync, R: Send>(
    jobs: &[T],
    threads: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..jobs.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= jobs.len() {
                        break;
                    }
                    let r = f(&jobs[i]);
                    results.lock().expect("no panics while holding the lock")[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .expect("no poisoned lock")
        .into_iter()
        .map(|r| r.expect("every job ran"))
        .collect()
}

/// What to evaluate.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Drift amplitudes, ppm.
    pub amplitudes_ppm: Vec<u32>,
    /// Observation times, milliseconds, ascending.
    pub observe_ms: Vec<u64>,
    /// Human sessions per scenario and amplitude.
    pub humans: u32,
    /// Sessions per cheat class and amplitude.
    pub cheats: u32,
    /// Cheat-model tuning.
    pub models: ModelParams,
    /// Worker threads.
    pub threads: usize,
}

/// Traces for every (amplitude, class, scenario).
#[derive(Clone, Debug)]
pub struct Results {
    /// What was run.
    pub plan: Plan,
    /// `(amplitude ppm, class, scenario, traces)`.
    pub groups: Vec<(u32, Class, Scenario, Vec<Trace>)>,
}

impl Results {
    /// Traces of one group.
    #[must_use]
    pub fn traces(&self, ppm: u32, class: Class, scenario: Scenario) -> &[Trace] {
        self.groups
            .iter()
            .find(|(a, c, s, _)| *a == ppm && *c == class && *s == scenario)
            .map_or(&[], |g| &g.3)
    }
}

/// Simulates and traces every session of `plan`.
#[must_use]
pub fn run(seed: &SimSeed, config: &DetectorConfig, plan: &Plan) -> Results {
    let root = seed.probe_root();
    let duration_s = *plan
        .observe_ms
        .last()
        .expect("at least one observation time") as f64
        / 1e3;
    let mut jobs = Vec::new();
    for &ppm in &plan.amplitudes_ppm {
        for class in Class::ALL {
            let n = if class == Class::Human {
                plan.humans
            } else {
                plan.cheats
            };
            for &scenario in class.scenarios() {
                for index in 0..n {
                    jobs.push((ppm, class, scenario, index));
                }
            }
        }
    }
    let traces = parallel_map(&jobs, plan.threads, |&(ppm, class, scenario, index)| {
        let spec = SessionSpec {
            class,
            scenario,
            index,
            duration_s,
            amplitude: Amplitude::from_ppm(ppm).expect("amplitude within range"),
            models: plan.models,
        };
        trace_session(
            seed,
            &root,
            spec,
            config,
            &plan.observe_ms,
            class != Class::Human,
        )
    });
    let mut groups: Vec<(u32, Class, Scenario, Vec<Trace>)> = Vec::new();
    for ((ppm, class, scenario, _), trace) in jobs.into_iter().zip(traces) {
        match groups.last_mut() {
            Some(g) if g.0 == ppm && g.1 == class && g.2 == scenario => g.3.push(trace),
            _ => groups.push((ppm, class, scenario, vec![trace])),
        }
    }
    Results {
        plan: plan.clone(),
        groups,
    }
}

/// The threshold that flags at most a fraction `fpr` of `null` scores (flag when
/// `score > threshold`): the `ceil((1 - fpr)·n)`-th smallest value.
#[must_use]
pub fn threshold_at(null: &[f64], fpr: f64) -> f64 {
    let mut sorted = null.to_vec();
    sorted.sort_by(f64::total_cmp);
    let k = ((1.0 - fpr) * sorted.len() as f64).ceil() as usize;
    sorted[k.clamp(1, sorted.len()) - 1]
}

/// Fraction of `scores` above `threshold`.
#[must_use]
pub fn rate_above(scores: &[f64], threshold: f64) -> f64 {
    scores.iter().filter(|s| **s > threshold).count() as f64 / scores.len().max(1) as f64
}

/// Detection rate at a false-positive rate, with a percentile bootstrap interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    /// Point estimate.
    pub rate: f64,
    /// 2.5th percentile of the bootstrap distribution.
    pub low: f64,
    /// 97.5th percentile.
    pub high: f64,
    /// The threshold used for the point estimate.
    pub threshold: f64,
}

fn resample(values: &[f64], rng: &mut SimRng) -> Vec<f64> {
    (0..values.len())
        .map(|_| values[(rng.next_u64() % values.len() as u64) as usize])
        .collect()
}

/// Detection rate of `cheat` at false-positive rate `fpr` on `human`. The bootstrap
/// resamples both groups, re-calibrating the threshold each time, so the interval
/// includes the uncertainty of the threshold itself.
#[must_use]
pub fn detection_rate(
    human: &[f64],
    cheat: &[f64],
    fpr: f64,
    rounds: usize,
    rng: &mut SimRng,
) -> Rate {
    let threshold = threshold_at(human, fpr);
    let rate = rate_above(cheat, threshold);
    let mut boots: Vec<f64> = (0..rounds)
        .map(|_| {
            let t = threshold_at(&resample(human, rng), fpr);
            rate_above(&resample(cheat, rng), t)
        })
        .collect();
    boots.sort_by(f64::total_cmp);
    let pick = |q: f64| boots[((boots.len() - 1) as f64 * q).round() as usize];
    Rate {
        rate,
        low: pick(0.025),
        high: pick(0.975),
        threshold,
    }
}

/// ROC points (false-positive rate, true-positive rate) over every distinct
/// threshold in `human`, plus the two ends.
#[must_use]
pub fn roc(human: &[f64], cheat: &[f64]) -> Vec<(f64, f64, f64)> {
    let mut thresholds = human.to_vec();
    thresholds.sort_by(f64::total_cmp);
    thresholds.dedup();
    let mut points = vec![(f64::INFINITY, 0.0, 0.0)];
    for &t in thresholds.iter().rev() {
        points.push((t, rate_above(human, t), rate_above(cheat, t)));
    }
    points.push((f64::NEG_INFINITY, 1.0, 1.0));
    points
}

/// Scores of a group at observation index `k`, by statistic.
#[must_use]
pub fn scores(traces: &[Trace], k: usize, pick: impl Fn(&Report) -> f64) -> Vec<f64> {
    traces.iter().map(|t| pick(&t.snapshots[k])).collect()
}

/// Alternative scores from the same evidence, to show why the detector weighs slope
/// against significance: plain significance of any correlation (null: r = 0), and a
/// slope-only test (null: slope <= `slope_bound`).
#[must_use]
pub fn alternative_z(e: &Evidence, slope_bound: f64) -> (f64, f64) {
    if e.pairs < 4 || e.slope_se <= 0.0 {
        return (0.0, 0.0);
    }
    let plain = libm::atanh(e.r) * libm::sqrt(e.pairs as f64 - 3.0);
    let slope = (e.slope - slope_bound) / e.slope_se;
    (plain, slope)
}

/// First point on a cheat's path where the fire-time score passes `error_threshold` or
/// the step-response score passes `steps_threshold`: (shots, engagements).
#[must_use]
pub fn first_detection(
    path: &[(u32, u32, f32, f32)],
    error_threshold: f64,
    steps_threshold: f64,
) -> Option<(u32, u32)> {
    path.iter()
        .find(|p| f64::from(p.2) > error_threshold || f64::from(p.3) > steps_threshold)
        .map(|p| (p.0, p.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_flags_at_most_the_requested_fraction() {
        let null: Vec<f64> = (0..1000).map(f64::from).collect();
        // 0.1% of 1000 is one score: only the largest (999) may pass.
        let t = threshold_at(&null, 0.001);
        assert_eq!(t, 998.0);
        assert_eq!(rate_above(&null, t), 0.001);
        let t = threshold_at(&null, 0.01);
        assert!(rate_above(&null, t) <= 0.01);
        assert_eq!(threshold_at(&[3.0], 0.001), 3.0);
    }

    #[test]
    fn detection_rate_and_bootstrap_bracket_a_clear_case() {
        let human: Vec<f64> = (0..2000).map(|i| f64::from(i % 100)).collect();
        let cheat: Vec<f64> = (0..200)
            .map(|i| if i < 150 { 500.0 } else { 0.0 })
            .collect();
        let mut rng = SimSeed::new(1).rng(0);
        let r = detection_rate(&human, &cheat, 0.001, 200, &mut rng);
        assert_eq!(r.rate, 0.75);
        assert!(
            r.low <= 0.75 && r.high >= 0.75 && r.high - r.low < 0.2,
            "{r:?}"
        );
    }

    #[test]
    fn roc_runs_from_nothing_to_everything() {
        let points = roc(&[1.0, 2.0, 3.0], &[2.5, 4.0]);
        assert_eq!(points.first().map(|p| (p.1, p.2)), Some((0.0, 0.0)));
        assert_eq!(points.last().map(|p| (p.1, p.2)), Some((1.0, 1.0)));
        assert!(
            points
                .windows(2)
                .all(|w| w[1].1 >= w[0].1 && w[1].2 >= w[0].2)
        );
    }

    #[test]
    fn parallel_map_keeps_job_order() {
        let jobs: Vec<u64> = (0..100).collect();
        assert_eq!(
            parallel_map(&jobs, 7, |x| x * x),
            jobs.iter().map(|x| x * x).collect::<Vec<_>>()
        );
    }

    #[test]
    fn small_run_is_deterministic_and_complete() {
        let config = DetectorConfig {
            kappa_bound: 2.0,
            error_flag_score: 1e9,
            steps_flag_score: 1e9,
            min_pairs: 20,
            window_ms: 30_000,
            min_step_counts: 100.0,
        };
        let plan = Plan {
            amplitudes_ppm: vec![5_000],
            observe_ms: vec![5_000, 10_000],
            humans: 2,
            cheats: 1,
            models: ModelParams::DEFAULT,
            threads: 4,
        };
        let a = run(&SimSeed::new(3), &config, &plan);
        let b = run(&SimSeed::new(3), &config, &plan);
        // Human flick + human spray + one group per cheat class.
        assert_eq!(a.groups.len(), Class::ALL.len() + 1);
        for (ga, gb) in a.groups.iter().zip(&b.groups) {
            assert_eq!(ga.3.len(), gb.3.len());
            for (ta, tb) in ga.3.iter().zip(&gb.3) {
                assert_eq!(ta.snapshots, tb.snapshots);
                assert_eq!(ta.path, tb.path);
                assert_eq!(ta.snapshots.len(), 2);
            }
        }
        let aimbot = &a.traces(5_000, Class::FlickAimbot, Scenario::Flick)[0];
        assert!(aimbot.snapshots[1].shots > aimbot.snapshots[0].shots);
        assert!(!aimbot.path.is_empty());
        assert!(
            a.traces(5_000, Class::Human, Scenario::Flick)[0]
                .path
                .is_empty()
        );
    }
}
