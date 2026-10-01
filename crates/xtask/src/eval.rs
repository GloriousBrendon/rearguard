// SPDX-License-Identifier: MIT OR Apache-2.0

//! The evaluation: simulate, trace, calibrate, measure.
//!
//! Two threshold sets are calibrated, each per drift amplitude and scenario:
//!
//! - `sim`: on simulated humans, measured on other simulated humans that were held
//!   out (and on people's evaluation sessions, when there are any);
//! - `human`: on the baseline sessions of the participants in the calibration set,
//!   measured on the participants in the evaluation set (and on the held-out simulated
//!   humans). Present only when human sessions were given.
//!
//! Detection rates are measured against both, on simulated cheats and on the aim
//! range's test bots. Nothing here reads a wall clock, a thread count or a path, so the
//! same inputs give the same [`Results`].

use std::collections::BTreeSet;

use rearguard_core::probe::Amplitude;
use rearguard_sim::evaluate::parallel_map;
use rearguard_sim::seed::SimSeed;
use rearguard_sim::session::{Class, ModelParams, SessionSpec, run_session};
use rearguard_sim::world::Scenario;

use crate::analysis::{
    Count, Thresholds, calibrate, detections, false_positives, in_sample, roc_targets,
};
use crate::config::{EvalConfig, SCENARIOS};
use crate::inputs::{Loaded, Tally};
use crate::split::{self, Calibration, Evaluation};
use crate::trace::{Traced, trace};

/// The class name of the aim range's scripted player: a bot that is not a cheat.
pub const CONTROL_CLASS: &str = "scripted-bot";

/// Everything the evaluation runs on.
#[derive(Debug)]
pub struct Inputs {
    /// Simulation seed. Never written out.
    pub seed: SimSeed,
    /// Human-study sessions, if any were given.
    pub humans: Option<Loaded>,
    /// Test-bot sessions, if any were given.
    pub bots: Option<Loaded>,
    /// Worker threads. The results do not depend on it.
    pub threads: usize,
}

/// What a rate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The calibration sessions under their own thresholds: a check, not a measurement.
    CalibrationCheck,
    /// Humans that played no part in the calibration, flagged: a false-positive rate.
    FalsePositive,
    /// The scripted player, flagged: a bot, but not a cheat.
    Control,
    /// Cheats flagged: a detection rate.
    Detection,
}

impl Kind {
    /// Name in the result files.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::CalibrationCheck => "calibration-check",
            Self::FalsePositive => "false-positive",
            Self::Control => "control",
            Self::Detection => "detection",
        }
    }
}

/// Where a row sits: threshold set, amplitude, scenario, and whose sessions.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Group {
    /// `sim` or `human`: what the thresholds were calibrated on.
    pub set: &'static str,
    /// Drift amplitude, ppm.
    pub amplitude_ppm: u32,
    /// `flick` or `spray`.
    pub scenario: &'static str,
    /// `sim`, `bot` or `human`: where the measured sessions came from.
    pub source: &'static str,
    /// `human`, a cheat's name, or `scripted-bot`.
    pub class: String,
}

/// One scenario's calibrated thresholds.
#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdRow {
    /// Threshold set, amplitude and scenario (`source` is the set's, `class` `human`).
    pub group: Group,
    /// The thresholds.
    pub thresholds: Thresholds,
    /// Distinct people calibrated on (0 for simulated humans).
    pub participants: usize,
}

/// A rate.
#[derive(Clone, Debug, PartialEq)]
pub struct RateRow {
    /// Which sessions, under which thresholds.
    pub group: Group,
    /// What the rate is.
    pub kind: Kind,
    /// Sessions and flags.
    pub count: Count,
}

/// One point of an operating curve: thresholds calibrated at `target`, then measured.
#[derive(Clone, Debug, PartialEq)]
pub struct RocRow {
    /// The cheat group.
    pub group: Group,
    /// False-positive rate the thresholds were calibrated at.
    pub target: f64,
    /// The threshold set's held-out humans at those thresholds.
    pub negatives: Count,
    /// The cheats at those thresholds.
    pub positives: Count,
}

