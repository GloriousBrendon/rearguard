// SPDX-License-Identifier: MIT OR Apache-2.0

//! The client uplink (`rearguard_core::uplink`) against the real server, including its
//! defined behaviour when the server connection drops (task 1.7, criterion 4).

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use rearguard_core::protocol::SessionStatus;
use rearguard_core::telemetry::Record;
use rearguard_core::uplink::{SessionInfo, Uplink, UplinkConfig, UplinkStatus};
use rearguard_sim::seed::SimSeed;
use rearguard_sim::session::{Class, ModelParams, SessionSpec, run_session_as};
use rearguard_sim::world::Scenario;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// A TCP proxy in front of the server that can cut every live connection, or stop
/// accepting altogether, to simulate network trouble.
struct Proxy {
    addr: SocketAddr,
    pipes: Arc<Mutex<Vec<JoinHandle<()>>>>,
    acceptor: JoinHandle<()>,
    /// Accept new connections but never answer (a hung server, or a network that
    /// swallows packets): every resume attempt blocks until it times out.
    black_hole: Arc<AtomicBool>,
}

impl Proxy {
    async fn start(target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let pipes: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
        let black_hole = Arc::new(AtomicBool::new(false));
        let list = Arc::clone(&pipes);
        let hole = Arc::clone(&black_hole);
        let acceptor = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let swallow = hole.load(Ordering::SeqCst);
                let pipe = tokio::spawn(async move {
                    if swallow {
                        let _held = inbound;
                        tokio::time::sleep(Duration::from_secs(3_600)).await;
                    } else if let Ok(mut outbound) = TcpStream::connect(target).await {
                        let _ = copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
                list.lock().unwrap().push(pipe);
            }
        });
        Self {
            addr,
            pipes,
            acceptor,
            black_hole,
        }
    }

    /// Cuts every live connection; new ones are accepted and then ignored.
    fn black_hole(&self) {
        self.black_hole.store(true, Ordering::SeqCst);
        self.cut();
    }

    /// Drops every live connection (both sides see a reset or end of stream).
    fn cut(&self) {
        for p in self.pipes.lock().unwrap().drain(..) {
            p.abort();
        }
    }

    /// Cuts everything and stops accepting: the server looks unreachable.
    fn stop(&self) {
        self.acceptor.abort();
        self.cut();
    }
}

fn config(addr: SocketAddr) -> UplinkConfig {
    UplinkConfig {
        retry_interval: Duration::from_millis(100),
        give_up_after: Duration::from_secs(3),
        chunk_records: 128,
        flush_interval: Duration::from_millis(50),
        ..UplinkConfig::local(addr, "uplink-test/0")
    }
}

/// Sim records played under the uplink's session seed.
fn records(
    info: &SessionInfo,
    seed: &rearguard_core::probe::EpochSeed,
    class: Class,
    seconds: f64,
) -> Vec<Record> {
    let spec = SessionSpec {
        class,
        scenario: Scenario::Flick,
        index: 0,
        duration_s: seconds,
        amplitude: rearguard_core::probe::Amplitude::from_ppm(info.amplitude_ppm).unwrap(),
        models: ModelParams::DEFAULT,
    };
    run_session_as(
        &SimSeed::new(11),
        seed,
        &info.match_id,
        info.player_id,
        spec,
    )
    .records
}

