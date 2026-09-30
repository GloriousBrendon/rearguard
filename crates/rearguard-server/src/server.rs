// SPDX-License-Identifier: MIT OR Apache-2.0

//! The server: accepts connections, issues session seeds, feeds telemetry to the
//! detector and stores evidence.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rearguard_core::detect::{Detector, Report};
use rearguard_core::probe::{Amplitude, ProbeConfig, RootSeed};
use rearguard_core::protocol::{
    ClientMessage, ErrorCode, EvidenceSummary, LENGTH_PREFIX, ProtocolError, ServerMessage,
    SessionStatus, SessionToken, VerdictReport, WireRecord, WireSeed, decode_payload, encode_frame,
    payload_length,
};
use rearguard_core::telemetry::Record;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::config::{Config, Limits};
use crate::limit::Bucket;
use crate::log::Logger;
use crate::store::{NewSession, Store};

/// Player identifier within a match. v0 has one player per match (no multiplayer).
const PLAYER_ID: u64 = 1;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A random identifier below 2^63 (so it fits SQLite's signed integers).
fn random_id() -> io::Result<u64> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(u64::from_le_bytes(b) >> 1)
}

/// An open session held in memory.
struct Live {
    match_id: String,
    token: SessionToken,
    detector: Detector,
    next_seq: u64,
    records: u64,
    /// The connection holding the session, if any.
    attached: Option<u64>,
    /// When the session lost its connection.
    detached_since: Option<Instant>,
    /// Completed windows already stored.
    persisted_windows: usize,
}

impl Live {
    fn verdict(&self, session_id: u64, status: SessionStatus) -> VerdictReport {
        let s = self.detector.session();
        let windows = self.detector.windows();
        VerdictReport {
            session_id,
            status,
            score: s.error.score.max(s.steps.score),
            confidence: s.error.confidence.max(s.steps.confidence),
            flagged: s.flagged(),
            records: self.records,
            shots: s.shots,
            engagements: s.engagements,
            angle_mismatches: s.angle_mismatches,
            error: EvidenceSummary::from(&s.error),
            steps: EvidenceSummary::from(&s.steps),
            windows: windows.len() as u64,
            flagged_windows: windows.iter().filter(|w| w.flagged()).count() as u64,
        }
    }
}

struct Shared {
    config: Config,
    root: RootSeed,
    store: Arc<Store>,
    log: Logger,
    sessions: Mutex<HashMap<u64, Live>>,
    next_connection: AtomicU64,
}

/// A bound server, ready to [`Server::run`].
pub struct Server {
    shared: Arc<Shared>,
    listener: TcpListener,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Server")
    }
}

impl Server {
    /// Binds the configured loopback address.
    ///
    /// # Errors
    /// An invalid configuration, or a bind failure.
    pub async fn bind(
        config: Config,
        root: RootSeed,
        store: Store,
        log: Logger,
    ) -> io::Result<Self> {
        config.validate().map_err(io::Error::other)?;
        let listener = TcpListener::bind(config.listen).await?;
        let shared = Arc::new(Shared {
            config,
            root,
            store: Arc::new(store),
            log,
            sessions: Mutex::new(HashMap::new()),
            next_connection: AtomicU64::new(1),
        });
        Ok(Self { shared, listener })
    }

    /// The address actually bound (useful with port 0).
    ///
    /// # Errors
    /// The socket's error, if any.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Serves until `shutdown` completes, then closes every connection and stores every
    /// open session as abandoned.
    ///
    /// # Errors
    /// A fatal listener error.
    pub async fn run(self, shutdown: impl Future<Output = ()>) -> io::Result<()> {
        let Self { shared, listener } = self;
        shared
            .log
            .info(format_args!("listening on {}", listener.local_addr()?));
        let (stop_tx, stop_rx) = watch::channel(false);
        let mut connections = JoinSet::new();
        let reap_every =
            Duration::from_millis((shared.config.limits.resume_timeout_ms / 4).clamp(10, 1_000));
        let mut reaper = tokio::time::interval(reap_every);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, peer)) => {
                        let id = shared.next_connection.fetch_add(1, Ordering::Relaxed);
                        connections.spawn(connection(Arc::clone(&shared), stream, peer, id, stop_rx.clone()));
                    }
                    Err(e) => shared.log.info(format_args!("accept failed: {e}")),
                },
                _ = reaper.tick() => reap(&shared).await,
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
                () = &mut shutdown => break,
            }
        }
        drop(listener);
        let _ = stop_tx.send(true);
        while connections.join_next().await.is_some() {}
        let open: Vec<u64> = shared
            .sessions
            .lock()
            .map(|s| s.keys().copied().collect())
            .unwrap_or_default();
        for id in open {
            close_session(&shared, id, SessionStatus::Abandoned).await;
        }
        shared.log.info("stopped");
        Ok(())
    }
}

