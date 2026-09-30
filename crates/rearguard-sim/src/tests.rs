// SPDX-License-Identifier: MIT OR Apache-2.0

//! Model-level tests: drift correlation (the models behave as intended), determinism,
//! telemetry consistency and batch generation.

use std::path::PathBuf;

use rearguard_core::probe::{Amplitude, ProbeConfig, ProbeGenerator, Stream};
use rearguard_core::telemetry::{self, Record};

use super::*;
use crate::generate::{Options, generate};
use crate::seed::SimSeed;
use crate::session::{Class, ModelParams, Session, SessionSpec, run_session};
use crate::validation::{DriftCorrelation, Unit, drift_correlation, units};
use crate::world::Scenario;

#[test]
fn links_against_core() {
    assert_eq!(core_version(), rearguard_core::VERSION);
}

fn spec(class: Class, scenario: Scenario, index: u32, duration_s: f64) -> SessionSpec {
    SessionSpec {
        class,
        scenario,
        index,
        duration_s,
        amplitude: Amplitude::DEFAULT,
        models: ModelParams::DEFAULT,
    }
}

fn session(seed: u64, spec: SessionSpec) -> Session {
    let seed = SimSeed::new(seed);
    run_session(&seed, &seed.probe_root(), spec)
}

/// Simulates `sessions` one-minute sessions of `class` in `scenario` and measures how
/// its aim error follows the drift. Also returns (shots, hits).
fn measure(
    seed: u64,
    class: Class,
    scenario: Scenario,
    sessions: u32,
) -> (DriftCorrelation, usize, usize) {
    measure_with(seed, class, scenario, sessions, ModelParams::DEFAULT)
}

/// [`measure`] with non-default cheat-model tuning.
fn measure_with(
    seed: u64,
    class: Class,
    scenario: Scenario,
    sessions: u32,
    models: ModelParams,
) -> (DriftCorrelation, usize, usize) {
    let sim_seed = SimSeed::new(seed);
    let root = sim_seed.probe_root();
    let mut all: Vec<Unit> = Vec::new();
    let (mut shots, mut hits) = (0, 0);
    for index in 0..sessions {
        let mut spec = spec(class, scenario, index, 60.0);
        spec.models = models;
        let session = run_session(&sim_seed, &root, spec);
        shots += session.truth.len();
        hits += session.truth.iter().filter(|t| t.hit).count();
        all.extend(units(scenario, &session.truth));
    }
    let mut rng = sim_seed.rng(0xC0FFEE);
    (drift_correlation(&all, &mut rng, 400), shots, hits)
}

// ---------------------------------------------------------------------------
// Drift correlation (acceptance criterion 2)
//
// Statistic: Pearson's r between aim error and the drift's effect on it, pooled over
// shots (see `validation`). Its null standard deviation comes from 400 permutations
// that re-pair each unit (flick target or spray burst) with another unit's drift, so
// dependence within a burst is accounted for; z = r / null sd.
//
// - Closed-loop human: consistent with the null, |z| < 3 (two-sided, p ~ 0.003).
// - Open-loop classes: z > 4 and slope > 0.2, i.e. clearly beyond the null and
//   carrying a substantial share of the drift into the error.
//
// Volume, by scenario: 10 one-minute sessions for every flick test (about 850 human
// or humanised shots, and thousands for the fast aimbots) and 30 for every spray test
// (about 14,700 shots, 450 bursts; a burst carries less drift than a flick).
//
// The human model is not perfectly closed-loop: it fires once its estimated error is
// within tolerance, not after correcting fully, so a small share of the drift reaches
// its flick error. Measured over 300 sessions (`print_human_leak`): slope 0.26 ± 0.03,
// r 0.037, z 8.4 at 25,692 shots. At the 10-session volume the expected z is about
// 1.5; the leak becomes significant (z ~ 3) from roughly 3,000 flick shots. So the null
// claim holds at this volume, not at any volume, and a detector has to judge the
// strength of the correlation (slope ~0.26, r ~0.04 here, against 0.3 to 1.0 for the
// open-loop classes), not merely its presence. See README.md.
// ---------------------------------------------------------------------------

fn assert_closed_loop(class: Class, scenario: Scenario, sessions: u32) {
    let (c, shots, _) = measure(1, class, scenario, sessions);
    assert!(shots > 500, "{class:?} {scenario:?}: only {shots} shots");
    assert!(c.z.abs() < 3.0, "{class:?} {scenario:?}: {c:?}");
}

