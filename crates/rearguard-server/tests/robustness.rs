// SPDX-License-Identifier: MIT OR Apache-2.0

//! Hostile, broken and vanishing clients: acceptance criteria 2 (at the socket) and 4.

mod common;

use std::time::Duration;

use common::*;
use rearguard_core::protocol::{
    ClientMessage, ErrorCode, PROTOCOL_VERSION, ProtocolError, ServerMessage, SessionStatus,
    SessionToken, WireRecord, encode_frame,
};
use rearguard_core::telemetry::Move;
use rearguard_server::client::Client;
use rearguard_sim::session::Class;
use rearguard_sim::world::Scenario;

fn is_error(reply: &ServerMessage, code: ErrorCode) -> bool {
    matches!(reply, ServerMessage::Error { code: c } if *c == code)
}

async fn raw_then_error(server: &TestServer, bytes: &[u8]) -> ServerMessage {
    let mut client = Client::connect(server.addr).await.unwrap();
    client.send_raw(bytes).await.unwrap();
    let reply = client.receive().await.unwrap();
    assert!(
        client.is_closed().await,
        "connection closed after {reply:?}"
    );
    reply
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_empty_malformed_and_wrong_version_frames_are_rejected() {
    let mut server = TestServer::start("frames", |c| c.limits.max_frame_bytes = 1_024).await;

    // Oversized: rejected from the prefix alone, before the payload is sent.
    let reply = raw_then_error(&server, &2_000u32.to_le_bytes()).await;
    assert!(
        is_error(&reply, ErrorCode::Protocol(ProtocolError::Oversized)),
        "{reply:?}"
    );
    let reply = raw_then_error(&server, &u32::MAX.to_le_bytes()).await;
    assert!(
        is_error(&reply, ErrorCode::Protocol(ProtocolError::Oversized)),
        "{reply:?}"
    );
    // Empty payload.
    let reply = raw_then_error(&server, &0u32.to_le_bytes()).await;
    assert!(
        is_error(&reply, ErrorCode::Protocol(ProtocolError::Malformed)),
        "{reply:?}"
    );
    // Garbage after a valid version byte.
    let mut frame = 5u32.to_le_bytes().to_vec();
    frame.extend_from_slice(&[PROTOCOL_VERSION, 0xFF, 0xFF, 0xFF, 0xFF]);
    let reply = raw_then_error(&server, &frame).await;
    assert!(
        is_error(&reply, ErrorCode::Protocol(ProtocolError::Malformed)),
        "{reply:?}"
    );
    // Another protocol version.
    let mut frame = encode_frame(&ClientMessage::Verdict { session_id: 1 }).unwrap();
    frame[4] = PROTOCOL_VERSION + 1;
    let reply = raw_then_error(&server, &frame).await;
    assert!(
        is_error(
            &reply,
            ErrorCode::Protocol(ProtocolError::UnsupportedVersion)
        ),
        "{reply:?}"
    );

    // The server is still healthy.
    let mut client = Client::connect(server.addr).await.unwrap();
    hello(&mut client).await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn random_byte_streams_do_not_hurt_the_server() {
    let mut server = TestServer::start("garbage", |c| c.limits.max_frame_bytes = 65_536).await;
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..64 {
        let mut client = Client::connect(server.addr).await.unwrap();
        let len = (next() % 300) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        // Half the time, a plausible length prefix with a random body.
        if next() % 2 == 0 && bytes.len() > 8 {
            let body = (bytes.len() - 4) as u32;
            bytes[..4].copy_from_slice(&body.to_le_bytes());
            bytes[4] = PROTOCOL_VERSION;
        }
        let _ = client.send_raw(&bytes).await;
        drop(client);
    }
    let (_, verdict) = play(server.addr, Class::FlickAimbot, Scenario::Flick, 10.0, 9).await;
    assert_eq!(verdict.status, SessionStatus::Finished);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn replayed_skipped_and_foreign_chunks_are_rejected() {
    let mut server = TestServer::start("replay", |_| {}).await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    let chunks = telemetry(&opened, Class::FlickAimbot, Scenario::Flick, 5.0, 5, 20);
    assert!(chunks.len() >= 4, "{} chunks", chunks.len());
    let id = opened.session_id;
    let t = |seq: u64, i: usize| ClientMessage::Telemetry {
        session_id: id,
        seq,
        records: chunks[i].clone(),
    };

    assert!(matches!(
        client.request(&t(0, 0)).await.unwrap(),
        ServerMessage::Ack { seq: 0, .. }
    ));
    // A replay of chunk 0 (same number, same records) is refused, and changes nothing.
    assert!(is_error(
        &client.request(&t(0, 0)).await.unwrap(),
        ErrorCode::Replayed
    ));
    // Skipping ahead is refused.
    assert!(is_error(
        &client.request(&t(2, 2)).await.unwrap(),
        ErrorCode::OutOfOrder
    ));
    // The connection is still usable and in order.
    assert!(matches!(
        client.request(&t(1, 1)).await.unwrap(),
        ServerMessage::Ack { seq: 1, .. }
    ));
    // Finishing with a stale or future number is refused too.
    assert!(is_error(
        &client
            .request(&ClientMessage::Finish {
                session_id: id,
                seq: 1
            })
            .await
            .unwrap(),
        ErrorCode::Replayed
    ));
    assert!(is_error(
        &client
            .request(&ClientMessage::Finish {
                session_id: id,
                seq: 9
            })
            .await
            .unwrap(),
        ErrorCode::OutOfOrder
    ));

    // Another connection cannot write to (or finish) this session.
    let mut other = Client::connect(server.addr).await.unwrap();
    assert!(is_error(
        &other.request(&t(2, 2)).await.unwrap(),
        ErrorCode::NotAllowed
    ));
    assert!(is_error(
        &other
            .request(&ClientMessage::Finish {
                session_id: id,
                seq: 2
            })
            .await
            .unwrap(),
        ErrorCode::NotAllowed
    ));
    // Nor open a second session on the first connection.
    assert!(is_error(
        &client
            .request(&ClientMessage::Hello {
                client: "again".into(),
                label: None,
            })
            .await
            .unwrap(),
        ErrorCode::NotAllowed
    ));

    send_chunks(&mut client, id, &chunks, 2).await;
    let verdict = finish(&mut client, id, chunks.len() as u64).await;
    let total: usize = chunks.iter().map(Vec::len).sum();
    assert_eq!(verdict.records, total as u64, "the replay was not counted");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_telemetry_is_rejected() {
    let mut server = TestServer::start("invalid", |c| c.limits.max_records_per_message = 50).await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    let chunks = telemetry(&opened, Class::FlickAimbot, Scenario::Flick, 5.0, 5, 40);
    let id = opened.session_id;
    send_chunks(&mut client, id, &chunks[..1], 0).await;
    let bad_move = WireRecord::Move(Move {
        ts_us: u64::MAX / 2,
        frame: 1,
        tick: 1,
        dx: f64::NAN,
        dy: 0.0,
        yaw: 0.0,
        pitch: 0.0,
    });
    let reply = client
        .request(&ClientMessage::Telemetry {
            session_id: id,
            seq: 1,
            records: vec![bad_move],
        })
        .await
        .unwrap();
    assert!(
        is_error(&reply, ErrorCode::InvalidTelemetry),
        "non-finite: {reply:?}"
    );
    let too_many = chunks[1]
        .iter()
        .chain(&chunks[2])
        .cloned()
        .collect::<Vec<_>>();
    assert!(too_many.len() > 50);
    let reply = client
        .request(&ClientMessage::Telemetry {
            session_id: id,
            seq: 2,
            records: too_many,
        })
        .await
        .unwrap();
    assert!(
        is_error(&reply, ErrorCode::InvalidTelemetry),
        "too many: {reply:?}"
    );
    // A second header is out of order for the detector.
    let reply = client
        .request(&ClientMessage::Telemetry {
            session_id: id,
            seq: 3,
            records: chunks[0][..1].to_vec(),
        })
        .await
        .unwrap();
    assert!(
        is_error(&reply, ErrorCode::InvalidTelemetry),
        "second header: {reply:?}"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_flooding_connection_is_rate_limited_and_closed() {
    let mut server = TestServer::start("rate", |c| {
        c.limits.messages_per_second = 20.0;
        c.limits.message_burst = 10.0;
    })
    .await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let mut limited = false;
    for _ in 0..100 {
        match client
            .request(&ClientMessage::Verdict { session_id: 1 })
            .await
        {
            Ok(reply) if is_error(&reply, ErrorCode::RateLimited) => {
                limited = true;
                break;
            }
            Ok(reply) => assert!(is_error(&reply, ErrorCode::UnknownSession), "{reply:?}"),
            Err(e) => panic!("{e}"),
        }
    }
    assert!(limited, "never rate limited");
    assert!(client.is_closed().await);
    // Other connections are unaffected.
    let mut other = Client::connect(server.addr).await.unwrap();
    hello(&mut other).await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_open_session_cap_holds() {
    let mut server = TestServer::start("busy", |c| c.limits.max_open_sessions = 1).await;
    let mut a = Client::connect(server.addr).await.unwrap();
    hello(&mut a).await;
    let mut b = Client::connect(server.addr).await.unwrap();
    let reply = b
        .request(&ClientMessage::Hello {
            client: "b".into(),
            label: None,
        })
        .await
        .unwrap();
    assert!(is_error(&reply, ErrorCode::ServerBusy), "{reply:?}");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_session_resumes_on_a_new_connection() {
    let mut server = TestServer::start("resume", |_| {}).await;
    let mut first = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut first).await;
    let chunks = telemetry(&opened, Class::FlickAimbot, Scenario::Flick, 30.0, 6, 200);
    let half = chunks.len() / 2;
    send_chunks(&mut first, opened.session_id, &chunks[..half], 0).await;

    // While attached, nobody else can take it over.
    let mut thief = Client::connect(server.addr).await.unwrap();
    let busy = ClientMessage::Resume {
        session_id: opened.session_id,
        token: opened.token.clone(),
    };
    assert!(is_error(
        &thief.request(&busy).await.unwrap(),
        ErrorCode::SessionBusy
    ));
    drop(first); // The connection drops mid-session.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut second = Client::connect(server.addr).await.unwrap();
    let wrong = ClientMessage::Resume {
        session_id: opened.session_id,
        token: SessionToken([0; 16]),
    };
    assert!(is_error(
        &second.request(&wrong).await.unwrap(),
        ErrorCode::BadToken
    ));
    let unknown = ClientMessage::Resume {
        session_id: opened.session_id ^ 1,
        token: opened.token.clone(),
    };
    assert!(is_error(
        &second.request(&unknown).await.unwrap(),
        ErrorCode::UnknownSession
    ));
    match second
        .request(&ClientMessage::Resume {
            session_id: opened.session_id,
            token: opened.token.clone(),
        })
        .await
        .unwrap()
    {
        ServerMessage::Resumed { next_seq, .. } => assert_eq!(next_seq, half as u64),
        other => panic!("{other:?}"),
    }
    send_chunks(&mut second, opened.session_id, &chunks, half).await;
    let verdict = finish(&mut second, opened.session_id, chunks.len() as u64).await;
    let total: usize = chunks.iter().map(Vec::len).sum();
    assert_eq!(verdict.records, total as u64);
    assert_eq!(verdict.status, SessionStatus::Finished);
    assert!(verdict.flagged);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abandoned_session_is_closed_and_stored_after_the_resume_timeout() {
    let mut server = TestServer::start("abandon", |c| c.limits.resume_timeout_ms = 300).await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    let chunks = telemetry(&opened, Class::FlickAimbot, Scenario::Flick, 40.0, 7, 200);
    send_chunks(
        &mut client,
        opened.session_id,
        &chunks[..chunks.len() - 2],
        0,
    )
    .await;
    drop(client);
    tokio::time::sleep(Duration::from_millis(1_000)).await;

    let mut later = Client::connect(server.addr).await.unwrap();
    // Too late to resume.
    let resume = ClientMessage::Resume {
        session_id: opened.session_id,
        token: opened.token.clone(),
    };
    assert!(is_error(
        &later.request(&resume).await.unwrap(),
        ErrorCode::UnknownSession
    ));
    // But the evidence was kept, and the verdict says how the session ended.
    match later
        .request(&ClientMessage::Verdict {
            session_id: opened.session_id,
        })
        .await
        .unwrap()
    {
        ServerMessage::Verdict(v) => {
            assert_eq!(v.status, SessionStatus::Abandoned);
            assert!(v.flagged && v.records > 0, "{v:?}");
        }
        other => panic!("{other:?}"),
    }
    server.stop().await;
    assert_eq!(
        server.store().sessions().unwrap(),
        vec![(opened.session_id, SessionStatus::Abandoned)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_half_open_connection_is_closed_and_its_session_stays_resumable() {
    let mut server = TestServer::start("idle", |c| c.limits.idle_timeout_ms = 300).await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    // Half a frame, then silence: the server must not wait forever.
    client.send_raw(&100u32.to_le_bytes()).await.unwrap();
    client.send_raw(&[PROTOCOL_VERSION, 1, 2]).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(5), client.is_closed()).await;
    assert_eq!(closed, Ok(true), "server closed the idle connection");
    let mut again = Client::connect(server.addr).await.unwrap();
    let resume = ClientMessage::Resume {
        session_id: opened.session_id,
        token: opened.token.clone(),
    };
    assert!(matches!(
        again.request(&resume).await.unwrap(),
        ServerMessage::Resumed { next_seq: 0, .. }
    ));
    server.stop().await;
    assert!(server.log_text().contains("idle timeout"));
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_closes_connections_and_stores_open_sessions() {
    let mut server = TestServer::start("shutdown", |_| {}).await;
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    let chunks = telemetry(&opened, Class::Human, Scenario::Flick, 10.0, 8, 200);
    send_chunks(&mut client, opened.session_id, &chunks, 0).await;
    server.stop().await;
    assert!(client.is_closed().await, "connection closed on shutdown");
    let store = server.store();
    assert_eq!(
        store.sessions().unwrap(),
        vec![(opened.session_id, SessionStatus::Abandoned)]
    );
    let v = store.verdict(opened.session_id).unwrap().unwrap();
    assert_eq!(v.status, SessionStatus::Abandoned);
    assert!(v.records > 0);
}

#[tokio::test]
async fn non_loopback_addresses_are_refused() {
    let dir = temp_dir("bind");
    let mut config = config(&dir);
    config.listen = "0.0.0.0:0".parse().unwrap();
    let root = rearguard_core::probe::RootSeed::from_bytes(&mut MASTER.clone());
    let store = rearguard_server::store::Store::open_in_memory().unwrap();
    let (log, _) = rearguard_server::log::Logger::capture();
    let err = rearguard_server::server::Server::bind(config, root, store, log)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("loopback"), "{err}");
}