/// Closes sessions whose connection has been gone longer than the resume timeout.
async fn reap(shared: &Arc<Shared>) {
    let limit = Duration::from_millis(shared.config.limits.resume_timeout_ms);
    let expired: Vec<u64> = shared
        .sessions
        .lock()
        .map(|s| {
            s.iter()
                .filter(|(_, l)| {
                    l.attached.is_none() && l.detached_since.is_some_and(|t| t.elapsed() >= limit)
                })
                .map(|(id, _)| *id)
                .collect()
        })
        .unwrap_or_default();
    for id in expired {
        close_session(shared, id, SessionStatus::Abandoned).await;
    }
}

/// Removes a session from memory and stores its final evidence and verdict.
async fn close_session(
    shared: &Arc<Shared>,
    session_id: u64,
    status: SessionStatus,
) -> Option<VerdictReport> {
    let live = shared.sessions.lock().ok()?.remove(&session_id)?;
    let verdict = live.verdict(session_id, status);
    let match_id = live.match_id.clone();
    let session = live.detector.session();
    let pending: Vec<Report> = live.detector.windows()[live.persisted_windows..].to_vec();
    let first = live.persisted_windows;
    let store = Arc::clone(&shared.store);
    let stored = verdict.clone();
    let result = tokio::task::spawn_blocking(move || {
        store.add_windows(session_id, first, &pending)?;
        store.finish_session(&stored, &session, now_ms())
    })
    .await;
    match result {
        Ok(Ok(())) => shared.log.info(format_args!(
            "session {session_id} (match {match_id}) {} after {} records: score {:.2}, flagged {}",
            status.name(),
            verdict.records,
            verdict.score,
            verdict.flagged
        )),
        _ => shared.log.info(format_args!(
            "session {session_id}: storing the verdict failed"
        )),
    }
    Some(verdict)
}

enum ReadError {
    Closed,
    Protocol(ProtocolError),
}

/// Reads one frame's payload. The length is checked before anything is allocated.
async fn read_frame(reader: &mut OwnedReadHalf, max: usize) -> Result<Vec<u8>, ReadError> {
    let mut prefix = [0u8; LENGTH_PREFIX];
    reader
        .read_exact(&mut prefix)
        .await
        .map_err(|_| ReadError::Closed)?;
    let length = payload_length(prefix, max).map_err(ReadError::Protocol)?;
    let mut payload = vec![0u8; length];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|_| ReadError::Closed)?;
    Ok(payload)
}

async fn send(writer: &mut OwnedWriteHalf, message: &ServerMessage) -> bool {
    match encode_frame(message) {
        Ok(frame) => writer.write_all(&frame).await.is_ok(),
        Err(_) => false,
    }
}

fn error(code: ErrorCode) -> ServerMessage {
    ServerMessage::Error { code }
}

/// One connection, until it closes, misbehaves, idles out or the server stops.
async fn connection(
    shared: Arc<Shared>,
    stream: TcpStream,
    peer: SocketAddr,
    id: u64,
    mut stop: watch::Receiver<bool>,
) {
    let limits = shared.config.limits;
    let (mut reader, mut writer) = stream.into_split();
    let start = Instant::now();
    let mut messages = Bucket::new(limits.messages_per_second, limits.message_burst, start);
    let mut bytes = Bucket::new(limits.bytes_per_second, limits.byte_burst, start);
    let mut held: Option<u64> = None;
    let idle = Duration::from_millis(limits.idle_timeout_ms);
    let reason = loop {
        let frame = tokio::select! {
            r = tokio::time::timeout(idle, read_frame(&mut reader, limits.max_frame_bytes)) => r,
            _ = stop.changed() => break "server stopping",
        };
        let payload = match frame {
            Err(_) => break "idle timeout",
            Ok(Err(ReadError::Closed)) => break "closed by peer",
            Ok(Err(ReadError::Protocol(e))) => {
                send(&mut writer, &error(ErrorCode::Protocol(e))).await;
                break "protocol error";
            }
            Ok(Ok(payload)) => payload,
        };
        let now = Instant::now();
        if !(messages.take(1.0, now) && bytes.take((payload.len() + LENGTH_PREFIX) as f64, now)) {
            send(&mut writer, &error(ErrorCode::RateLimited)).await;
            break "rate limited";
        }
        let message = match decode_payload::<ClientMessage>(&payload) {
            Ok(m) => m,
            Err(e) => {
                send(&mut writer, &error(ErrorCode::Protocol(e))).await;
                break "protocol error";
            }
        };
        let reply = handle(&shared, &limits, message, id, &mut held).await;
        if !send(&mut writer, &reply).await {
            break "write failed";
        }
    };
    if let Some(session_id) = held
        && let Ok(mut sessions) = shared.sessions.lock()
        && let Some(live) = sessions.get_mut(&session_id)
        && live.attached == Some(id)
    {
        live.attached = None;
        live.detached_since = Some(Instant::now());
        shared.log.info(format_args!(
            "session {session_id} detached; resumable for {} ms",
            limits.resume_timeout_ms
        ));
    }
    shared
        .log
        .info(format_args!("connection {id} from {peer} closed: {reason}"));
}

