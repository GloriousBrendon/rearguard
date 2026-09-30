//! Task 1.7, acceptance 5: a session recorded by the Godot aim range (rearguard.telemetry
//! v1, written through the extension) goes straight into the detector.
//!
//! Fixture: `demo` headless bot, spray, seed 4, 3 s, probe at 2% from
//! `fixtures/aimrange-seed.hex` (public test data, not a secret). Regenerate with:
//! `godot --headless --path demo --fixed-fps 120 -- --bot --scenario spray --seed 4
//! --duration 3 --probe-seed-file .../aimrange-seed.hex --probe-amplitude-ppm 20000
//! --out .../aimrange-spray.jsonl`.

use std::path::Path;

use rearguard_core::detect::{Detector, DetectorConfig};
use rearguard_core::probe::{Amplitude, EpochSeed, ProbeConfig};
use rearguard_core::telemetry::{self, Record};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn config() -> DetectorConfig {
    DetectorConfig {
        kappa_bound: 2.0,
        error_flag_score: 11.25,
        steps_flag_score: 4.25,
        min_pairs: 20,
        window_ms: 1_000,
        min_step_counts: 100.0,
    }
}

fn detect(seed: &EpochSeed, ppm: u32, records: &[Record]) -> rearguard_core::detect::Report {
    let probe = ProbeConfig {
        amplitude: Amplitude::from_ppm(ppm).unwrap(),
        ..ProbeConfig::default()
    };
    let mut d = Detector::new(seed, &probe, config()).unwrap();
    d.feed(records).unwrap();
    d.session()
}

#[test]
fn an_aim_range_recording_goes_straight_into_the_detector() {
    let records = telemetry::read_jsonl(fixture("aimrange-spray.jsonl").as_bytes()).unwrap();
    assert!(records.len() > 500);
    let Some(Record::Header(h)) = records.first() else {
        panic!("header")
    };
    assert_eq!(h.format, telemetry::FORMAT);
    assert_eq!(h.scenario, "spray");
    assert!(matches!(records.last(), Some(Record::End(e)) if e.complete));

    let seed = EpochSeed::from_hex(&fixture("aimrange-seed.hex")).unwrap();
    let report = detect(&seed, 20_000, &records);
    // The server-side replay reproduces every view angle the game computed (in
    // GDScript, through the extension's multipliers): Godot and the detector agree.
    assert!(report.shots > 20, "{report:?}");
    assert_eq!(report.angle_mismatches, 0, "{report:?}");
    assert!(report.error.pairs > 0, "{report:?}");
}

#[test]
fn the_wrong_seed_or_no_drift_does_not_reproduce_the_angles() {
    let records = telemetry::read_jsonl(fixture("aimrange-spray.jsonl").as_bytes()).unwrap();
    let wrong = EpochSeed::from_hex(&"ab".repeat(32)).unwrap();
    assert!(
        detect(&wrong, 20_000, &records).angle_mismatches > 0,
        "a wrong seed is noticed"
    );
    let seed = EpochSeed::from_hex(&fixture("aimrange-seed.hex")).unwrap();
    assert!(
        detect(&seed, 0, &records).angle_mismatches > 0,
        "missing drift is noticed"
    );
}
