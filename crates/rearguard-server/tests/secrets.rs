// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance criterion 3: neither the master secret nor any session seed (nor any
//! resume token) appears in logs, command output or the evidence database. Runs in CI
//! on every push (Linux and Windows), as part of `cargo test`.

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use rearguard_core::protocol::{ClientMessage, PROTOCOL_VERSION, SessionToken};
use rearguard_server::client::Client;
use rearguard_sim::session::Class;
use rearguard_sim::world::Scenario;

/// Raw byte search, for the database file.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn assert_file_has_no_secret(path: &std::path::Path, secrets: &[Vec<u8>]) {
    let bytes = std::fs::read(path).unwrap();
    for secret in secrets {
        assert!(
            !contains_bytes(&bytes, secret),
            "{} holds raw secret bytes",
            path.display()
        );
        for pattern in leak_patterns(secret) {
            assert!(
                !contains_bytes(&bytes, pattern.as_bytes()),
                "{} holds {pattern}",
                path.display()
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_never_appear_in_server_logs_or_the_database() {
    let mut server = TestServer::start("secrets", |c| c.limits.resume_timeout_ms = 200).await;
    let mut secrets: Vec<Vec<u8>> = vec![MASTER.to_vec()];

    // Normal sessions.
    for (i, class) in [Class::FlickAimbot, Class::Human, Class::SmoothingAimbot]
        .into_iter()
        .enumerate()
    {
        let (opened, _) = play(server.addr, class, Scenario::Flick, 20.0, 40 + i as u64).await;
        secrets.push(opened.seed_bytes.to_vec());
        secrets.push(opened.token.0.to_vec());
    }
    // Errors, a resume, an abandoned session and a session open at shutdown.
    let mut client = Client::connect(server.addr).await.unwrap();
    let opened = hello(&mut client).await;
    secrets.push(opened.seed_bytes.to_vec());
    secrets.push(opened.token.0.to_vec());
    let chunks = telemetry(&opened, Class::FlickAimbot, Scenario::Flick, 10.0, 50, 100);
    send_chunks(&mut client, opened.session_id, &chunks[..1], 0).await;
    drop(client);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut other = Client::connect(server.addr).await.unwrap();
    let _ = other
        .request(&ClientMessage::Resume {
            session_id: opened.session_id,
            token: SessionToken([7; 16]),
        })
        .await;
    let _ = other
        .request(&ClientMessage::Resume {
            session_id: opened.session_id,
            token: opened.token.clone(),
        })
        .await;
    drop(other);
    let mut bad = Client::connect(server.addr).await.unwrap();
    let _ = bad
        .send_raw(&[5, 0, 0, 0, PROTOCOL_VERSION, 0xFF, 0xFF, 0xFF, 0xFF])
        .await;
    let _ = bad.receive().await;
    tokio::time::sleep(Duration::from_millis(600)).await; // the session is abandoned
    let mut open_at_shutdown = Client::connect(server.addr).await.unwrap();
    let last = hello(&mut open_at_shutdown).await;
    secrets.push(last.seed_bytes.to_vec());
    secrets.push(last.token.0.to_vec());
    server.stop().await;

    let logs = server.log_text();
    assert!(
        logs.contains("listening on") && logs.contains("abandoned") && logs.contains("finished"),
        "{logs}"
    );
    assert!(
        logs.lines().count() > 15,
        "enough activity to be a meaningful check"
    );
    assert_no_leak("server log", &logs, &secrets);
    assert_file_has_no_secret(&server.dir.join("evidence.sqlite3"), &secrets);
}

/// Runs the real binary, as an operator would, and checks everything it prints.
#[tokio::test(flavor = "multi_thread")]
async fn the_server_binary_never_prints_secrets() {
    let bin = env!("CARGO_BIN_EXE_rearguard-server");
    let dir = temp_dir("binary");

    // gen-secret prints where it wrote the secret, not the secret.
    let generated = dir.join("generated.hex");
    let out = Command::new(bin)
        .arg("gen-secret")
        .arg(&generated)
        .output()
        .unwrap();
    assert!(out.status.success());
    let secret_hex = std::fs::read_to_string(&generated)
        .unwrap()
        .trim()
        .to_owned();
    assert_eq!(secret_hex.len(), 64);
    let printed =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        !printed.contains(&secret_hex) && !printed.contains(&secret_hex[..12]),
        "{printed}"
    );
    // It refuses to overwrite.
    assert!(
        !Command::new(bin)
            .arg("gen-secret")
            .arg(&generated)
            .output()
            .unwrap()
            .status
            .success()
    );

    // run: serve one client, then stop.
    std::fs::write(dir.join("master.hex"), format!("{}\n", hex(&MASTER))).unwrap();
    let config = include_str!("../config/server.example.json")
        .replace("127.0.0.1:7461", "127.0.0.1:0")
        .replace("server-master.hex", "master.hex")
        .replace("rearguard.sqlite3", "evidence.sqlite3");
    std::fs::write(dir.join("server.json"), config).unwrap();
    let mut child = Command::new(bin)
        .args(["run", "--config"])
        .arg(dir.join("server.json"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let stderr = child.stderr.take().unwrap();
    let sink = Arc::clone(&lines);
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            sink.lock().unwrap().push(line);
        }
    });
    let started = Instant::now();
    let addr = loop {
        let found = lines
            .lock()
            .unwrap()
            .iter()
            .find_map(|l| l.split("listening on ").nth(1).map(str::to_owned));
        if let Some(a) = found {
            break a.trim().parse().unwrap();
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "server did not start: {:?}",
            lines.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let (opened, verdict) = play(addr, Class::FlickAimbot, Scenario::Flick, 20.0, 60).await;
    assert!(verdict.flagged);
    let mut bad = Client::connect(addr).await.unwrap();
    let _ = bad
        .request(&ClientMessage::Resume {
            session_id: opened.session_id,
            token: SessionToken([1; 16]),
        })
        .await;
    drop(bad);
    tokio::time::sleep(Duration::from_millis(200)).await;
    child.kill().unwrap();
    let _ = child.wait();
    reader.join().unwrap();
    let mut printed = lines.lock().unwrap().join("\n");
    let mut stdout = String::new();
    if let Some(mut s) = child.stdout.take() {
        use std::io::Read as _;
        let _ = s.read_to_string(&mut stdout);
    }
    printed.push_str(&stdout);
    assert!(printed.contains("finished"), "{printed}");

    // verdict prints the stored verdict (numbers only).
    let out = Command::new(bin)
        .args(["verdict", "--db"])
        .arg(dir.join("evidence.sqlite3"))
        .args(["--session", &opened.session_id.to_string()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(json.contains("\"flagged\": true"), "{json}");
    printed.push_str(&json);

    let secrets = vec![
        MASTER.to_vec(),
        opened.seed_bytes.to_vec(),
        opened.token.0.to_vec(),
    ];
    assert_no_leak("server binary output", &printed, &secrets);
    assert_file_has_no_secret(&dir.join("evidence.sqlite3"), &secrets);
}

/// Negative control: the scanner does catch secrets in the forms it looks for.
#[test]
fn the_leak_scanner_catches_hex_and_debug_forms() {
    let secret = MASTER.to_vec();
    for leaked in [
        format!("session seed {}", hex(&MASTER)),
        format!("SEED={}", hex(&MASTER).to_uppercase()),
        format!("prefix {}...", hex(&MASTER[..6])),
        format!("bytes {:?}", &MASTER[..]),
    ] {
        let caught = std::panic::catch_unwind(|| {
            assert_no_leak("control", &leaked, std::slice::from_ref(&secret))
        });
        assert!(caught.is_err(), "missed: {leaked}");
    }
    assert_no_leak("control", "session 42 opened for match m-00ff", &[secret]);
}
