//! Human-study plans (task 1.9): which sessions a participant plays, in which order,
//! and the blind two-alternative drift comparisons.
//!
//! A plan is a pure function of a [`StudyProtocol`] and a 64-bit randomisation seed, so
//! an analysis can regenerate it exactly from the seed recorded in a study export. The
//! randomisation uses ChaCha20 and integer arithmetic only, so it is identical on every
//! platform.
//!
//! Drift seeds are not part of a plan. Each session's seed is derived from the study key
//! with the probe key hierarchy, using [`session_match_id`] as the match and player 0,
//! epoch 0. Whoever holds the study key can re-derive any session's seed; the export
//! only carries the labels.

use chacha20::ChaCha20Rng;
use chacha20::rand_core::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::probe::Amplitude;

/// Current [`StudyProtocol::protocol_version`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Most sessions (baseline plus blind intervals) one plan may hold.
const MAX_SESSIONS: usize = 1_000;

/// Scenarios the aim range knows.
const SCENARIOS: [&str; 3] = ["flick", "spray", "tracking"];

/// What a study consists of. Read from JSON; unknown fields are refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudyProtocol {
    /// Must equal [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Short identifier of the study, recorded in exports and session match ids.
    pub study_id: String,
    /// Baseline sessions: plain play, drift on or off, participant not told which.
    pub baseline: Vec<BaselineCondition>,
    /// Blind two-alternative comparisons.
    pub blind: BlindProtocol,
}

/// One kind of baseline session, played `repeats` times.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineCondition {
    /// Aim-range scenario.
    pub scenario: String,
    /// Drift amplitude (0: drift off), ppm.
    pub amplitude_ppm: u32,
    /// Session length, seconds of play.
    pub duration_s: f64,
    /// How many sessions of this condition.
    pub repeats: u32,
}

/// The blind comparison block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindProtocol {
    /// Aim-range scenario for every interval.
    pub scenario: String,
    /// Length of each of a trial's two intervals, seconds of play.
    pub interval_duration_s: f64,
    /// Drift amplitudes to compare against no drift, ppm. 0 makes a catch trial: both
    /// intervals are drift-free.
    pub amplitudes_ppm: Vec<u32>,
    /// Trials per amplitude. The drifted interval is A in half of them and B in the
    /// other half (the extra one of an odd count is random).
    pub repeats: u32,
}

/// Which of a trial's two intervals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interval {
    /// Played first.
    A,
    /// Played second.
    B,
}

/// One session to play.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedSession {
    /// Unique label, also the file name stem: `baseline-03`, `trial-07-A`.
    pub label: String,
    /// `drift-off`, or `drift-<ppm>ppm`.
    pub condition: String,
    /// Aim-range scenario.
    pub scenario: String,
    /// Target sequence seed (the aim range's `--seed`).
    pub scenario_seed: u32,
    /// Drift amplitude (0: drift off), ppm.
    pub amplitude_ppm: u32,
    /// Session length, seconds of play.
    pub duration_s: f64,
}

/// One blind two-alternative trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedTrial {
    /// Position in the blind block, from 0.
    pub index: u32,
    /// Amplitude of the drifted interval (0 for a catch trial), ppm.
    pub amplitude_ppm: u32,
    /// Which interval carries the drift.
    pub drifted: Interval,
    /// Interval A, then interval B. Both use the same target sequence.
    pub intervals: [PlannedSession; 2],
}

/// A participant's full, ordered plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudyPlan {
    /// Study identifier.
    pub study_id: String,
    /// Protocol version the plan was made from.
    pub protocol_version: u32,
    /// Randomisation seed, as a decimal string (JSON numbers lose precision above 2^53).
    pub seed: String,
    /// Baseline sessions in play order.
    pub baseline: Vec<PlannedSession>,
    /// Blind trials in play order.
    pub trials: Vec<PlannedTrial>,
}

/// Why a protocol cannot be planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudyError(pub String);

impl std::fmt::Display for StudyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StudyError {}