fn assert_open_loop(class: Class, scenario: Scenario, sessions: u32) {
    let (c, shots, _) = measure(1, class, scenario, sessions);
    assert!(shots > 500, "{class:?} {scenario:?}: only {shots} shots");
    assert!(c.z > 4.0 && c.slope > 0.2, "{class:?} {scenario:?}: {c:?}");
}

#[test]
fn human_flick_error_is_not_correlated_with_drift() {
    assert_closed_loop(Class::Human, Scenario::Flick, 10);
}

#[test]
fn human_spray_error_is_not_correlated_with_drift() {
    assert_closed_loop(Class::Human, Scenario::Spray, 30);
}

#[test]
fn recoil_macro_error_follows_drift() {
    assert_open_loop(Class::RecoilMacro, Scenario::Spray, 30);
}

#[test]
fn flick_aimbot_error_follows_drift() {
    assert_open_loop(Class::FlickAimbot, Scenario::Flick, 10);
}

#[test]
fn humanised_aimbot_error_follows_drift() {
    assert_open_loop(Class::HumanisedAimbot, Scenario::Flick, 10);
}

#[test]
fn adaptive_aimbot_error_follows_drift() {
    assert_open_loop(Class::AdaptiveAimbot, Scenario::Flick, 10);
}

// Task 1.2a models. The smoothing aimbot corrects from the view it actually sees,
// so it is held to the closed-loop standard (|z| < 3 at 30 sessions, the same volume
// as the spray tests; its 300-session slope is -0.07, reaching |z| = 4 only after
// about 64,000 shots). The fast adaptive aimbot cancels most of the drift (slope about
// 0.07 with the default 250 ms window) but its error has so little other noise that it
// stays visible: z > 4 at 10 sessions, with a slope below the 0.2 open-loop bar.

#[test]
fn smoothing_aimbot_error_is_not_correlated_with_drift() {
    assert_closed_loop(Class::SmoothingAimbot, Scenario::Flick, 30);
}

#[test]
fn fast_adaptive_aimbot_cancels_most_drift_but_stays_visible() {
    let (c, shots, _) = measure(1, Class::FastAdaptiveAimbot, Scenario::Flick, 10);
    assert!(shots > 500, "{shots} shots");
    assert!(c.z > 4.0 && c.slope > 0.0 && c.slope < 0.2, "{c:?}");
}

#[test]
fn fast_adaptive_window_sets_how_much_drift_survives() {
    let slope = |estimation_window_ms| {
        let models = ModelParams {
            estimation_window_ms,
            ..ModelParams::DEFAULT
        };
        measure_with(1, Class::FastAdaptiveAimbot, Scenario::Flick, 5, models)
            .0
            .slope
    };
    // A long window averages stale measurements, so more of the drift survives.
    let (short, long) = (slope(100), slope(20_000));
    assert!(short < 0.1 && long > 0.8, "short {short}, long {long}");
}

#[test]
fn smoothing_factor_sets_convergence_speed() {
    // More smoothing per frame reaches each target sooner, so more targets are hit in
    // the same time (until the weapon's fire rate is the limit).
    let shots_in_20s = |smoothing| {
        let mut spec = spec(Class::SmoothingAimbot, Scenario::Flick, 0, 20.0);
        spec.models = ModelParams {
            smoothing,
            ..ModelParams::DEFAULT
        };
        session(4, spec).truth.len()
    };
    let (slow, fast) = (shots_in_20s(0.05), shots_in_20s(0.4));
    assert!(
        fast > slow * 2,
        "0.05: {slow} shots, 0.4: {fast} shots in 20 s"
    );
}

#[test]
fn without_drift_there_is_nothing_to_correlate() {
    let seed = SimSeed::new(3);
    let root = seed.probe_root();
    let mut s = spec(Class::FlickAimbot, Scenario::Flick, 0, 10.0);
    s.amplitude = Amplitude::from_ppm(0).unwrap();
    let session = run_session(&seed, &root, s);
    assert!(!session.truth.is_empty());
    assert!(session.truth.iter().all(|t| t.drift_offset == [0.0, 0.0]));
}