fn records_are_valid(records: &[WireRecord], limits: &Limits) -> bool {
    let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
    records.len() <= limits.max_records_per_message
        && records.iter().all(|r| match r {
            WireRecord::Header(h) => finite(&[
                h.deg_per_count,
                h.duration_s,
                h.target_distance_m,
                h.target_radius_m,
            ]),
            WireRecord::Move(m) => finite(&[m.dx, m.dy, m.yaw, m.pitch]),
            WireRecord::Fire(f) => finite(&[f.yaw, f.pitch, f.target_yaw, f.target_pitch]),
            WireRecord::Recoil(r) => finite(&[r.kick_yaw, r.kick_pitch, r.yaw, r.pitch]),
            WireRecord::Target(t) => finite(&[t.target_yaw, t.target_pitch]),
            WireRecord::Button(_) | WireRecord::End(_) => true,
        })
}

async fn handle(
    shared: &Arc<Shared>,
    limits: &Limits,
    message: ClientMessage,
    conn: u64,
    held: &mut Option<u64>,
) -> ServerMessage {
    match message {
        ClientMessage::Hello { client } => hello(shared, client, conn, held).await,
        ClientMessage::Resume { session_id, token } => {
            if held.is_some() {
                return error(ErrorCode::NotAllowed);
            }
            let Ok(mut sessions) = shared.sessions.lock() else {
                return error(ErrorCode::Internal);
            };
            let Some(live) = sessions.get_mut(&session_id) else {
                return error(ErrorCode::UnknownSession);
            };
            if live.token != token {
                return error(ErrorCode::BadToken);
            }
            if live.attached.is_some() {
                return error(ErrorCode::SessionBusy);
            }
            live.attached = Some(conn);
            live.detached_since = None;
            *held = Some(session_id);
            shared.log.info(format_args!(
                "session {session_id} resumed on connection {conn} at seq {}",
                live.next_seq
            ));
            ServerMessage::Resumed {
                session_id,
                next_seq: live.next_seq,
            }
        }
        ClientMessage::Telemetry {
            session_id,
            seq,
            records,
        } => {
            if *held != Some(session_id) {
                return error(ErrorCode::NotAllowed);
            }
            let persist = {
                let Ok(mut sessions) = shared.sessions.lock() else {
                    return error(ErrorCode::Internal);
                };
                let Some(live) = sessions.get_mut(&session_id) else {
                    return error(ErrorCode::UnknownSession);
                };
                if seq < live.next_seq {
                    return error(ErrorCode::Replayed);
                }
                if seq > live.next_seq {
                    return error(ErrorCode::OutOfOrder);
                }
                // The chunk number is used up whatever happens next, so a resend of a
                // rejected chunk is a replay.
                live.next_seq += 1;
                if !records_are_valid(&records, limits) {
                    return error(ErrorCode::InvalidTelemetry);
                }
                let n = records.len() as u64;
                let records: Vec<Record> = records.into_iter().map(Record::from).collect();
                // Records before a bad one are kept: the detector stops at the first
                // invalid record.
                let fed = live.detector.feed(&records);
                live.records += n;
                let done = live.detector.windows().len();
                let pending = (done > live.persisted_windows).then(|| {
                    (
                        live.persisted_windows,
                        live.detector.windows()[live.persisted_windows..].to_vec(),
                    )
                });
                live.persisted_windows = done;
                if fed.is_err() {
                    return error(ErrorCode::InvalidTelemetry);
                }
                pending
            };
            if let Some((first, windows)) = persist {
                let store = Arc::clone(&shared.store);
                let stored = tokio::task::spawn_blocking(move || {
                    store.add_windows(session_id, first, &windows)
                })
                .await;
                if !matches!(stored, Ok(Ok(()))) {
                    shared.log.info(format_args!(
                        "session {session_id}: storing window evidence failed"
                    ));
                }
            }
            ServerMessage::Ack { session_id, seq }
        }
        ClientMessage::Finish { session_id, seq } => {
            if *held != Some(session_id) {
                return error(ErrorCode::NotAllowed);
            }
            let expected = shared
                .sessions
                .lock()
                .ok()
                .and_then(|s| s.get(&session_id).map(|l| l.next_seq));
            match expected {
                None => return error(ErrorCode::UnknownSession),
                Some(next) if seq < next => return error(ErrorCode::Replayed),
                Some(next) if seq > next => return error(ErrorCode::OutOfOrder),
                Some(_) => {}
            }
            *held = None;
            match close_session(shared, session_id, SessionStatus::Finished).await {
                Some(v) => ServerMessage::Verdict(v),
                None => error(ErrorCode::UnknownSession),
            }
        }
        ClientMessage::Verdict { session_id } => {
            let live = shared.sessions.lock().ok().and_then(|s| {
                s.get(&session_id)
                    .map(|l| l.verdict(session_id, SessionStatus::Open))
            });
            if let Some(v) = live {
                return ServerMessage::Verdict(v);
            }
            let store = Arc::clone(&shared.store);
            match tokio::task::spawn_blocking(move || store.verdict(session_id)).await {
                Ok(Ok(Some(v))) => ServerMessage::Verdict(v),
                Ok(Ok(None)) => error(ErrorCode::UnknownSession),
                _ => error(ErrorCode::Internal),
            }
        }
    }
}