fn wait_for(
    uplink: &Uplink,
    done: impl Fn(&UplinkStatus) -> bool,
    limit: Duration,
) -> UplinkStatus {
    let start = Instant::now();
    loop {
        let status = uplink.status();
        if done(&status) || start.elapsed() > limit {
            return status;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_streams_to_a_final_verdict() {
    let mut server = TestServer::start("uplink-normal", |_| {}).await;
    let addr = server.addr;
    let (verdict, stats, total) = tokio::task::spawn_blocking(move || {
        let (uplink, info, seed) = Uplink::connect(config(addr)).unwrap();
        let records = records(&info, &seed, Class::FlickAimbot, 30.0);
        let total = records.len() as u64;
        for r in records {
            uplink.record(r);
        }
        uplink.finish();
        let status = wait_for(
            &uplink,
            |s| *s == UplinkStatus::Finished,
            Duration::from_secs(20),
        );
        assert_eq!(status, UplinkStatus::Finished);
        (uplink.verdict().unwrap(), uplink.stats(), total)
    })
    .await
    .unwrap();
    assert_eq!(verdict.status, SessionStatus::Finished);
    assert!(verdict.flagged, "{verdict:?}");
    assert_eq!(verdict.records, total);
    assert_eq!(stats.records_queued, total);
    assert_eq!(stats.chunks_acked, stats.chunks_sent);
    assert_eq!(stats.resumes, 0);
    assert!(stats.bytes_sent > total * 10, "{stats:?}");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_connection_resumes_and_nothing_is_lost() {
    let mut server = TestServer::start("uplink-drop", |_| {}).await;
    let proxy = Proxy::start(server.addr).await;
    let addr = proxy.addr;
    let proxy = Arc::new(proxy);
    let cutter = Arc::clone(&proxy);
    let (verdict, stats, total) = tokio::task::spawn_blocking(move || {
        let (uplink, info, seed) = Uplink::connect(config(addr)).unwrap();
        let records = records(&info, &seed, Class::FlickAimbot, 30.0);
        let total = records.len() as u64;
        let half = records.len() / 2;
        for (i, r) in records.into_iter().enumerate() {
            if i == half {
                cutter.cut();
                // Keep playing while disconnected: records are buffered.
                std::thread::sleep(Duration::from_millis(30));
            }
            uplink.record(r);
        }
        uplink.finish();
        let status = wait_for(
            &uplink,
            |s| *s == UplinkStatus::Finished,
            Duration::from_secs(20),
        );
        assert_eq!(status, UplinkStatus::Finished, "{:?}", uplink.stats());
        (uplink.verdict().unwrap(), uplink.stats(), total)
    })
    .await
    .unwrap();
    assert!(stats.resumes >= 1, "{stats:?}");
    assert_eq!(
        verdict.records, total,
        "every record reached the server exactly once"
    );
    assert!(verdict.flagged);
    assert_eq!(stats.records_dropped, 0);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_server_ends_in_lost_without_blocking_the_game() {
    let mut server = TestServer::start("uplink-gone", |_| {}).await;
    let proxy = Arc::new(Proxy::start(server.addr).await);
    let addr = proxy.addr;
    let stopper = Arc::clone(&proxy);
    tokio::task::spawn_blocking(move || {
        let (uplink, info, seed) = Uplink::connect(config(addr)).unwrap();
        let records = records(&info, &seed, Class::Human, 20.0);
        let half = records.len() / 2;
        stopper.stop();
        // Recording never blocks, connected or not.
        let start = Instant::now();
        for r in records.into_iter().skip(half) {
            uplink.record(r);
        }
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "record() blocked: {:?}",
            start.elapsed()
        );
        let status = wait_for(
            &uplink,
            |s| matches!(s, UplinkStatus::Lost(_)),
            Duration::from_secs(10),
        );
        assert!(
            matches!(status, UplinkStatus::Lost(ref why) if why.contains("unreachable")),
            "{status:?}"
        );
        // After that, records and finish are accepted and dropped, instantly.
        uplink.record(Record::End(rearguard_core::telemetry::End {
            ts_us: 1,
            frame: 1,
            tick: 1,
            shots: 0,
            hits: 0,
            complete: true,
        }));
        uplink.finish();
        assert!(uplink.verdict().is_none());
        let started = Instant::now();
        drop(uplink);
        assert!(started.elapsed() < Duration::from_secs(2), "drop is prompt");
    })
    .await
    .unwrap();
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_refused_by_the_server_ends_in_lost() {
    // The server closes the session as abandoned before the client comes back (the same
    // happens when a restarted server has never heard of it).
    let mut server =
        TestServer::start("uplink-refused", |c| c.limits.resume_timeout_ms = 100).await;
    let proxy = Arc::new(Proxy::start(server.addr).await);
    let addr = proxy.addr;
    let cutter = Arc::clone(&proxy);
    tokio::task::spawn_blocking(move || {
        let cfg = UplinkConfig { retry_interval: Duration::from_millis(600), ..config(addr) };
        let (uplink, info, seed) = Uplink::connect(cfg).unwrap();
        for r in records(&info, &seed, Class::Human, 5.0) {
            uplink.record(r);
        }
        std::thread::sleep(Duration::from_millis(200));
        cutter.cut();
        let status = wait_for(&uplink, |s| matches!(s, UplinkStatus::Lost(_)), Duration::from_secs(10));
        assert!(matches!(status, UplinkStatus::Lost(ref why) if why.contains("refused") || why.contains("no longer")), "{status:?}");
    })
    .await
    .unwrap();
    server.stop().await;
    assert_eq!(
        server.store().sessions().unwrap()[0].1,
        SessionStatus::Abandoned
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_buffer_cap_holds_while_disconnected() {
    let mut server = TestServer::start("uplink-cap", |_| {}).await;
    let proxy = Arc::new(Proxy::start(server.addr).await);
    let addr = proxy.addr;
    let stopper = Arc::clone(&proxy);
    tokio::task::spawn_blocking(move || {
        let cfg = UplinkConfig {
            max_buffered_bytes: 20_000,
            give_up_after: Duration::from_secs(60),
            ..config(addr)
        };
        let (uplink, info, seed) = Uplink::connect(cfg).unwrap();
        stopper.stop();
        std::thread::sleep(Duration::from_millis(100));
        for r in records(&info, &seed, Class::FlickAimbot, 30.0) {
            uplink.record(r);
        }
        let status = wait_for(
            &uplink,
            |s| matches!(s, UplinkStatus::Lost(_)),
            Duration::from_secs(10),
        );
        assert!(
            matches!(status, UplinkStatus::Lost(ref why) if why.contains("buffer")),
            "{status:?}"
        );
        assert!(uplink.stats().records_dropped > 0);
    })
    .await
    .unwrap();
    server.stop().await;
}

/// On some systems (Windows) a refused loopback connection takes seconds to fail, and a
/// hung server makes every resume attempt wait for its timeout. Queued records must
/// still reach the buffer accounting promptly, so the cap holds and memory stays bounded.
#[tokio::test(flavor = "multi_thread")]
async fn the_buffer_cap_holds_when_reconnection_attempts_hang() {
    let mut server = TestServer::start("uplink-hang", |_| {}).await;
    let proxy = Arc::new(Proxy::start(server.addr).await);
    let addr = proxy.addr;
    let hole = Arc::clone(&proxy);
    tokio::task::spawn_blocking(move || {
        let cfg = UplinkConfig {
            max_buffered_bytes: 20_000,
            give_up_after: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(2),
            ..config(addr)
        };
        let (uplink, info, seed) = Uplink::connect(cfg).unwrap();
        hole.black_hole();
        std::thread::sleep(Duration::from_millis(100));
        for r in records(&info, &seed, Class::FlickAimbot, 30.0) {
            uplink.record(r);
        }
        let status = wait_for(
            &uplink,
            |s| matches!(s, UplinkStatus::Lost(_)),
            Duration::from_secs(10),
        );
        assert!(
            matches!(status, UplinkStatus::Lost(ref why) if why.contains("buffer")),
            "{status:?}"
        );
    })
    .await
    .unwrap();
    proxy.stop();
    server.stop().await;
}