#[test]
#[ignore = "prints model statistics at 10, 30 and 60 sessions; run with --release"]
fn print_model_report() {
    for sessions in [10, 30, 60] {
        println!("{sessions} sessions x 60 s, amplitude 5000 ppm");
        for class in Class::ALL {
            for &scenario in class.scenarios() {
                let (c, shots, hits) = measure(1, class, scenario, sessions);
                println!(
                    "  {:<20} {:<6} shots {shots:>6}  hit {:>5.1}%  r {:>+.3}  null sd {:.3}  z {:>+7.1}  slope {:>+.3}",
                    class.name(),
                    scenario.name(),
                    100.0 * hits as f64 / shots.max(1) as f64,
                    c.r,
                    c.null_sd,
                    c.z,
                    c.slope
                );
            }
        }
    }
}

#[test]
#[ignore = "measures the human model's small drift leak over 300 sessions; run with --release"]
fn print_human_leak() {
    for (class, scenario) in [
        (Class::Human, Scenario::Flick),
        (Class::Human, Scenario::Spray),
        (Class::RecoilMacro, Scenario::Spray),
    ] {
        let (c, shots, _) = measure(2, class, scenario, 300);
        // Slope standard error from r and the pair count (ignores within-unit
        // dependence, so it is a lower bound for spray).
        let se = c.slope.abs() / c.z.abs().max(1e-9);
        println!(
            "{:<13} {:<6} shots {shots:>7}  r {:>+.4}  z {:>+6.1}  slope {:>+.3} (se ~{:.3})",
            class.name(),
            scenario.name(),
            c.r,
            c.z,
            c.slope,
            se
        );
    }
}

/// Shots needed for z = 4, extrapolated from a measurement (z grows with the square
/// root of the number of shots); `None` when the measurement is too weak to say.
fn shots_for_z4(c: &DriftCorrelation, shots: usize) -> Option<f64> {
    (c.z.abs() >= 2.0).then(|| shots as f64 * (4.0 / c.z) * (4.0 / c.z))
}

fn print_row(label: &str, c: &DriftCorrelation, shots: usize) {
    let volume =
        shots_for_z4(c, shots).map_or_else(|| "      n/a".to_owned(), |n| format!("{n:>9.0}"));
    println!(
        "  {label:<34} shots {shots:>7}  r {:>+.4}  z {:>+7.1}  slope {:>+.3}  shots for z=4 {volume}",
        c.r, c.z, c.slope
    );
}

#[test]
#[ignore = "sweeps the task 1.2a closed-loop and fast adaptive aimbots; run with --release"]
fn print_closed_loop_report() {
    println!("smoothing-aimbot, 30 sessions x 60 s, by smoothing factor");
    for smoothing in [0.05, 0.1, 0.2, 0.4, 0.7, 1.0] {
        let models = ModelParams {
            smoothing,
            ..ModelParams::DEFAULT
        };
        let (c, shots, _) = measure_with(1, Class::SmoothingAimbot, Scenario::Flick, 30, models);
        print_row(&format!("smoothing {smoothing}"), &c, shots);
    }
    println!("fast-adaptive-aimbot, 30 sessions x 60 s, by estimation window");
    for window in [10, 50, 125, 250, 1_000, 5_000, 20_000] {
        let models = ModelParams {
            estimation_window_ms: window,
            ..ModelParams::DEFAULT
        };
        let (c, shots, _) = measure_with(1, Class::FastAdaptiveAimbot, Scenario::Flick, 30, models);
        print_row(&format!("window {window} ms"), &c, shots);
    }
    println!("defaults, 300 sessions x 60 s (seed 2)");
    for class in [
        Class::SmoothingAimbot,
        Class::FastAdaptiveAimbot,
        Class::Human,
    ] {
        let (c, shots, _) = measure(2, class, Scenario::Flick, 300);
        print_row(class.name(), &c, shots);
    }
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

fn jsonl(records: &[Record]) -> Vec<u8> {
    let mut bytes = Vec::new();
    telemetry::write_jsonl(&mut bytes, records).unwrap();
    bytes
}

/// FNV-1a, 64-bit: a dependency-free fingerprint for golden values.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn all_pairs() -> Vec<(Class, Scenario)> {
    Class::ALL
        .into_iter()
        .flat_map(|c| c.scenarios().iter().map(move |s| (c, *s)))
        .collect()
}

