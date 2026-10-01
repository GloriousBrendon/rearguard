// SPDX-License-Identifier: MIT OR Apache-2.0

//! One session through the detector, as a server would run it: telemetry plus the
//! player's epoch seed, nothing else. Simulated, test-bot and human sessions all come
//! through here.

use rearguard_core::detect::{Detector, DetectorConfig};
use rearguard_core::probe::{Amplitude, EpochSeed, ProbeConfig};
use rearguard_core::telemetry::Record;

/// The session evidence at a moment when some statistic's score reached a new high.
#[derive(Clone, Debug, PartialEq)]
pub struct PathPoint {
    /// Telemetry time since the probe started, milliseconds.
    pub t_ms: u64,
    /// Shots so far.
    pub shots: u64,
    /// Engagements (flick targets, spray bursts) so far.
    pub engagements: u64,
    /// Every statistic's session score, in `Report::statistics` order.
    pub scores: [f64; 4],
}

/// A session's detector output.
#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    /// Every moment a statistic's session score rose above all its earlier values, in
    /// order. The first time any thresholds are passed is always one of these.
    pub path: Vec<PathPoint>,
    /// Each statistic's highest session score.
    pub peak: [f64; 4],
    /// Shots in the session.
    pub shots: u64,
    /// Shots whose reported view disagreed with the detector's replay. Not zero means
    /// the wrong seed or amplitude, or altered telemetry.
    pub angle_mismatches: u64,
    /// The header's scenario.
    pub scenario: String,
    /// The header's planned length, seconds.
    pub duration_s: f64,
}

/// A traced session and what the evaluation knows about it. The detector saw none of
/// these labels.
#[derive(Clone, Debug, PartialEq)]
pub struct Traced {
    /// `sim`, `bot` or `human`.
    pub source: &'static str,
    /// `human`, a cheat's name, or `scripted-bot`.
    pub class: String,
    /// `flick` or `spray`.
    pub scenario: String,
    /// Drift amplitude the session ran under, ppm.
    pub amplitude_ppm: u32,
    /// Who played, for the calibration/evaluation split (people only; else empty).
    pub participant: String,
    /// The detector's output.
    pub trace: Trace,
}

/// Streams `records` through a detector, reading the session evidence after every
/// record that can change it (everything but mouse moves).
///
/// # Errors
/// An amplitude out of range, or telemetry the detector refuses.
pub fn trace(
    records: &[Record],
    epoch: &EpochSeed,
    amplitude_ppm: u32,
    config: &DetectorConfig,
) -> Result<Trace, String> {
    let Some(Record::Header(header)) = records.first() else {
        return Err("no telemetry header".to_owned());
    };
    let probe = ProbeConfig {
        amplitude: Amplitude::from_ppm(amplitude_ppm).map_err(|e| e.to_string())?,
        ..ProbeConfig::default()
    };
    let mut detector = Detector::new(epoch, &probe, config.clone()).map_err(|e| e.to_string())?;
    let mut out = Trace {
        path: Vec::new(),
        peak: [f64::NEG_INFINITY; 4],
        shots: 0,
        angle_mismatches: 0,
        scenario: header.scenario.clone(),
        duration_s: header.duration_s,
    };
    let start_us = header.probe_start_us;
    for record in records {
        detector
            .feed(std::slice::from_ref(record))
            .map_err(|e| e.to_string())?;
        if matches!(record, Record::Move(_)) {
            continue;
        }
        let report = detector.session();
        let scores = report.statistics().map(|(_, e)| e.score);
        let mut rose = false;
        for (peak, score) in out.peak.iter_mut().zip(scores) {
            if score > *peak {
                *peak = score;
                rose = true;
            }
        }
        if rose {
            out.path.push(PathPoint {
                t_ms: record.ts_us().saturating_sub(start_us) / 1_000,
                shots: report.shots,
                engagements: report.engagements,
                scores,
            });
        }
        out.shots = report.shots;
        out.angle_mismatches = report.angle_mismatches;
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use rearguard_sim::seed::SimSeed;
    use rearguard_sim::session::{Class, ModelParams, Session, SessionSpec, run_session};
    use rearguard_sim::world::Scenario;

    use super::*;
    use crate::config::tests::small;

    pub(crate) fn sim_session(
        class: Class,
        scenario: Scenario,
        index: u32,
        ppm: u32,
    ) -> (Session, EpochSeed) {
        let seed = SimSeed::new(11);
        let root = seed.probe_root();
        let spec = SessionSpec {
            class,
            scenario,
            index,
            duration_s: 10.0,
            amplitude: Amplitude::from_ppm(ppm).unwrap(),
            models: ModelParams::DEFAULT,
        };
        let session = run_session(&seed, &root, spec);
        let epoch = root
            .match_key(session.match_id.as_bytes())
            .player_key(session.player_id)
            .epoch_seed(0);
        (session, epoch)
    }

    #[test]
    fn the_path_holds_every_new_high_and_ends_at_the_peak() {
        let config = small().detector.config(None);
        let (session, epoch) = sim_session(Class::FlickAimbot, Scenario::Flick, 0, 10_000);
        let t = trace(&session.records, &epoch, 10_000, &config).unwrap();
        assert_eq!(t.angle_mismatches, 0);
        assert!(t.shots > 50 && t.scenario == "flick" && t.duration_s == 10.0);
        assert!(t.peak[0] > 100.0, "an open-loop aimbot: {:?}", t.peak);
        // Every statistic's running maximum along the path ends at its peak.
        for i in 0..4 {
            let high = t
                .path
                .iter()
                .map(|p| p.scores[i])
                .fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(high, t.peak[i]);
        }
        assert!(
            t.path
                .windows(2)
                .all(|w| w[0].t_ms <= w[1].t_ms && w[0].shots <= w[1].shots)
        );
        assert!(t.path.last().unwrap().t_ms <= 10_000);
    }

    #[test]
    fn the_wrong_seed_or_amplitude_shows_as_angle_mismatches() {
        let config = small().detector.config(None);
        let (session, epoch) = sim_session(Class::Human, Scenario::Flick, 0, 10_000);
        let (_, other) = sim_session(Class::Human, Scenario::Flick, 1, 10_000);
        assert_eq!(
            trace(&session.records, &epoch, 10_000, &config)
                .unwrap()
                .angle_mismatches,
            0
        );
        assert!(
            trace(&session.records, &other, 10_000, &config)
                .unwrap()
                .angle_mismatches
                > 0
        );
        assert!(
            trace(&session.records, &epoch, 5_000, &config)
                .unwrap()
                .angle_mismatches
                > 0
        );
    }

    #[test]
    fn bad_input_is_an_error() {
        let config = small().detector.config(None);
        let (session, epoch) = sim_session(Class::Human, Scenario::Flick, 0, 10_000);
        assert!(trace(&session.records[1..], &epoch, 10_000, &config).is_err());
        assert!(trace(&[], &epoch, 10_000, &config).is_err());
        assert!(trace(&session.records, &epoch, 30_000, &config).is_err());
        let mut twice = session.records.clone();
        twice.push(session.records[0].clone());
        assert!(trace(&twice, &epoch, 10_000, &config).is_err());
    }
}
