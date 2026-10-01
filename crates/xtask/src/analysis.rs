// SPDX-License-Identifier: MIT OR Apache-2.0

//! Thresholds from a calibration set, and rates at those thresholds.
//!
//! **The flag rule.** A session is flagged if, at any moment up to the horizon, the
//! session score of any statistic its scenario runs is above that statistic's
//! threshold. This is what a server polling a live verdict sees; the verdict at the end
//! of a session is the same test at the last record, so it flags no more often.
//!
//! **Calibration.** Each statistic's threshold is the smallest value that at most a
//! share of the calibration sessions' highest scores exceed, the share being the target
//! false-positive rate divided by the number of statistics (a union bound: together
//! they flag at most the target share of the calibration sessions). With `n`
//! calibration sessions no rate below `1/n` can be told from zero, and the threshold is
//! then simply the highest score seen.
//!
//! A statistic that no calibration session scored on (every highest score zero or
//! below) cannot be calibrated and is left off: it never flags.
//!
//! The functions take [`Calibration`] and [`Evaluation`] sets, not plain lists, so
//! thresholds can only come from the first and false-positive rates only from the
//! second.

use rearguard_sim::evaluate::threshold_at;

use crate::split::{Calibration, Evaluation};
use crate::trace::{PathPoint, Trace, Traced};

/// One scenario's thresholds.
#[derive(Clone, Debug, PartialEq)]
pub struct Thresholds {
    /// The scenario's statistics (indices in `Report::statistics` order).
    pub stats: Vec<usize>,
    /// Flag when a statistic's score is above its value. Infinite (never flags) for a
    /// statistic the scenario does not run, and for one that no calibration session
    /// scored on.
    pub values: [f64; 4],
    /// Calibration sessions used.
    pub sessions: usize,
    /// False-positive share each statistic was calibrated at.
    pub share: f64,
}

impl Thresholds {
    /// Whether `trace` is flagged, and by which statistics.
    #[must_use]
    pub fn flags(&self, trace: &Trace) -> [bool; 4] {
        let mut by = [false; 4];
        for &i in &self.stats {
            by[i] = trace.peak[i] > self.values[i];
        }
        by
    }

    /// The moment `trace` is first flagged, if it is.
    #[must_use]
    pub fn first_flag<'a>(&self, trace: &'a Trace) -> Option<&'a PathPoint> {
        trace
            .path
            .iter()
            .find(|p| self.stats.iter().any(|&i| p.scores[i] > self.values[i]))
    }

    /// The detector flag scores that flag exactly the sessions these thresholds do:
    /// the detector flags at `score >= flag score`, so each is the next number above
    /// its threshold. A statistic that never flags keeps `unused`.
    #[must_use]
    pub fn flag_scores(&self, unused: f64) -> [f64; 4] {
        let mut scores = [unused; 4];
        for &i in &self.stats {
            if self.values[i].is_finite() {
                scores[i] = self.values[i].next_up();
            }
        }
        scores
    }
}

/// Calibrates one scenario's thresholds at a total false-positive rate of `fpr`, or
/// `None` without calibration sessions.
#[must_use]
pub fn calibrate(
    sessions: &Calibration<Vec<&Traced>>,
    stats: &[usize],
    fpr: f64,
) -> Option<Thresholds> {
    let sessions = sessions.get();
    if sessions.is_empty() || stats.is_empty() {
        return None;
    }
    let share = fpr / stats.len() as f64;
    let mut values = [f64::INFINITY; 4];
    for &i in stats {
        let peaks: Vec<f64> = sessions.iter().map(|s| s.trace.peak[i]).collect();
        // A statistic no calibration session ever scored on has nothing to calibrate
        // against: it stays off (never flags), instead of flagging at the first score
        // above zero.
        if peaks.iter().any(|p| *p > 0.0) {
            values[i] = threshold_at(&peaks, share);
        }
    }
    Some(Thresholds {
        stats: stats.to_vec(),
        values,
        sessions: sessions.len(),
        share,
    })
}

/// How many of a group of sessions were flagged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    /// Sessions.
    pub sessions: u64,
    /// Flagged by any statistic.
    pub flagged: u64,
    /// Flagged by each statistic (a session can count under several).
    pub by_statistic: [u64; 4],
}

