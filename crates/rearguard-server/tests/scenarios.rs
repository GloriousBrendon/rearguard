// SPDX-License-Identifier: MIT OR Apache-2.0

//! Task 1.12: one detector threshold set per scenario. The telemetry header's scenario
//! picks the set; a scenario without a set is refused.

mod common;

use std::collections::BTreeMap;

use common::*;
use rearguard_core::detect::{DetectorConfig, DetectorSet};
use rearguard_core::protocol::{ClientMessage, ErrorCode, ServerMessage, WireRecord};
use rearguard_server::client::Client;
use rearguard_server::config::Config;
use rearguard_sim::session::Class;
use rearguard_sim::world::Scenario;

/// Flag scores no session reaches.
const NEVER: f64 = 1e12;

/// The example thresholds for `flagging`, and unreachable ones for the other scenario.
fn only(flagging: &str) -> impl FnOnce(&mut Config) {
    let flagging = flagging.to_owned();
    move |config: &mut Config| {
        let usual = config.detector.for_scenario("flick").unwrap().clone();
        let never = DetectorConfig {
            error_flag_score: NEVER,
            steps_flag_score: NEVER,
            ..usual.clone()
        };
        let sets = ["flick", "spray"]
            .into_iter()
            .map(|s| {
                let set = if s == flagging { &usual } else { &never };
                (s.to_owned(), set.clone())
            })
            .collect::<BTreeMap<_, _>>();
        config.detector = DetectorSet::PerScenario(sets);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scenario_in_the_header_picks_the_threshold_set() {
    // The same kind of session, a flick aimbot, under two configurations that differ
    // only in which scenario's thresholds can be reached.
    let mut flick = TestServer::start("scenarios-flick", only("flick")).await;
    let (_, verdict) = play(flick.addr, Class::FlickAimbot, Scenario::Flick, 60.0, 1).await;
    assert!(verdict.flagged && verdict.error.flagged, "{verdict:?}");
    assert!(verdict.score > 100.0, "{verdict:?}");
    flick.stop().await;

    let mut spray = TestServer::start("scenarios-spray", only("spray")).await;
    let (_, verdict) = play(spray.addr, Class::FlickAimbot, Scenario::Flick, 60.0, 1).await;
    assert!(
        !verdict.flagged,
        "flick thresholds are out of reach: {verdict:?}"
    );
    assert!(
        verdict.score > 100.0,
        "the evidence is the same: {verdict:?}"
    );
    // A spray session on that server meets the reachable spray thresholds.
    let (_, verdict) = play(spray.addr, Class::RecoilMacro, Scenario::Spray, 60.0, 2).await;
    assert_eq!(verdict.angle_mismatches, 0, "{verdict:?}");
    assert!(verdict.error.pairs > 0, "{verdict:?}");
    spray.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_has_no_evidence_until_its_header_arrives() {
    let mut server = TestServer::start("scenarios-waiting", only("flick")).await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    let live = client
        .request(&ClientMessage::Verdict {
            session_id: opened.session_id,
        })
        .await
        .unwrap();
    let ServerMessage::Verdict(v) = live else {
        panic!("{live:?}")
    };
    assert!(!v.flagged && v.score == 0.0 && v.records == 0, "{v:?}");
    assert_eq!((v.shots, v.windows, v.error.pairs), (0, 0, 0), "{v:?}");

    // Telemetry that does not start with a header is refused, and uses up its number.
    let chunks = telemetry(&opened, Class::FlickAimbot, Scenario::Flick, 20.0, 3, 400);
    let reply = client
        .request(&ClientMessage::Telemetry {
            session_id: opened.session_id,
            seq: 0,
            records: chunks[1].clone(),
        })
        .await
        .unwrap();
    assert!(
        matches!(
            reply,
            ServerMessage::Error {
                code: ErrorCode::InvalidTelemetry
            }
        ),
        "{reply:?}"
    );

    // So is a scenario nobody configured thresholds for.
    let mut unknown = chunks[0].clone();
    let WireRecord::Header(header) = &mut unknown[0] else {
        panic!("sessions start with a header")
    };
    header.scenario = "tracking".into();
    let reply = client
        .request(&ClientMessage::Telemetry {
            session_id: opened.session_id,
            seq: 1,
            records: unknown,
        })
        .await
        .unwrap();
    assert!(
        matches!(
            reply,
            ServerMessage::Error {
                code: ErrorCode::InvalidTelemetry
            }
        ),
        "{reply:?}"
    );
    assert!(server.log_text().contains("no detector configuration"));

    // The real session then goes through, from the next chunk number.
    for (i, records) in chunks.iter().enumerate() {
        let reply = client
            .request(&ClientMessage::Telemetry {
                session_id: opened.session_id,
                seq: i as u64 + 2,
                records: records.clone(),
            })
            .await
            .unwrap();
        assert!(matches!(reply, ServerMessage::Ack { .. }), "{reply:?}");
    }
    let verdict = finish(&mut client, opened.session_id, chunks.len() as u64 + 2).await;
    assert!(verdict.flagged && verdict.shots > 50, "{verdict:?}");

    // A session abandoned before any telemetry is stored with no evidence.
    let mut idle = Client::connect(server.addr).await.unwrap();
    let never_sent = hello(&mut idle).await;
    drop(idle);
    server.stop().await;
    let stored = server
        .store()
        .verdict(never_sent.session_id)
        .unwrap()
        .unwrap();
    assert!(!stored.flagged && stored.records == 0, "{stored:?}");
}