fn condition(amplitude_ppm: u32) -> String {
    if amplitude_ppm == 0 {
        "drift-off".to_owned()
    } else {
        format!("drift-{amplitude_ppm}ppm")
    }
}

fn check(protocol: &StudyProtocol) -> Result<(), StudyError> {
    let err = |m: String| Err(StudyError(m));
    if protocol.protocol_version != PROTOCOL_VERSION {
        return err(format!(
            "protocol_version {} is not {PROTOCOL_VERSION}",
            protocol.protocol_version
        ));
    }
    let id_ok = !protocol.study_id.is_empty()
        && protocol.study_id.len() <= 32
        && protocol
            .study_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !id_ok {
        return err("study_id must be 1 to 32 characters of A-Z, a-z, 0-9, '-' or '_'".to_owned());
    }
    let amplitude_ok = |ppm: u32| Amplitude::from_ppm(ppm).is_ok();
    let duration_ok = |s: f64| s.is_finite() && s > 0.0 && s <= 3_600.0;
    for c in &protocol.baseline {
        if !SCENARIOS.contains(&c.scenario.as_str())
            || !amplitude_ok(c.amplitude_ppm)
            || !duration_ok(c.duration_s)
        {
            return err(format!("invalid baseline condition {c:?}"));
        }
    }
    let b = &protocol.blind;
    if !SCENARIOS.contains(&b.scenario.as_str())
        || !duration_ok(b.interval_duration_s)
        || !b.amplitudes_ppm.iter().all(|a| amplitude_ok(*a))
    {
        return err(format!("invalid blind protocol {b:?}"));
    }
    let baseline: u64 = protocol.baseline.iter().map(|c| u64::from(c.repeats)).sum();
    let blind = 2 * b.amplitudes_ppm.len() as u64 * u64::from(b.repeats);
    if baseline + blind == 0 || baseline + blind > MAX_SESSIONS as u64 {
        return err(format!(
            "a plan must hold 1 to {MAX_SESSIONS} sessions, not {}",
            baseline + blind
        ));
    }
    Ok(())
}