#[test]
fn sessions_are_reproducible_and_seed_dependent() {
    for (class, scenario) in all_pairs() {
        let a = session(5, spec(class, scenario, 2, 5.0));
        let b = session(5, spec(class, scenario, 2, 5.0));
        assert_eq!(a.records, b.records, "{class:?}");
        assert_eq!(a.truth, b.truth, "{class:?}");
        let c = session(6, spec(class, scenario, 2, 5.0));
        let d = session(5, spec(class, scenario, 3, 5.0));
        assert_ne!(
            jsonl(&a.records),
            jsonl(&c.records),
            "{class:?}: other seed"
        );
        assert_ne!(
            jsonl(&a.records),
            jsonl(&d.records),
            "{class:?}: other index"
        );
    }
}

/// FNV-1a of each class's 5-second session (seed 1, index 0), as JSON Lines. They pin
/// the whole simulation (models, RNG, libm, probe, serialisation) across platforms: CI
/// checks them on Linux and Windows. Regenerate with `cargo test -p rearguard-sim
/// print_golden_hashes -- --ignored --nocapture` after an intentional model change.
///
/// History: task 1.2a added the last two rows (the new smoothing and fast adaptive
/// aimbots). The first six are unchanged, because new classes are appended to
/// `Class::ALL` and so leave every existing class's random streams alone.
const GOLDEN_HASHES: &[(&str, &str, u64)] = &[
    ("human", "flick", 0x5679461e8873c0ee),
    ("human", "spray", 0x0a4b0e0516da980d),
    ("recoil-macro", "spray", 0x9be8c9fe804bf3a7),
    ("flick-aimbot", "flick", 0x287fbe41d1506a00),
    ("humanised-aimbot", "flick", 0x07ff09b885ae3dd5),
    ("adaptive-aimbot", "flick", 0xf47113f33c207518),
    ("smoothing-aimbot", "flick", 0x9bd028aa121b62a9),
    ("fast-adaptive-aimbot", "flick", 0xa6bdaa5be1abe5a0),
];

fn golden_rows() -> Vec<(&'static str, &'static str, u64)> {
    all_pairs()
        .into_iter()
        .map(|(class, scenario)| {
            let s = session(1, spec(class, scenario, 0, 5.0));
            (class.name(), scenario.name(), fnv1a(&jsonl(&s.records)))
        })
        .collect()
}

#[test]
#[ignore = "prints the golden hash table"]
fn print_golden_hashes() {
    println!("const GOLDEN_HASHES: &[(&str, &str, u64)] = &[");
    for (class, scenario, hash) in golden_rows() {
        println!("    (\"{class}\", \"{scenario}\", {hash:#018x}),");
    }
    println!("];");
}

#[test]
fn golden_hashes_match() {
    assert_eq!(golden_rows(), GOLDEN_HASHES);
}

// ---------------------------------------------------------------------------
// Telemetry consistency: the stream is sufficient to reproduce every view angle
// ---------------------------------------------------------------------------

/// Replays a session from its telemetry and the probe seed, the way a server would
/// verify it (decision D2), and checks every recorded angle exactly.
fn replay(seed: &SimSeed, records: &[Record]) {
    let Some(Record::Header(h)) = records.first() else {
        panic!("no header")
    };
    let epoch = seed
        .probe_root()
        .match_key(h.match_id.as_bytes())
        .player_key(h.player_id)
        .epoch_seed(0);
    let mut probe = ProbeGenerator::new(&epoch, &ProbeConfig::default()).unwrap();
    let mut multiplier = |stream, ts_us: u64| {
        probe
            .drift(stream, (ts_us - h.probe_start_us) / 1_000)
            .multiplier()
    };
    let (mut yaw, mut pitch) = (0.0f64, 0.0f64);
    let mut moves = 0;
    for record in records {
        match record {
            Record::Move(m) => {
                let k = multiplier(Stream::Sensitivity, m.ts_us);
                yaw += -m.dx * h.deg_per_count * k;
                pitch = (pitch + -m.dy * h.deg_per_count * k).clamp(-89.0, 89.0);
                assert_eq!(
                    (yaw.to_bits(), pitch.to_bits()),
                    (m.yaw.to_bits(), m.pitch.to_bits())
                );
                moves += 1;
            }
            Record::Recoil(r) => {
                let k = multiplier(Stream::Recoil, r.ts_us);
                yaw += r.kick_yaw * k;
                pitch = (pitch + r.kick_pitch * k).clamp(-89.0, 89.0);
                assert_eq!(
                    (yaw.to_bits(), pitch.to_bits()),
                    (r.yaw.to_bits(), r.pitch.to_bits())
                );
            }
            Record::Fire(f) => {
                assert_eq!(
                    (yaw.to_bits(), pitch.to_bits()),
                    (f.yaw.to_bits(), f.pitch.to_bits())
                );
            }
            _ => {}
        }
    }
    assert!(moves > 0);
}

