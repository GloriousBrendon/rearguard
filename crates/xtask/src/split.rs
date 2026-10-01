// SPDX-License-Identifier: MIT OR Apache-2.0

//! The split between sessions that set thresholds and sessions that measure them.
//!
//! A session is in exactly one of two sets, and the set is part of its type:
//!
//! - [`Calibration`]: thresholds are calibrated on these. `analysis::calibrate` takes
//!   nothing else.
//! - [`Evaluation`]: false-positive rates are measured on these.
//!   `analysis::false_positives` takes nothing else.
//!
//! Both wrappers can only be made here, by [`by_participant`] (people) and
//! [`sim_humans`] (simulated humans), which hand every participant or session index to
//! one set only. So a human session cannot be used for tuning and for evaluation in the
//! same run: there is no value of both types to pass. [`check_disjoint`] checks the
//! result again at run time.

use std::collections::BTreeSet;
use std::ops::Range;

use sha2::{Digest, Sha256};

use crate::config::HumanSplit;

/// Sessions thresholds are calibrated on. Never evaluated.
#[derive(Debug)]
pub struct Calibration<T>(T);

/// Sessions rates are measured on. Never used for calibration.
#[derive(Debug)]
pub struct Evaluation<T>(T);

macro_rules! set {
    ($name:ident) => {
        impl<T> $name<T> {
            /// The sessions, for reading.
            #[must_use]
            pub fn get(&self) -> &T {
                &self.0
            }

            /// Transforms the sessions; they stay in the same set.
            #[must_use]
            pub fn map<U>(self, f: impl FnOnce(T) -> U) -> $name<U> {
                $name(f(self.0))
            }
        }

        impl<T> $name<Vec<T>> {
            /// The sessions `keep` accepts, still in the same set.
            #[must_use]
            pub fn filter(&self, keep: impl Fn(&T) -> bool) -> $name<Vec<&T>> {
                $name(self.0.iter().filter(|t| keep(t)).collect())
            }
        }
    };
}
set!(Calibration);
set!(Evaluation);

/// Whether `participant` belongs to the calibration set: a hash of the salt and the
/// id, read as a fraction, falls below `calibration_fraction`. It depends on nothing
/// else, so adding or removing other participants never moves anyone.
#[must_use]
pub fn in_calibration(split: &HumanSplit, participant: &str) -> bool {
    let mut hash = Sha256::new();
    hash.update(b"rearguard-eval-split/v1\0");
    hash.update(split.salt.as_bytes());
    hash.update([0]);
    hash.update(participant.as_bytes());
    let digest = hash.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    // 53 bits, so the fraction is exact in an f64.
    let fraction = (u64::from_be_bytes(first) >> 11) as f64 / (1u64 << 53) as f64;
    fraction < split.calibration_fraction
}

/// Splits sessions by participant: all of a participant's sessions go to one set.
#[must_use]
pub fn by_participant<T>(
    sessions: Vec<T>,
    participant: impl Fn(&T) -> &str,
    split: &HumanSplit,
) -> (Calibration<Vec<T>>, Evaluation<Vec<T>>) {
    let (calibration, evaluation) = sessions
        .into_iter()
        .partition(|s| in_calibration(split, participant(s)));
    (Calibration(calibration), Evaluation(evaluation))
}

/// Session indices of the simulated humans: the first `calibration` are calibrated on,
/// the next `held_out` are measured on. An index numbers a session's random streams, so
/// different indices are different simulated people.
#[must_use]
pub fn sim_humans(
    calibration: u32,
    held_out: u32,
) -> (Calibration<Range<u32>>, Evaluation<Range<u32>>) {
    (
        Calibration(0..calibration),
        Evaluation(calibration..calibration + held_out),
    )
}