/// When a cheat group's sessions were first flagged.
#[derive(Clone, Debug, PartialEq)]
pub struct TtdRow {
    /// The cheat group.
    pub group: Group,
    /// Sessions in the group.
    pub sessions: usize,
    /// Seconds to the first flag, ascending, for the sessions flagged.
    pub seconds: Vec<f64>,
    /// Shots to the first flag, ascending.
    pub shots: Vec<f64>,
    /// Engagements to the first flag, ascending.
    pub engagements: Vec<f64>,
}

/// How many sessions of each kind went in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DataRow {
    /// `sim`, `bot` or `human`.
    pub source: &'static str,
    /// `calibration`, `evaluation`, `cheat` or `control`.
    pub role: &'static str,
    /// Drift amplitude, ppm.
    pub amplitude_ppm: u32,
    /// Scenario.
    pub scenario: String,
    /// Class.
    pub class: String,
    /// Sessions.
    pub sessions: usize,
    /// Distinct people (0 for simulated and bot sessions).
    pub participants: usize,
}

/// The evaluation's results, ready to be written out.
#[derive(Clone, Debug, PartialEq)]
pub struct Results {
    /// Sessions that went in.
    pub data: Vec<DataRow>,
    /// Calibrated thresholds.
    pub thresholds: Vec<ThresholdRow>,
    /// False-positive and detection rates.
    pub rates: Vec<RateRow>,
    /// Operating curves.
    pub roc: Vec<RocRow>,
    /// Times to detection.
    pub ttd: Vec<TtdRow>,
    /// Recorded sessions left out.
    pub excluded: Tally,
    /// The inputs, as one line of JSON: counts and digests, no names, paths or seeds.
    pub inputs_json: String,
    /// Human exports that are not from people (see `Loaded::test_exports`).
    pub test_exports: usize,
}

fn simulate(
    config: &EvalConfig,
    inputs: &Inputs,
    jobs: &[(u32, Class, Scenario, u32)],
) -> Vec<Traced> {
    let root = inputs.seed.probe_root();
    let detector = config.detector.config(None);
    parallel_map(jobs, inputs.threads, |&(ppm, class, scenario, index)| {
        let spec = SessionSpec {
            class,
            scenario,
            index,
            duration_s: config.horizon_s,
            amplitude: Amplitude::from_ppm(ppm).expect("checked with the configuration"),
            models: ModelParams::DEFAULT,
        };
        let session = run_session(&inputs.seed, &root, spec);
        // The server's view: the player's epoch seed from its own key hierarchy.
        let epoch = root
            .match_key(session.match_id.as_bytes())
            .player_key(session.player_id)
            .epoch_seed(0);
        Traced {
            source: "sim",
            class: class.name().to_owned(),
            scenario: scenario.name().to_owned(),
            amplitude_ppm: ppm,
            participant: String::new(),
            trace: trace(&session.records, &epoch, ppm, &detector)
                .expect("simulated telemetry is valid"),
        }
    })
}

fn human_jobs(
    config: &EvalConfig,
    indices: std::ops::Range<u32>,
) -> Vec<(u32, Class, Scenario, u32)> {
    let mut jobs = Vec::new();
    for &ppm in &config.amplitudes_ppm {
        for scenario in [Scenario::Flick, Scenario::Spray] {
            jobs.extend(indices.clone().map(|i| (ppm, Class::Human, scenario, i)));
        }
    }
    jobs
}