#[test]
fn telemetry_replays_exactly_from_deltas_and_seed() {
    for (class, scenario) in all_pairs() {
        let seed = SimSeed::new(11);
        let s = run_session(&seed, &seed.probe_root(), spec(class, scenario, 0, 8.0));
        // Through the file format, as the server would receive it.
        let records = telemetry::read_jsonl(jsonl(&s.records).as_slice()).unwrap();
        assert_eq!(records, s.records);
        replay(&seed, &records);
    }
}

#[test]
fn telemetry_never_names_the_class() {
    for (class, scenario) in all_pairs() {
        let s = session(1, spec(class, scenario, 0, 2.0));
        let text = String::from_utf8(jsonl(&s.records)).unwrap();
        for c in Class::ALL {
            assert!(
                !text.contains(c.name()),
                "{class:?} telemetry mentions {}",
                c.name()
            );
        }
        assert!(text.contains(r#""source":"sim""#));
    }
}

#[test]
fn models_play_plausibly() {
    for (class, scenario) in all_pairs() {
        let s = session(2, spec(class, scenario, 0, 30.0));
        let shots = s.truth.len();
        let hits = s.truth.iter().filter(|t| t.hit).count();
        assert!(shots >= 20, "{class:?} {scenario:?}: {shots} shots");
        assert!(
            hits * 10 >= shots * 8,
            "{class:?} {scenario:?}: {hits}/{shots} hits"
        );
    }
}

// ---------------------------------------------------------------------------
// Batch generation (acceptance criterion 1)
// ---------------------------------------------------------------------------

/// A fresh, empty directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("rearguard-sim-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every file under `dir`, as (relative path with `/`, bytes), sorted.
fn tree(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn generate_writes_n_sessions_per_class_reproducibly() {
    let options = Options {
        sessions: 2,
        duration_s: 3.0,
        amplitude: Amplitude::DEFAULT,
        classes: Class::ALL.to_vec(),
        models: ModelParams::DEFAULT,
    };
    let (a, b, c) = (TempDir::new("a"), TempDir::new("b"), TempDir::new("c"));
    let manifest = generate(&SimSeed::new(9), &options, &a.0).unwrap();
    generate(&SimSeed::new(9), &options, &b.0).unwrap();
    generate(&SimSeed::new(10), &options, &c.0).unwrap();

    // Two sessions for every class and scenario it plays.
    let expected: usize = Class::ALL.iter().map(|c| 2 * c.scenarios().len()).sum();
    assert_eq!(manifest.sessions.len(), expected);
    let files = tree(&a.0);
    assert_eq!(files.len(), expected + 1, "sessions plus manifest");
    for class in Class::ALL {
        let n = files
            .iter()
            .filter(|(p, _)| p.starts_with(&format!("{}/", class.name())))
            .count();
        assert_eq!(n, 2 * class.scenarios().len(), "{class:?}");
    }

    // Same seed: byte-identical. Other seed: different.
    assert_eq!(files, tree(&b.0));
    assert_ne!(files, tree(&c.0));

    // Every file is valid telemetry; the manifest holds no seed.
    for entry in &manifest.sessions {
        let text = std::fs::read(a.0.join(&entry.file)).unwrap();
        let records = telemetry::read_jsonl(text.as_slice()).unwrap();
        assert!(matches!(records.last(), Some(Record::End(_))));
    }
    let manifest_text = std::fs::read_to_string(a.0.join("manifest.json")).unwrap();
    assert!(!manifest_text.contains("seed\""), "{manifest_text}");

    // A batch is never overwritten.
    assert!(generate(&SimSeed::new(9), &options, &a.0).is_err());
}
