// SPDX-License-Identifier: MIT OR Apache-2.0
//! Task 1.8: ground-truth labels are session metadata for the evaluation harness only.
//! The detector never reads them.
//!
//! The proof: the server's verdict for a labelled session is exactly what a detector
//! computes offline from the same telemetry and seed, with no label anywhere in reach.
//! And labels that lie (a simulated human labelled as a cheat, an aimbot labelled as a
//! human) change nothing: the verdicts follow the behaviour.

mod common;

use common::*;
use rearguard_core::detect::Detector;
use rearguard_core::probe::{Amplitude, ProbeConfig};
use rearguard_core::protocol::{EvidenceSummary, SessionStatus, VerdictReport};
use rearguard_core::telemetry::Record;
use rearguard_server::client::Client;
use rearguard_sim::session::Class;
use rearguard_sim::world::Scenario;

/// Plays one simulated session with `label` and returns its verdict and records.
async fn play_labelled(
    server: &TestServer,
    label: Option<&str>,
    class: Class,
    sim_seed: u64,
) -> (Opened, VerdictReport, Vec<Record>) {
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello_labelled(&mut client, label).await;
    let chunks = telemetry(&opened, class, Scenario::Flick, 90.0, sim_seed, 500);
    send_chunks(&mut client, opened.session_id, &chunks, 0).await;
    let verdict = finish(&mut client, opened.session_id, chunks.len() as u64).await;
    let records = chunks.into_iter().flatten().map(Record::from).collect();
    (opened, verdict, records)
}

/// The verdict a detector gives the records offline. Its inputs are the seed, the probe
/// configuration, the detector configuration and the telemetry: there is no label.
fn offline_verdict(server: &TestServer, opened: &Opened, records: &[Record]) -> VerdictReport {
    let probe = ProbeConfig {
        amplitude: Amplitude::from_ppm(opened.amplitude_ppm).unwrap(),
        ..ProbeConfig::default()
    };
    let detector_config = config(&server.dir).detector;
    let detector_config = detector_config.for_scenario("flick").unwrap().clone();
    let mut detector = Detector::new(&opened.seed, &probe, detector_config).unwrap();
    detector.feed(records).unwrap();
    let s = detector.session();
    let windows = detector.windows();
    let (score, confidence) = rearguard_server::store::verdict_score(&s);
    VerdictReport {
        session_id: opened.session_id,
        status: SessionStatus::Finished,
        score,
        confidence,
        flagged: s.flagged(),
        records: records.len() as u64,
        shots: s.shots,
        engagements: s.engagements,
        angle_mismatches: s.angle_mismatches,
        error: EvidenceSummary::from(&s.error),
        steps: EvidenceSummary::from(&s.steps),
        windows: windows.len() as u64,
        flagged_windows: windows.iter().filter(|w| w.flagged()).count() as u64,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_detector_never_reads_the_ground_truth_label() {
    let mut server = TestServer::start("labels", |_| {}).await;
    let long = "x".repeat(200);
    let cases = [
        (Some("cheat:flick-aimbot"), Class::Human, 11),
        (Some("human"), Class::FlickAimbot, 12),
        (None, Class::FlickAimbot, 12),
        (Some(long.as_str()), Class::Human, 11),
    ];
    let mut played = Vec::new();
    for (label, class, sim_seed) in cases {
        played.push(play_labelled(&server, label, class, sim_seed).await);
    }

    for ((label, class, _), (opened, verdict, records)) in cases.iter().zip(&played) {
        // Bit for bit what the label-free offline detector says (Debug, so that NaN
        // and infinite fields compare too).
        let offline = offline_verdict(&server, opened, records);
        assert_eq!(
            format!("{verdict:?}"),
            format!("{offline:?}"),
            "label {label:?} on a {class:?} session"
        );
        // The verdict follows the behaviour, whatever the label claims.
        match class {
            Class::FlickAimbot => assert!(verdict.flagged, "{label:?}: {verdict:?}"),
            _ => assert!(!verdict.flagged, "{label:?}: {verdict:?}"),
        }
    }
    server.stop().await;

    // The labels are stored with their sessions (truncated to 64 characters), and only
    // the evaluation listing reads them.
    let listed = server.store().labelled_sessions().unwrap();
    let expected: Vec<(u64, String, SessionStatus)> = vec![
        (
            played[0].0.session_id,
            "cheat:flick-aimbot".into(),
            SessionStatus::Finished,
        ),
        (
            played[1].0.session_id,
            "human".into(),
            SessionStatus::Finished,
        ),
        (
            played[3].0.session_id,
            "x".repeat(64),
            SessionStatus::Finished,
        ),
    ];
    let mut listed_sorted = listed.clone();
    listed_sorted.sort_by_key(|(id, _, _)| *id);
    let mut expected_sorted = expected.clone();
    expected_sorted.sort_by_key(|(id, _, _)| *id);
    assert_eq!(listed_sorted, expected_sorted);

    // `rearguard-server labels` joins labels to verdicts for the evaluation harness.
    let bin = env!("CARGO_BIN_EXE_rearguard-server");
    let db = server.dir.join("evidence.sqlite3");
    let out = std::process::Command::new(bin)
        .args(["labels", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let lines: Vec<serde_json::Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    for (id, label, _) in &expected {
        let line = lines
            .iter()
            .find(|l| l["session_id"] == *id)
            .unwrap_or_else(|| panic!("session {id} missing: {lines:?}"));
        assert_eq!(line["label"], label.as_str());
        assert_eq!(line["status"], "finished");
        let (_, verdict, _) = played.iter().find(|(o, _, _)| o.session_id == *id).unwrap();
        assert_eq!(line["verdict"]["flagged"], verdict.flagged);
        assert_eq!(line["verdict"]["score"].as_f64(), Some(verdict.score));
    }
}
