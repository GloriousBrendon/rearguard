// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared helpers for the server integration tests.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rearguard_core::probe::Amplitude;
use rearguard_core::protocol::{
    ClientMessage, ServerMessage, SessionToken, VerdictReport, WireRecord,
};
use rearguard_server::client::Client;
use rearguard_server::config::Config;
use rearguard_server::load_master_secret;
use rearguard_server::log::Logger;
use rearguard_server::server::Server;
use rearguard_server::store::Store;
use rearguard_sim::seed::SimSeed;
use rearguard_sim::session::{Class, ModelParams, SessionSpec, run_session_as};
use rearguard_sim::world::Scenario;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Distinctive master secret bytes, so any leak is unmistakable.
pub const MASTER: [u8; 32] = [
    0xC7, 0x1E, 0x55, 0xA3, 0x09, 0xD2, 0x6B, 0xF0, 0x3C, 0x84, 0x2F, 0x91, 0xE6, 0x5D, 0x07, 0xB8,
    0x4A, 0x13, 0xCE, 0x76, 0x98, 0x2B, 0xE1, 0x0F, 0x65, 0xDA, 0x31, 0x8C, 0x5E, 0xA9, 0x14, 0x77,
];

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A fresh directory under Cargo's per-target temporary directory.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("server-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The example configuration, on port 0, with paths in `dir`.
pub fn config(dir: &std::path::Path) -> Config {
    let text = include_str!("../../config/server.example.json");
    let mut config = Config::from_json(text).unwrap();
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.master_secret_file = dir.join("master.hex");
    config.database = dir.join("evidence.sqlite3");
    config
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub logs: Arc<Mutex<Vec<String>>>,
    pub dir: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<std::io::Result<()>>>,
}

impl TestServer {
    pub async fn start(name: &str, tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = temp_dir(name);
        let mut config = config(&dir);
        tweak(&mut config);
        std::fs::write(&config.master_secret_file, format!("{}\n", hex(&MASTER))).unwrap();
        let root = load_master_secret(&config.master_secret_file).unwrap();
        let store = Store::open(&config.database).unwrap();
        let (log, logs) = Logger::capture();
        let server = Server::bind(config, root, store, log).await.unwrap();
        let addr = server.local_addr().unwrap();
        let (tx, rx) = oneshot::channel();
        let handle = tokio::spawn(server.run(async {
            let _ = rx.await;
        }));
        Self {
            addr,
            logs,
            dir,
            stop: Some(tx),
            handle: Some(handle),
        }
    }

    /// Stops the server gracefully and waits for it.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            h.await.unwrap().unwrap();
        }
    }

    pub fn store(&self) -> Store {
        Store::open(&self.dir.join("evidence.sqlite3")).unwrap()
    }

    pub fn log_text(&self) -> String {
        self.logs.lock().unwrap().join("\n")
    }
}

/// What a client learns from `Welcome`.
pub struct Opened {
    pub session_id: u64,
    pub token: SessionToken,
    pub match_id: String,
    pub player_id: u64,
    /// Copy of the seed bytes, kept only so tests can search logs for them.
    pub seed_bytes: [u8; 32],
    pub seed: rearguard_core::probe::EpochSeed,
    pub amplitude_ppm: u32,
}

pub async fn hello(client: &mut Client) -> Opened {
    match client
        .request(&ClientMessage::Hello {
            client: "test-client/0".into(),
        })
        .await
        .unwrap()
    {
        ServerMessage::Welcome {
            session_id,
            token,
            match_id,
            player_id,
            epoch,
            epoch_seed,
            amplitude_ppm,
        } => {
            assert_eq!(epoch, 0);
            let seed_bytes = *epoch_seed.0.expose_secret();
            Opened {
                session_id,
                token,
                match_id,
                player_id,
                seed_bytes,
                seed: epoch_seed.0,
                amplitude_ppm,
            }
        }
        other => panic!("expected Welcome, got {other:?}"),
    }
}

/// The telemetry a simulated `class` player produces under the session's seed, in
/// chunks of `chunk` records.
pub fn telemetry(
    opened: &Opened,
    class: Class,
    scenario: Scenario,
    duration_s: f64,
    sim_seed: u64,
    chunk: usize,
) -> Vec<Vec<WireRecord>> {
    let spec = SessionSpec {
        class,
        scenario,
        index: 0,
        duration_s,
        amplitude: Amplitude::from_ppm(opened.amplitude_ppm).unwrap(),
        models: ModelParams::DEFAULT,
    };
    let session = run_session_as(
        &SimSeed::new(sim_seed),
        &opened.seed,
        &opened.match_id,
        opened.player_id,
        spec,
    );
    let records: Vec<WireRecord> = session.records.into_iter().map(WireRecord::from).collect();
    records.chunks(chunk).map(<[WireRecord]>::to_vec).collect()
}

/// Sends chunks `from..` of `chunks` with their sequence numbers, expecting acks.
pub async fn send_chunks(
    client: &mut Client,
    session_id: u64,
    chunks: &[Vec<WireRecord>],
    from: usize,
) {
    for (seq, records) in chunks.iter().enumerate().skip(from) {
        let reply = client
            .request(&ClientMessage::Telemetry {
                session_id,
                seq: seq as u64,
                records: records.clone(),
            })
            .await
            .unwrap();
        assert!(
            matches!(reply, ServerMessage::Ack { seq: s, .. } if s == seq as u64),
            "seq {seq}: {reply:?}"
        );
    }
}

pub async fn finish(client: &mut Client, session_id: u64, seq: u64) -> VerdictReport {
    match client
        .request(&ClientMessage::Finish { session_id, seq })
        .await
        .unwrap()
    {
        ServerMessage::Verdict(v) => v,
        other => panic!("expected Verdict, got {other:?}"),
    }
}

/// A whole session: a simulated player through the server to its verdict.
pub async fn play(
    addr: SocketAddr,
    class: Class,
    scenario: Scenario,
    duration_s: f64,
    sim_seed: u64,
) -> (Opened, VerdictReport) {
    let mut client = Client::connect(addr).await.unwrap();
    let opened = hello(&mut client).await;
    let chunks = telemetry(&opened, class, scenario, duration_s, sim_seed, 500);
    send_chunks(&mut client, opened.session_id, &chunks, 0).await;
    let verdict = finish(&mut client, opened.session_id, chunks.len() as u64).await;
    (opened, verdict)
}

/// Every rendering of `secret` that must never appear in logs or stored files.
pub fn leak_patterns(secret: &[u8]) -> Vec<String> {
    vec![
        hex(secret),
        hex(secret).to_uppercase(),
        hex(&secret[..6]),
        hex(&secret[..6]).to_uppercase(),
        format!("{secret:?}"),
        format!("{:?}", &secret[..6])
            .trim_end_matches(']')
            .to_owned(),
    ]
}

pub fn assert_no_leak(what: &str, text: &str, secrets: &[Vec<u8>]) {
    for secret in secrets {
        for pattern in leak_patterns(secret) {
            assert!(
                !text.contains(&pattern),
                "{what} contains secret material ({pattern})"
            );
        }
    }
}
