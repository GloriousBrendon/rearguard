// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance criteria 1 and 5: a simulated client (rearguard-sim) through the server to
//! a verdict with a score and an evidence summary.

mod common;

use common::*;
use rearguard_core::probe::RootSeed;
use rearguard_core::protocol::{ClientMessage, ServerMessage, SessionStatus};
use rearguard_server::client::Client;
use rearguard_sim::session::Class;
use rearguard_sim::world::Scenario;

#[tokio::test(flavor = "multi_thread")]
async fn simulated_clients_get_verdicts_and_evidence_is_stored() {
    let mut server = TestServer::start("e2e", |_| {}).await;

    // Two minutes of play each, under server-issued seeds.
    let (aimbot, aimbot_verdict) =
        play(server.addr, Class::FlickAimbot, Scenario::Flick, 120.0, 1).await;
    let (smoothing, smoothing_verdict) = play(
        server.addr,
        Class::SmoothingAimbot,
        Scenario::Flick,
        120.0,
        2,
    )
    .await;
    let (human, human_verdict) = play(server.addr, Class::Human, Scenario::Flick, 120.0, 3).await;

    // The open-loop aimbot is caught by the fire-time statistic, the smoothing aimbot by
    // the step response, and the simulated human is not flagged.
    assert!(
        aimbot_verdict.flagged && aimbot_verdict.error.flagged,
        "{aimbot_verdict:?}"
    );
    assert!(aimbot_verdict.score > 100.0, "{aimbot_verdict:?}");
    assert!(
        smoothing_verdict.flagged && smoothing_verdict.steps.flagged,
        "{smoothing_verdict:?}"
    );
    assert!(!human_verdict.flagged, "{human_verdict:?}");
    assert!(human_verdict.score < 11.25, "{human_verdict:?}");

    // Evidence summaries are populated.
    for v in [&aimbot_verdict, &smoothing_verdict, &human_verdict] {
        assert_eq!(v.status, SessionStatus::Finished);
        assert!(
            v.records > 1_000 && v.shots > 50 && v.engagements > 50,
            "{v:?}"
        );
        assert!(v.error.pairs > 100, "{v:?}");
        assert_eq!(v.windows, 4, "two minutes in 30 s windows: {v:?}");
        assert_eq!(v.angle_mismatches, 0, "the sim reports honest angles");
    }
    assert!(
        (aimbot_verdict.error.slope - 1.0).abs() < 0.05,
        "open-loop slope ~1: {aimbot_verdict:?}"
    );

    // The verdict API gives the same verdicts later, on a new connection.
    let mut client = Client::connect(server.addr).await.unwrap();
    for (opened, verdict) in [
        (&aimbot, &aimbot_verdict),
        (&smoothing, &smoothing_verdict),
        (&human, &human_verdict),
    ] {
        match client
            .request(&ClientMessage::Verdict {
                session_id: opened.session_id,
            })
            .await
            .unwrap()
        {
            ServerMessage::Verdict(v) => {
                assert_eq!(v.score, verdict.score);
                assert_eq!(v.flagged, verdict.flagged);
                assert_eq!(v.records, verdict.records);
                assert_eq!(v.status, SessionStatus::Finished);
            }
            other => panic!("{other:?}"),
        }
    }

    // Seeds are derived, not stored: the master secret and the ids reproduce them.
    let root = RootSeed::from_bytes(&mut MASTER.clone());
    for opened in [&aimbot, &smoothing, &human] {
        let seed = root
            .match_key(opened.match_id.as_bytes())
            .player_key(opened.player_id)
            .epoch_seed(0);
        assert_eq!(seed.expose_secret(), &opened.seed_bytes);
    }

    server.stop().await;
    // Everything is in SQLite after the server has gone.
    let store = server.store();
    let stored = store.sessions().unwrap();
    assert_eq!(stored.len(), 3);
    assert!(stored.iter().all(|(_, s)| *s == SessionStatus::Finished));
    let v = store.verdict(aimbot.session_id).unwrap().unwrap();
    assert_eq!(v.score, aimbot_verdict.score);
    assert_eq!(v.error.pairs, aimbot_verdict.error.pairs);
    assert_eq!(
        store.window_rows(aimbot.session_id).unwrap(),
        8,
        "4 windows x 2 statistics"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn verdict_of_an_open_session_is_live() {
    let mut server = TestServer::start("e2e-live", |_| {}).await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    let chunks = telemetry(&opened, Class::FlickAimbot, Scenario::Flick, 20.0, 4, 200);
    send_chunks(
        &mut client,
        opened.session_id,
        &chunks[..chunks.len() / 2],
        0,
    )
    .await;
    match client
        .request(&ClientMessage::Verdict {
            session_id: opened.session_id,
        })
        .await
        .unwrap()
    {
        ServerMessage::Verdict(v) => {
            assert_eq!(v.status, SessionStatus::Open);
            assert!(v.records > 0 && v.shots > 0, "{v:?}");
        }
        other => panic!("{other:?}"),
    }
    server.stop().await;
}