/// Checks that no participant has sessions in both sets.
///
/// # Errors
/// How many participants are in both.
pub fn check_disjoint<T>(
    calibration: &Calibration<Vec<T>>,
    evaluation: &Evaluation<Vec<T>>,
    participant: impl Fn(&T) -> &str,
) -> Result<(), String> {
    let tuned: BTreeSet<&str> = calibration.0.iter().map(&participant).collect();
    let both = evaluation
        .0
        .iter()
        .map(&participant)
        .filter(|p| tuned.contains(p))
        .collect::<BTreeSet<_>>()
        .len();
    if both == 0 {
        Ok(())
    } else {
        Err(format!(
            "{both} participants have sessions in both the calibration and the evaluation set"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(fraction: f64, salt: &str) -> HumanSplit {
        HumanSplit {
            calibration_fraction: fraction,
            salt: salt.to_owned(),
        }
    }

    /// (participant, session) pairs: 200 participants with 3 sessions each.
    fn sessions() -> Vec<(String, u32)> {
        (0..200)
            .flat_map(|p| (0..3).map(move |s| (format!("study/P-{p:04}"), s)))
            .collect()
    }

    #[test]
    fn every_participant_is_in_exactly_one_set() {
        let all = sessions();
        let (calibration, evaluation) = by_participant(all.clone(), |s| &s.0, &split(0.5, "a"));
        assert_eq!(calibration.get().len() + evaluation.get().len(), all.len());
        check_disjoint(&calibration, &evaluation, |s| &s.0).unwrap();
        // All three sessions of a participant are together.
        for set in [calibration.get(), evaluation.get()] {
            assert_eq!(set.len() % 3, 0);
            for chunk in set.chunks(3) {
                assert!(chunk.iter().all(|s| s.0 == chunk[0].0));
            }
        }
        // Roughly the requested share.
        let share = calibration.get().len() as f64 / all.len() as f64;
        assert!((0.4..0.6).contains(&share), "{share}");
    }

    #[test]
    fn the_split_is_stable_and_depends_only_on_salt_and_id() {
        let s = split(0.5, "a");
        let all = sessions();
        let (first, _) = by_participant(all.clone(), |s| &s.0, &s);
        // Again, in another order, with other participants added.
        let mut more = all.clone();
        more.reverse();
        more.extend((200..260).map(|p| (format!("study/P-{p:04}"), 0)));
        let (second, _) = by_participant(more, |s| &s.0, &s);
        let ids = |set: &Calibration<Vec<(String, u32)>>| -> BTreeSet<String> {
            set.get().iter().map(|s| s.0.clone()).collect()
        };
        let original: BTreeSet<String> = all.iter().map(|s| s.0.clone()).collect();
        let second_original: BTreeSet<String> =
            ids(&second).intersection(&original).cloned().collect();
        assert_eq!(ids(&first), second_original);
        // Another salt is another split.
        let (other, _) = by_participant(all, |s| &s.0, &split(0.5, "b"));
        assert_ne!(ids(&first), ids(&other));
        // Pinned, so the assignment cannot change between releases unnoticed.
        let pinned: Vec<bool> = [
            "study/P-0000",
            "study/P-0001",
            "study/P-0002",
            "study/P-0003",
        ]
        .iter()
        .map(|p| in_calibration(&s, p))
        .collect();
        assert_eq!(pinned, PINNED, "print: {pinned:?}");
    }

    const PINNED: [bool; 4] = [false, true, false, true];

    #[test]
    fn the_fraction_moves_the_share() {
        let all = sessions();
        let count = |f: f64| {
            by_participant(all.clone(), |s| &s.0, &split(f, "a"))
                .0
                .get()
                .len()
        };
        assert!(count(0.1) < count(0.5) && count(0.5) < count(0.9));
    }

    #[test]
    fn simulated_humans_use_separate_indices() {
        let (calibration, held_out) = sim_humans(5, 3);
        assert_eq!(*calibration.get(), 0..5);
        assert_eq!(*held_out.get(), 5..8);
        let kept = held_out.map(|r| r.collect::<Vec<_>>());
        assert_eq!(kept.filter(|i| *i > 5).get().len(), 2);
    }

    #[test]
    fn overlap_is_caught() {
        // Only this module can build the sets, so only it can build a bad pair.
        let calibration = Calibration(vec!["p1", "p2"]);
        let evaluation = Evaluation(vec!["p2", "p3"]);
        assert!(check_disjoint(&calibration, &evaluation, |s| s).is_err());
        let evaluation = Evaluation(vec!["p3"]);
        assert!(check_disjoint(&calibration, &evaluation, |s| s).is_ok());
    }
}