/// Fisher–Yates shuffle with an unbiased bounded draw.
fn shuffle<T>(items: &mut [T], rng: &mut ChaCha20Rng) {
    for i in (1..items.len()).rev() {
        let j = below(rng, i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

/// Uniform in `0..n` (rejection sampling, no modulo bias).
fn below(rng: &mut ChaCha20Rng, n: u64) -> u64 {
    let zone = u64::MAX - u64::MAX % n;
    loop {
        let x = rng.next_u64();
        if x < zone {
            return x % n;
        }
    }
}

/// The match identifier whose probe key seeds `label`'s drift (player 0, epoch 0 under
/// the study key).
#[must_use]
pub fn session_match_id(study_id: &str, participant_id: &str, label: &str) -> String {
    format!("study:{study_id}:{participant_id}:{label}")
}

/// Makes a participant's plan.
///
/// # Errors
/// [`StudyError`] if the protocol is invalid.
pub fn plan(protocol: &StudyProtocol, seed: u64) -> Result<StudyPlan, StudyError> {
    check(protocol)?;
    let mut key = [0u8; 32];
    key[..18].copy_from_slice(b"rearguard-study-v1");
    key[24..].copy_from_slice(&seed.to_le_bytes());
    let mut rng = ChaCha20Rng::from_seed(key);

    // Baseline: every condition `repeats` times, in random order.
    let mut conditions: Vec<&BaselineCondition> = protocol
        .baseline
        .iter()
        .flat_map(|c| std::iter::repeat_n(c, c.repeats as usize))
        .collect();
    shuffle(&mut conditions, &mut rng);
    let baseline = conditions
        .into_iter()
        .enumerate()
        .map(|(i, c)| PlannedSession {
            label: format!("baseline-{:02}", i + 1),
            condition: condition(c.amplitude_ppm),
            scenario: c.scenario.clone(),
            scenario_seed: rng.next_u32(),
            amplitude_ppm: c.amplitude_ppm,
            duration_s: c.duration_s,
        })
        .collect();

    // Blind: each amplitude `repeats` times, the drifted interval balanced between A and
    // B within each amplitude, then everything in random order.
    let b = &protocol.blind;
    let mut trials: Vec<(u32, Interval)> = Vec::new();
    for &a in &b.amplitudes_ppm {
        let mut sides: Vec<Interval> = (0..b.repeats)
            .map(|i| if i % 2 == 0 { Interval::A } else { Interval::B })
            .collect();
        if b.repeats % 2 == 1 && rng.next_u64() & 1 == 1 {
            // Odd count: which side gets the extra trial is random.
            if let Some(last) = sides.last_mut() {
                *last = Interval::B;
            }
        }
        trials.extend(sides.into_iter().map(|side| (a, side)));
    }
    shuffle(&mut trials, &mut rng);
    let trials = trials
        .into_iter()
        .enumerate()
        .map(|(i, (amplitude, drifted))| {
            let scenario_seed = rng.next_u32();
            let interval = |which: Interval| {
                let drift = if which == drifted { amplitude } else { 0 };
                let name = if which == Interval::A { "A" } else { "B" };
                PlannedSession {
                    label: format!("trial-{:02}-{name}", i + 1),
                    condition: condition(drift),
                    scenario: b.scenario.clone(),
                    scenario_seed,
                    amplitude_ppm: drift,
                    duration_s: b.interval_duration_s,
                }
            };
            PlannedTrial {
                index: i as u32,
                amplitude_ppm: amplitude,
                drifted,
                intervals: [interval(Interval::A), interval(Interval::B)],
            }
        })
        .collect();

    Ok(StudyPlan {
        study_id: protocol.study_id.clone(),
        protocol_version: protocol.protocol_version,
        seed: seed.to_string(),
        baseline,
        trials,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> StudyProtocol {
        StudyProtocol {
            protocol_version: 1,
            study_id: "pilot-1".into(),
            baseline: vec![
                BaselineCondition {
                    scenario: "flick".into(),
                    amplitude_ppm: 0,
                    duration_s: 60.0,
                    repeats: 2,
                },
                BaselineCondition {
                    scenario: "flick".into(),
                    amplitude_ppm: 5_000,
                    duration_s: 60.0,
                    repeats: 2,
                },
                BaselineCondition {
                    scenario: "spray".into(),
                    amplitude_ppm: 0,
                    duration_s: 60.0,
                    repeats: 1,
                },
                BaselineCondition {
                    scenario: "spray".into(),
                    amplitude_ppm: 5_000,
                    duration_s: 60.0,
                    repeats: 1,
                },
            ],
            blind: BlindProtocol {
                scenario: "flick".into(),
                interval_duration_s: 12.0,
                amplitudes_ppm: vec![0, 2_500, 5_000, 10_000, 20_000],
                repeats: 4,
            },
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_plan() {
        let a = plan(&protocol(), 42).unwrap();
        let b = plan(&protocol(), 42).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        assert_eq!(a.seed, "42");
    }

    #[test]
    fn different_seeds_give_different_orders() {
        let a = plan(&protocol(), 1).unwrap();
        let b = plan(&protocol(), 2).unwrap();
        assert_ne!(a.trials, b.trials);
        assert_ne!(a.baseline, b.baseline);
    }

    #[test]
    fn plans_hold_every_condition_and_balance_the_drifted_side() {
        for seed in 0..50 {
            let p = plan(&protocol(), seed).unwrap();
            assert_eq!(p.baseline.len(), 6);
            let count = |ppm: u32, scenario: &str| {
                p.baseline
                    .iter()
                    .filter(|s| s.amplitude_ppm == ppm && s.scenario == scenario)
                    .count()
            };
            assert_eq!(
                (
                    count(0, "flick"),
                    count(5_000, "flick"),
                    count(0, "spray"),
                    count(5_000, "spray")
                ),
                (2, 2, 1, 1)
            );
            assert_eq!(p.trials.len(), 20);
            for a in [0, 2_500, 5_000, 10_000, 20_000] {
                let of: Vec<&PlannedTrial> =
                    p.trials.iter().filter(|t| t.amplitude_ppm == a).collect();
                assert_eq!(of.len(), 4);
                assert_eq!(
                    of.iter().filter(|t| t.drifted == Interval::A).count(),
                    2,
                    "seed {seed}, {a} ppm"
                );
            }
            for t in &p.trials {
                let [x, y] = &t.intervals;
                assert_eq!(
                    x.scenario_seed, y.scenario_seed,
                    "same targets in both intervals"
                );
                let drifted = if t.drifted == Interval::A { x } else { y };
                let plain = if t.drifted == Interval::A { y } else { x };
                assert_eq!(drifted.amplitude_ppm, t.amplitude_ppm);
                assert_eq!(plain.amplitude_ppm, 0);
                assert_eq!(plain.condition, "drift-off");
            }
            let labels: std::collections::HashSet<&str> = p
                .baseline
                .iter()
                .chain(p.trials.iter().flat_map(|t| t.intervals.iter()))
                .map(|s| s.label.as_str())
                .collect();
            assert_eq!(labels.len(), 6 + 40, "labels are unique");
        }
    }

    /// Pins the randomisation across platforms and releases: an analysis can rely on
    /// regenerating a plan from its seed.
    #[test]
    fn plan_golden() {
        let p = plan(&protocol(), 20_260_930).unwrap();
        let order: Vec<String> = p
            .baseline
            .iter()
            .map(|s| format!("{}/{}", s.scenario, s.condition))
            .collect();
        let trials: Vec<String> = p
            .trials
            .iter()
            .map(|t| format!("{}{:?}", t.amplitude_ppm, t.drifted))
            .collect();
        let golden = (
            order.join(","),
            trials.join(","),
            p.baseline[0].scenario_seed,
            p.trials[0].intervals[0].scenario_seed,
        );
        assert_eq!(format!("{golden:?}"), GOLDEN, "print: {golden:?}");
    }

    const GOLDEN: &str = r#"("flick/drift-off,spray/drift-off,flick/drift-off,flick/drift-5000ppm,spray/drift-5000ppm,flick/drift-5000ppm", "20000A,10000B,0B,5000B,0A,0A,2500B,20000B,5000A,5000A,0B,10000A,2500B,2500A,10000B,2500A,5000B,20000A,10000A,20000B", 4062689141, 1473242883)"#;

    #[test]
    fn invalid_protocols_are_refused() {
        let mut p = protocol();
        p.protocol_version = 2;
        assert!(plan(&p, 1).is_err());
        let mut p = protocol();
        p.study_id = "has space".into();
        assert!(plan(&p, 1).is_err());
        let mut p = protocol();
        p.blind.amplitudes_ppm.push(30_000);
        assert!(plan(&p, 1).is_err());
        let mut p = protocol();
        p.baseline[0].scenario = "chess".into();
        assert!(plan(&p, 1).is_err());
        let mut p = protocol();
        p.blind.repeats = 1_000;
        assert!(plan(&p, 1).is_err());
        let json = serde_json::to_string(&protocol())
            .unwrap()
            .replacen('{', "{\"extra\":1,", 1);
        assert!(serde_json::from_str::<StudyProtocol>(&json).is_err());
    }

    #[test]
    fn odd_repeats_still_cover_both_sides_over_seeds() {
        let mut p = protocol();
        p.blind.repeats = 3;
        let mut a_counts = std::collections::HashSet::new();
        for seed in 0..40 {
            let plan = plan(&p, seed).unwrap();
            let a = plan
                .trials
                .iter()
                .filter(|t| t.amplitude_ppm == 5_000 && t.drifted == Interval::A)
                .count();
            assert!(a == 1 || a == 2);
            a_counts.insert(a);
        }
        assert_eq!(a_counts.len(), 2, "the extra trial goes either way");
    }

    #[test]
    fn match_ids_name_the_study_participant_and_session() {
        assert_eq!(
            session_match_id("pilot-1", "P-ABCD", "trial-03-B"),
            "study:pilot-1:P-ABCD:trial-03-B"
        );
    }
}