impl Count {
    /// Flagged share, or `None` without sessions.
    #[must_use]
    pub fn rate(&self) -> Option<f64> {
        (self.sessions > 0).then(|| self.flagged as f64 / self.sessions as f64)
    }
}

fn count<'a>(sessions: impl IntoIterator<Item = &'a Traced>, thresholds: &Thresholds) -> Count {
    let mut c = Count::default();
    for session in sessions {
        let by = thresholds.flags(&session.trace);
        c.sessions += 1;
        c.flagged += u64::from(by.contains(&true));
        for (n, flagged) in c.by_statistic.iter_mut().zip(by) {
            *n += u64::from(flagged);
        }
    }
    c
}

/// False positives among sessions that played no part in the calibration.
#[must_use]
pub fn false_positives(sessions: &Evaluation<Vec<&Traced>>, thresholds: &Thresholds) -> Count {
    count(sessions.get().iter().copied(), thresholds)
}

/// The calibration sessions' own flag count. Not a false-positive rate: the thresholds
/// were fitted to these sessions, so it is at most the target by construction. Reported
/// as a check, without an interval.
#[must_use]
pub fn in_sample(sessions: &Calibration<Vec<&Traced>>, thresholds: &Thresholds) -> Count {
    count(sessions.get().iter().copied(), thresholds)
}

/// Flags among sessions known not to be people (cheats, or a scripted control). They
/// are never calibrated on, so they need no split.
#[must_use]
pub fn detections(sessions: &[&Traced], thresholds: &Thresholds) -> Count {
    count(sessions.iter().copied(), thresholds)
}

/// The value at quantile `q` of ascending `sorted` (nearest rank), or `None` if empty.
#[must_use]
pub fn quantile(sorted: &[f64], q: f64) -> Option<f64> {
    (!sorted.is_empty()).then(|| sorted[((sorted.len() - 1) as f64 * q).round() as usize])
}