fn participants<'a>(sessions: impl IntoIterator<Item = &'a &'a Traced>) -> usize {
    sessions
        .into_iter()
        .filter(|s| !s.participant.is_empty())
        .map(|s| s.participant.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

fn data_rows(role: &'static str, sessions: &[Traced], rows: &mut Vec<DataRow>) {
    let keys: BTreeSet<_> = sessions
        .iter()
        .map(|s| {
            (
                s.source,
                s.amplitude_ppm,
                s.scenario.clone(),
                s.class.clone(),
            )
        })
        .collect();
    for (source, amplitude_ppm, scenario, class) in keys {
        let group: Vec<&Traced> = sessions
            .iter()
            .filter(|s| {
                s.source == source
                    && s.amplitude_ppm == amplitude_ppm
                    && s.scenario == scenario
                    && s.class == class
            })
            .collect();
        let role = match (role, class.as_str()) {
            ("cheat", CONTROL_CLASS) => "control",
            _ => role,
        };
        rows.push(DataRow {
            source,
            role,
            amplitude_ppm,
            scenario,
            class,
            sessions: group.len(),
            participants: participants(&group),
        });
    }
}

fn input_summary(loaded: Option<&Loaded>, unit: &str) -> serde_json::Value {
    loaded.map_or(serde_json::Value::Null, |l| {
        serde_json::json!({
            unit: l.units,
            "sessions_used": l.sessions.len(),
            "sessions_left_out": l.excluded.0.values().sum::<u64>(),
            "sha256": l.sha256,
        })
    })
}

/// Runs the evaluation.
///
/// # Errors
/// A participant in both the calibration and the evaluation set (which the split makes
/// impossible; checked all the same).
pub fn evaluate(config: &EvalConfig, mut inputs: Inputs) -> Result<Results, String> {
    let inputs_json = serde_json::json!({
        "sim_seed": "given on the command line; never written out",
        "humans": input_summary(inputs.humans.as_ref(), "exports"),
        "bots": input_summary(inputs.bots.as_ref(), "recordings"),
    })
    .to_string();
    let mut excluded = Tally::default();
    let mut test_exports = 0;
    let mut human_sessions = Vec::new();
    if let Some(humans) = inputs.humans.take() {
        excluded.0.extend(humans.excluded.0);
        test_exports = humans.test_exports;
        human_sessions = humans.sessions;
    }
    let mut positives = Vec::new();
    if let Some(bots) = inputs.bots.take() {
        excluded.0.extend(bots.excluded.0);
        positives = bots.sessions;
    }

    // People: every participant in one set only.
    let (human_cal, human_eval) =
        split::by_participant(human_sessions, |s| &s.participant, &config.human_split);
    split::check_disjoint(&human_cal, &human_eval, |s| &s.participant)?;

    // Simulated humans: separate session indices for calibration and for measuring.
    let (cal_indices, held_out_indices) =
        split::sim_humans(config.sim.calibration_humans, config.sim.held_out_humans);
    let sim_cal = cal_indices.map(|range| simulate(config, &inputs, &human_jobs(config, range)));
    let sim_eval =
        held_out_indices.map(|range| simulate(config, &inputs, &human_jobs(config, range)));

    // Simulated cheats, then the test bots.
    let mut cheat_jobs = Vec::new();
    for &ppm in &config.amplitudes_ppm {
        for class in Class::ALL.into_iter().filter(|c| *c != Class::Human) {
            for &scenario in class.scenarios() {
                cheat_jobs.extend((0..config.sim.cheats).map(|i| (ppm, class, scenario, i)));
            }
        }
    }
    let mut cheats = simulate(config, &inputs, &cheat_jobs);
    cheats.append(&mut positives);
    let positives = cheats;

    let mut results = Results {
        data: Vec::new(),
        thresholds: Vec::new(),
        rates: Vec::new(),
        roc: Vec::new(),
        ttd: Vec::new(),
        excluded,
        inputs_json,
        test_exports,
    };
    data_rows("calibration", sim_cal.get(), &mut results.data);
    data_rows("evaluation", sim_eval.get(), &mut results.data);
    data_rows("calibration", human_cal.get(), &mut results.data);
    data_rows("evaluation", human_eval.get(), &mut results.data);
    data_rows("cheat", &positives, &mut results.data);
    results.data.sort();

    let amplitudes: BTreeSet<u32> = config
        .amplitudes_ppm
        .iter()
        .copied()
        .chain(human_cal.get().iter().map(|s| s.amplitude_ppm))
        .collect();
    let sets: [(&'static str, &Calibration<Vec<Traced>>); 2] =
        [("sim", &sim_cal), ("human", &human_cal)];
    let targets = roc_targets(config.fpr_target);
    for (set, calibration) in sets {
        for &amplitude_ppm in &amplitudes {
            for scenario in SCENARIOS {
                let here = |s: &Traced| s.amplitude_ppm == amplitude_ppm && s.scenario == scenario;
                let cal = calibration.filter(here);
                let stats = config.detector.applicable(scenario);
                let Some(thresholds) = calibrate(&cal, &stats, config.fpr_target) else {
                    continue;
                };
                let group = |source: &'static str, class: &str| Group {
                    set,
                    amplitude_ppm,
                    scenario,
                    source,
                    class: class.to_owned(),
                };
                let own_source = if set == "sim" { "sim" } else { "human" };
                results.rates.push(RateRow {
                    group: group(own_source, "human"),
                    kind: Kind::CalibrationCheck,
                    count: in_sample(&cal, &thresholds),
                });
                results.thresholds.push(ThresholdRow {
                    group: group(own_source, "human"),
                    participants: participants(cal.get()),
                    thresholds: thresholds.clone(),
                });

                // False positives: only sessions that set no threshold.
                let negatives: [(&'static str, Evaluation<Vec<&Traced>>); 2] = [
                    ("sim", sim_eval.filter(here)),
                    ("human", human_eval.filter(here)),
                ];
                for (source, sessions) in &negatives {
                    if !sessions.get().is_empty() {
                        results.rates.push(RateRow {
                            group: group(source, "human"),
                            kind: Kind::FalsePositive,
                            count: false_positives(sessions, &thresholds),
                        });
                    }
                }
                let own_negatives = &negatives[usize::from(set == "human")].1;

                // Cheats and the scripted control.
                let classes: BTreeSet<(&'static str, &str)> = positives
                    .iter()
                    .filter(|s| here(s))
                    .map(|s| (s.source, s.class.as_str()))
                    .collect();
                for (source, class) in classes {
                    let sessions: Vec<&Traced> = positives
                        .iter()
                        .filter(|s| here(s) && s.source == source && s.class == class)
                        .collect();
                    let control = class == CONTROL_CLASS;
                    results.rates.push(RateRow {
                        group: group(source, class),
                        kind: if control {
                            Kind::Control
                        } else {
                            Kind::Detection
                        },
                        count: detections(&sessions, &thresholds),
                    });
                    if control {
                        continue;
                    }
                    let first: Vec<_> = sessions
                        .iter()
                        .filter_map(|s| thresholds.first_flag(&s.trace))
                        .collect();
                    let sorted = |pick: &dyn Fn(&crate::trace::PathPoint) -> f64| {
                        let mut v: Vec<f64> = first.iter().map(|p| pick(p)).collect();
                        v.sort_by(f64::total_cmp);
                        v
                    };
                    results.ttd.push(TtdRow {
                        group: group(source, class),
                        sessions: sessions.len(),
                        seconds: sorted(&|p| p.t_ms as f64 / 1e3),
                        shots: sorted(&|p| p.shots as f64),
                        engagements: sorted(&|p| p.engagements as f64),
                    });
                    if own_negatives.get().is_empty() {
                        continue;
                    }
                    for &target in &targets {
                        let Some(at) = calibrate(&cal, &stats, target) else {
                            continue;
                        };
                        results.roc.push(RocRow {
                            group: group(source, class),
                            target,
                            negatives: false_positives(own_negatives, &at),
                            positives: detections(&sessions, &at),
                        });
                    }
                }
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use rearguard_core::detect::Detector;
    use rearguard_core::probe::ProbeConfig;
    use rearguard_core::telemetry::Record;

    use super::*;
    use crate::config::tests::small;
    use crate::inputs::tests::{KEY_HEX, temp_dir, write_bot_recordings, write_export_zip};
    use crate::inputs::{load_bots, load_humans};

    /// Inputs with eight simulated "participants" and a few bot recordings.
    pub(crate) fn inputs(name: &str, threads: usize) -> Inputs {
        let config = small();
        let dir = temp_dir(name);
        let (exports, bots) = (dir.join("exports"), dir.join("bots"));
        fs::create_dir_all(&exports).unwrap();
        fs::create_dir_all(&bots).unwrap();
        for i in 0..8 {
            write_export_zip(&exports, &format!("P-{i:04}"), i, false);
        }
        write_bot_recordings(&bots);
        let key = || rearguard_core::probe::RootSeed::from_hex(KEY_HEX).unwrap();
        Inputs {
            seed: SimSeed::new(7),
            humans: Some(load_humans(&[exports], &[key()], &config, false).unwrap()),
            bots: Some(load_bots(&bots, &key(), &config).unwrap()),
            threads,
        }
    }

    /// One evaluation of [`inputs`] under the small configuration, shared by the tests
    /// that only read it.
    pub(crate) fn shared() -> &'static Results {
        static RESULTS: std::sync::OnceLock<Results> = std::sync::OnceLock::new();
        RESULTS.get_or_init(|| evaluate(&small(), inputs("eval-shared", 4)).unwrap())
    }

    #[test]
    fn human_sessions_calibrate_or_evaluate_never_both() {
        let results = shared();
        let people = |role: &str| -> usize {
            results
                .data
                .iter()
                .filter(|d| d.source == "human" && d.role == role && d.scenario == "spray")
                .map(|d| d.participants)
                .sum()
        };
        // Eight participants, each in one set.
        assert!(people("calibration") > 0 && people("evaluation") > 0);
        assert_eq!(people("calibration") + people("evaluation"), 8);
        let sessions = |role: &str| -> usize {
            results
                .data
                .iter()
                .filter(|d| d.source == "human" && d.role == role)
                .map(|d| d.sessions)
                .sum()
        };
        assert_eq!(sessions("calibration") + sessions("evaluation"), 24);
        assert_eq!(sessions("calibration"), 3 * people("calibration"));

        // The human thresholds are calibrated on exactly the calibration sessions, and
        // the human false-positive rates are measured on exactly the evaluation ones.
        for row in results.thresholds.iter().filter(|t| t.group.set == "human") {
            let expected: usize = results
                .data
                .iter()
                .filter(|d| {
                    d.source == "human"
                        && d.role == "calibration"
                        && d.scenario == row.group.scenario
                })
                .map(|d| d.sessions)
                .sum();
            assert_eq!(row.thresholds.sessions, expected);
            assert_eq!(row.participants, people("calibration"));
        }
        for row in results.rates.iter().filter(|r| r.group.source == "human") {
            let role = match row.kind {
                Kind::CalibrationCheck => "calibration",
                Kind::FalsePositive => "evaluation",
                other => panic!("people are never positives: {other:?}"),
            };
            let expected: usize = results
                .data
                .iter()
                .filter(|d| {
                    d.source == "human" && d.role == role && d.scenario == row.group.scenario
                })
                .map(|d| d.sessions)
                .sum();
            assert_eq!(row.count.sessions as usize, expected, "{row:?}");
            // A human calibration check appears only under the human thresholds.
            assert!(row.kind != Kind::CalibrationCheck || row.group.set == "human");
        }
    }

    #[test]
    fn simulated_false_positives_are_measured_on_held_out_humans() {
        let config = small();
        let results = shared();
        for row in results
            .rates
            .iter()
            .filter(|r| r.group.source == "sim" && r.group.class == "human")
        {
            let expected = match row.kind {
                Kind::CalibrationCheck => config.sim.calibration_humans,
                Kind::FalsePositive => config.sim.held_out_humans,
                other => panic!("{other:?}"),
            };
            assert_eq!(row.count.sessions, u64::from(expected), "{row:?}");
        }
        // Both scenarios, both amplitudes, under the sim thresholds; and the human
        // thresholds are measured on the held-out simulated humans too.
        let held_out = |set: &str| {
            results
                .rates
                .iter()
                .filter(|r| {
                    r.group.set == set && r.group.source == "sim" && r.kind == Kind::FalsePositive
                })
                .count()
        };
        assert_eq!(held_out("sim"), 4);
        assert_eq!(held_out("human"), 2, "people played at one amplitude");
        // The calibration humans never exceed their own target.
        for row in results
            .rates
            .iter()
            .filter(|r| r.kind == Kind::CalibrationCheck)
        {
            assert!(
                row.count.flagged as f64 <= config.fpr_target * row.count.sessions as f64,
                "{row:?}"
            );
        }
    }

    #[test]
    fn thresholds_differ_by_scenario_and_cheats_are_detected() {
        let config = small();
        let results = shared();
        let thresholds = |set: &str, scenario: &str| {
            &results
                .thresholds
                .iter()
                .find(|t| {
                    t.group.set == set
                        && t.group.scenario == scenario
                        && t.group.amplitude_ppm == 20_000
                })
                .unwrap()
                .thresholds
        };
        for set in ["sim", "human"] {
            let (flick, spray) = (thresholds(set, "flick"), thresholds(set, "spray"));
            assert_eq!(
                (flick.stats.as_slice(), spray.stats.as_slice()),
                (&[0, 1, 2][..], &[0, 1, 3][..])
            );
            assert_ne!(flick.values, spray.values, "{set}");
        }
        let rate = |set: &str, source: &str, class: &str| {
            results
                .rates
                .iter()
                .find(|r| {
                    r.group.set == set
                        && r.group.source == source
                        && r.group.class == class
                        && r.group.amplitude_ppm == 20_000
                })
                .unwrap()
        };
        for set in ["sim", "human"] {
            let sim = rate(set, "sim", "flick-aimbot");
            assert_eq!(
                (sim.kind, sim.count.sessions, sim.count.flagged),
                (Kind::Detection, 3, 3)
            );
            let bot = rate(set, "bot", "flick-aimbot");
            assert_eq!((bot.count.sessions, bot.count.flagged), (2, 2));
            assert_eq!(rate(set, "bot", CONTROL_CLASS).kind, Kind::Control);
        }
        // Flagged within the session, at a time on its path.
        let ttd = results
            .ttd
            .iter()
            .find(|t| {
                t.group.set == "sim"
                    && t.group.class == "flick-aimbot"
                    && t.group.source == "sim"
                    && t.group.amplitude_ppm == 20_000
            })
            .unwrap();
        assert_eq!((ttd.sessions, ttd.seconds.len()), (3, 3));
        assert!(
            ttd.seconds
                .iter()
                .all(|s| *s > 0.0 && *s <= config.horizon_s)
        );
        assert!(results.ttd.iter().all(|t| t.group.class != CONTROL_CLASS));
        // Operating curves: more false positives allowed never means fewer detections.
        let curve: Vec<&RocRow> = results
            .roc
            .iter()
            .filter(|r| {
                r.group.set == "sim"
                    && r.group.class == "humanised-aimbot"
                    && r.group.amplitude_ppm == 20_000
            })
            .collect();
        assert_eq!(curve.len(), roc_targets(config.fpr_target).len());
        assert!(
            curve
                .windows(2)
                .all(|w| w[0].positives.flagged <= w[1].positives.flagged)
        );
        assert!(
            curve
                .windows(2)
                .all(|w| w[0].negatives.flagged <= w[1].negatives.flagged)
        );
        // Left-out recordings are carried through, and inputs are named by digest only.
        assert_eq!(results.excluded.problems(), 2);
        assert!(results.inputs_json.contains("sha256") && !results.inputs_json.contains("P-0"));
    }

    /// The flag scores written for the server make the detector itself flag at the
    /// same record the evaluation says the session is first flagged.
    #[test]
    fn the_detector_flags_where_the_evaluation_says() {
        let config = small();
        let (session, epoch) =
            crate::trace::tests::sim_session(Class::HumanisedAimbot, Scenario::Flick, 1, 20_000);
        let traced = trace(
            &session.records,
            &epoch,
            20_000,
            &config.detector.config(None),
        )
        .unwrap();
        // A threshold the session passes part of the way through.
        let mut values = [f64::INFINITY; 4];
        values[0] = traced.peak[0] / 2.0;
        assert!(values[0] > 0.0, "{:?}", traced.peak);
        let thresholds = Thresholds {
            stats: vec![0],
            values,
            sessions: 1,
            share: 1.0,
        };
        let expected = thresholds.first_flag(&traced).unwrap();

        let flags = thresholds.flag_scores(f64::MAX);
        let probe = ProbeConfig {
            amplitude: Amplitude::from_ppm(20_000).unwrap(),
            ..ProbeConfig::default()
        };
        let mut detector =
            Detector::new(&epoch, &probe, config.detector.config(Some(flags))).unwrap();
        let start = match &session.records[0] {
            Record::Header(h) => h.probe_start_us,
            _ => unreachable!(),
        };
        let mut first = None;
        for record in &session.records {
            detector.feed(std::slice::from_ref(record)).unwrap();
            if first.is_none() && detector.session().flagged() {
                first = Some((record.ts_us() - start) / 1_000);
            }
        }
        assert_eq!(first, Some(expected.t_ms));
    }
}