async fn hello(
    shared: &Arc<Shared>,
    client: String,
    conn: u64,
    held: &mut Option<u64>,
) -> ServerMessage {
    if held.is_some() {
        return error(ErrorCode::NotAllowed);
    }
    let open = shared
        .sessions
        .lock()
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    if open >= shared.config.limits.max_open_sessions {
        return error(ErrorCode::ServerBusy);
    }
    let (Ok(session_id), Ok(match_number)) = (random_id(), random_id()) else {
        return error(ErrorCode::Internal);
    };
    let mut token = [0u8; 16];
    if getrandom::fill(&mut token).is_err() {
        return error(ErrorCode::Internal);
    }
    let token = SessionToken(token);
    let match_id = format!("m-{match_number:016x}");
    // Decision D5, v0: one epoch (number 0) covering the whole match. The seed is
    // derived, never stored: master secret -> match -> player -> epoch.
    let seed = shared
        .root
        .match_key(match_id.as_bytes())
        .player_key(PLAYER_ID)
        .epoch_seed(0);
    let amplitude = Amplitude::from_ppm(shared.config.amplitude_ppm).unwrap_or(Amplitude::DEFAULT);
    let probe = ProbeConfig {
        amplitude,
        ..ProbeConfig::default()
    };
    let Ok(detector) = Detector::new(&seed, &probe, shared.config.detector.clone()) else {
        return error(ErrorCode::Internal);
    };
    let client: String = client.chars().take(64).collect();
    let row = NewSession {
        session_id,
        match_id: match_id.clone(),
        player_id: PLAYER_ID,
        amplitude_ppm: amplitude.ppm(),
        client: client.clone(),
        created_ms: now_ms(),
    };
    let store = Arc::clone(&shared.store);
    if !matches!(
        tokio::task::spawn_blocking(move || store.create_session(&row)).await,
        Ok(Ok(()))
    ) {
        return error(ErrorCode::Internal);
    }
    let Ok(mut sessions) = shared.sessions.lock() else {
        return error(ErrorCode::Internal);
    };
    sessions.insert(
        session_id,
        Live {
            match_id: match_id.clone(),
            token: token.clone(),
            detector,
            next_seq: 0,
            records: 0,
            attached: Some(conn),
            detached_since: None,
            persisted_windows: 0,
        },
    );
    drop(sessions);
    *held = Some(session_id);
    shared.log.info(format_args!(
        "session {session_id} opened for match {match_id} by '{client}' on connection {conn}"
    ));
    ServerMessage::Welcome {
        session_id,
        token,
        match_id,
        player_id: PLAYER_ID,
        epoch: 0,
        epoch_seed: WireSeed(seed),
        amplitude_ppm: amplitude.ppm(),
    }
}