/// False-positive targets for the ROC sweep: 1, 2 and 5 per decade from 0.01% to 50%,
/// and the evaluation's own target.
#[must_use]
pub fn roc_targets(fpr_target: f64) -> Vec<f64> {
    let mut targets = vec![fpr_target];
    for exponent in [1e-4, 1e-3, 1e-2, 1e-1] {
        for mantissa in [1.0, 2.0, 5.0] {
            targets.push(mantissa * exponent);
        }
    }
    targets.sort_by(f64::total_cmp);
    targets.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HumanSplit;
    use crate::split::by_participant;

    fn session(participant: &str, peak: [f64; 4]) -> Traced {
        Traced {
            source: "human",
            class: "human".into(),
            scenario: "flick".into(),
            amplitude_ppm: 5_000,
            participant: participant.into(),
            trace: Trace {
                path: vec![
                    PathPoint {
                        t_ms: 1_000,
                        shots: 3,
                        engagements: 3,
                        scores: peak.map(|p| p / 2.0),
                    },
                    PathPoint {
                        t_ms: 4_000,
                        shots: 9,
                        engagements: 8,
                        scores: peak,
                    },
                ],
                peak,
                shots: 9,
                angle_mismatches: 0,
                scenario: "flick".into(),
                duration_s: 60.0,
            },
        }
    }

    /// 1000 sessions with peaks 0..1000 on two statistics, split by participant.
    fn sets() -> (Calibration<Vec<Traced>>, Evaluation<Vec<Traced>>) {
        let all = (0..1000)
            .map(|i| {
                session(
                    &format!("p{i}"),
                    [f64::from(i), f64::from(999 - i), 5e9, 5e9],
                )
            })
            .collect();
        let split = HumanSplit {
            calibration_fraction: 0.5,
            salt: "t".into(),
        };
        by_participant(all, |s| &s.participant, &split)
    }

    #[test]
    fn thresholds_split_the_budget_and_bound_the_calibration_set() {
        let (calibration, evaluation) = sets();
        let cal = calibration.filter(|_| true);
        let n = cal.get().len();
        let t = calibrate(&cal, &[0, 1], 0.02).unwrap();
        assert_eq!((t.sessions, t.share), (n, 0.01));
        // Statistics 2 and 3 are not this scenario's: never flagged, whatever the score.
        assert_eq!((t.values[2], t.values[3]), (f64::INFINITY, f64::INFINITY));
        let inside = in_sample(&cal, &t);
        assert!(inside.flagged as f64 <= 0.02 * n as f64, "{inside:?}");
        assert!(inside.by_statistic[0] as f64 <= 0.01 * n as f64);
        assert_eq!((inside.by_statistic[2], inside.by_statistic[3]), (0, 0));
        // The held-out half is measured, not fitted: about 2%, give or take.
        let held_out = false_positives(&evaluation.filter(|_| true), &t);
        assert_eq!(held_out.sessions as usize, 1000 - n);
        assert!(
            held_out.flagged > 0 && held_out.rate().unwrap() < 0.06,
            "{held_out:?}"
        );
        // A looser target flags more.
        let loose = calibrate(&cal, &[0, 1], 0.2).unwrap();
        assert!(loose.values[0] < t.values[0] && loose.values[1] < t.values[1]);
    }

    #[test]
    fn few_sessions_give_the_highest_score_seen() {
        let (calibration, _) = sets();
        let few = calibration.filter(|s| s.trace.peak[0] < 40.0);
        let n = few.get().len();
        assert!(n > 2 && n < 40);
        let highest = few
            .get()
            .iter()
            .map(|s| s.trace.peak[0])
            .fold(0.0, f64::max);
        let t = calibrate(&few, &[0], 0.001).unwrap();
        assert_eq!(t.values[0], highest);
        assert_eq!(in_sample(&few, &t).flagged, 0);
        assert!(calibrate(&calibration.filter(|_| false), &[0], 0.001).is_none());
    }

    #[test]
    fn a_statistic_nobody_scored_on_stays_off() {
        let silent: Vec<Traced> = (0..50)
            .map(|i| session(&format!("p{i}"), [f64::from(i), 0.0, 0.0, 0.0]))
            .collect();
        let split = HumanSplit {
            calibration_fraction: 0.5,
            salt: "t".into(),
        };
        let (calibration, _) = by_participant(silent, |s| &s.participant, &split);
        let t = calibrate(&calibration.filter(|_| true), &[0, 1], 0.1).unwrap();
        assert!(t.values[0].is_finite());
        assert_eq!(t.values[1], f64::INFINITY);
        // Even a clear score on it does not flag, and the server's flag score for it is
        // the unreachable one.
        let loud = session("x", [0.0, 500.0, 0.0, 0.0]);
        assert_eq!(t.flags(&loud.trace), [false; 4]);
        assert_eq!(t.flag_scores(f64::MAX)[1], f64::MAX);
    }

    #[test]
    fn first_flag_and_flag_scores_agree_with_the_flag_rule() {
        let t = Thresholds {
            stats: vec![0, 1],
            values: [10.0, 3.0, f64::INFINITY, f64::INFINITY],
            sessions: 1,
            share: 0.5,
        };
        let quiet = session("a", [10.0, 3.0, 99.0, 99.0]);
        assert_eq!(
            t.flags(&quiet.trace),
            [false; 4],
            "at the threshold is not above it"
        );
        assert!(t.first_flag(&quiet.trace).is_none());
        let late = session("b", [12.0, 0.0, 0.0, 0.0]);
        assert_eq!(t.flags(&late.trace), [true, false, false, false]);
        assert_eq!(t.first_flag(&late.trace).unwrap().t_ms, 4_000);
        let early = session("c", [30.0, 0.0, 0.0, 0.0]);
        assert_eq!(t.first_flag(&early.trace).unwrap().t_ms, 1_000);
        // score >= flag score exactly when score > threshold.
        let flags = t.flag_scores(f64::MAX);
        assert!(flags[0] > 10.0 && 10.0_f64.next_up() >= flags[0]);
        assert_eq!((flags[2], flags[3]), (f64::MAX, f64::MAX));
        let c = detections(&[&quiet, &late, &early], &t);
        assert_eq!((c.sessions, c.flagged, c.by_statistic[0]), (3, 2, 2));
        assert_eq!(Count::default().rate(), None);
    }

    #[test]
    fn quantiles_and_roc_targets() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(quantile(&v, 0.5), Some(3.0));
        assert_eq!(quantile(&v, 0.0), Some(1.0));
        assert_eq!(quantile(&v, 1.0), Some(5.0));
        assert_eq!(quantile(&[], 0.5), None);
        let targets = roc_targets(0.001);
        assert_eq!(
            targets.len(),
            12,
            "the target is already on the grid: {targets:?}"
        );
        assert!(targets.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(roc_targets(0.003).len(), 13);
    }
}
