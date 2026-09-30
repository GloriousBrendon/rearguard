// SPDX-License-Identifier: MIT OR Apache-2.0

//! Client side of the wire protocol ([`crate::protocol`]): streams one session's
//! telemetry to a Rearguard server from a background thread.
//!
//! [`Uplink::connect`] performs the handshake (blocking, bounded by a timeout) and
//! returns the session's identifiers and epoch seed. After that, [`Uplink::record`] only
//! queues a record and never blocks. A worker thread batches records into numbered
//! chunks, sends them and tracks acknowledgements.
//!
//! # When the server goes away
//!
//! The drift keeps working, because the seed is already on the client (decision D5).
//! The worker keeps every unacknowledged chunk, and new records pile up behind them, up
//! to `max_buffered_bytes`. It then tries to re-attach the session with
//! [`ClientMessage::Resume`] every `retry_interval`:
//!
//! - **Resumed:** the server says which chunk it expects next; the worker drops what the
//!   server already has, resends the rest in order, and carries on. Nothing is lost.
//! - **Refused:** the server no longer knows the session (it was closed as abandoned, or
//!   restarted), or rejects the token. The status becomes [`UplinkStatus::Lost`].
//! - **Still unreachable after `give_up_after`,** or the buffer is full: also
//!   [`UplinkStatus::Lost`].
//!
//! Once lost, records are accepted and dropped. The game carries on and its local
//! recording is unaffected, but no verdict will come.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::probe::EpochSeed;
use crate::protocol::{
    ClientMessage, ErrorCode, LENGTH_PREFIX, ServerMessage, SessionStatus, SessionToken,
    VerdictReport, WireRecord, decode_payload, encode_frame, payload_length,
};
use crate::telemetry::Record;

/// Largest server message the client accepts.
const MAX_REPLY: usize = 1 << 20;

/// Connection and buffering settings.
#[derive(Clone, Debug, PartialEq)]
pub struct UplinkConfig {
    /// Server address.
    pub addr: SocketAddr,
    /// Client software name, sent in `Hello`.
    pub client_name: String,
    /// Ground-truth label, sent in `Hello`, for evaluation sessions in Rearguard's own
    /// test environment only (see [`ClientMessage::Hello`]). `None` for real play.
    pub label: Option<String>,
    /// Limit on connecting and on the handshake.
    pub connect_timeout: Duration,
    /// Records per telemetry chunk.
    pub chunk_records: usize,
    /// A partial chunk is sent after this long.
    pub flush_interval: Duration,
    /// Unacknowledged plus queued telemetry kept while disconnected, bytes.
    pub max_buffered_bytes: usize,
    /// Pause between reconnection attempts.
    pub retry_interval: Duration,
    /// Give up reconnecting after this long without a connection.
    pub give_up_after: Duration,
}

impl UplinkConfig {
    /// Settings suited to a game client on a local server: 256-record chunks sent at
    /// least every 250 ms, 16 MiB of buffer, a retry every 500 ms, giving up after
    /// 30 s.
    #[must_use]
    pub fn local(addr: SocketAddr, client_name: &str) -> Self {
        Self {
            addr,
            client_name: client_name.to_owned(),
            label: None,
            connect_timeout: Duration::from_secs(2),
            chunk_records: 256,
            flush_interval: Duration::from_millis(250),
            max_buffered_bytes: 16 << 20,
            retry_interval: Duration::from_millis(500),
            give_up_after: Duration::from_secs(30),
        }
    }
}

/// Why a connection could not be opened.
#[derive(Debug)]
pub enum UplinkError {
    /// Connecting or the handshake failed at the socket level.
    Io(io::Error),
    /// The server refused the session.
    Refused(ErrorCode),
    /// The server's reply was not a `Welcome`.
    Unexpected,
}

impl fmt::Display for UplinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "connection failed: {e}"),
            Self::Refused(code) => write!(f, "server refused the session: {code:?}"),
            Self::Unexpected => f.write_str("unexpected reply from the server"),
        }
    }
}

impl std::error::Error for UplinkError {}

/// What the server assigned to the session. (The seed is returned separately.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    /// Session identifier.
    pub session_id: u64,
    /// Match identifier, for the telemetry header.
    pub match_id: String,
    /// Player identifier, for the telemetry header.
    pub player_id: u64,
    /// Drift amplitude to apply, ppm.
    pub amplitude_ppm: u32,
}

/// Connection state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UplinkStatus {
    /// Connected and streaming.
    Connected,
    /// Disconnected; buffering and trying to resume.
    Reconnecting,
    /// Finish sent; waiting for the final verdict.
    Finishing,
    /// The final verdict arrived.
    Finished,
    /// The session cannot reach the server any more (see the module docs).
    Lost(String),
}

/// Counters, for the developer overlay and bandwidth measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UplinkStats {
    /// Frame bytes written to the socket (including resends).
    pub bytes_sent: u64,
    /// Frame bytes read from the socket.
    pub bytes_received: u64,
    /// Records handed to the uplink.
    pub records_queued: u64,
    /// Telemetry chunks sent (including resends).
    pub chunks_sent: u64,
    /// Chunks acknowledged by the server.
    pub chunks_acked: u64,
    /// Successful resumes.
    pub resumes: u64,
    /// Records dropped because the session was lost.
    pub records_dropped: u64,
}

#[derive(Debug)]
struct State {
    status: UplinkStatus,
    stats: UplinkStats,
    verdict: Option<VerdictReport>,
    last_error: Option<ErrorCode>,
}

enum Command {
    Record(Record),
    Finish,
    Verdict,
    Server(ServerMessage, u64),
    ReadFailed(u64),
    Shutdown,
}

/// A session's link to the server. Dropping it stops the worker thread.
pub struct Uplink {
    commands: Sender<Command>,
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
}

impl fmt::Debug for Uplink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Uplink")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

fn read_message(stream: &mut TcpStream) -> io::Result<(ServerMessage, u64)> {
    let mut prefix = [0u8; LENGTH_PREFIX];
    stream.read_exact(&mut prefix)?;
    let length = payload_length(prefix, MAX_REPLY).map_err(io::Error::other)?;
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload)?;
    let message = decode_payload(&payload).map_err(io::Error::other)?;
    Ok((message, (LENGTH_PREFIX + length) as u64))
}

fn write_frame(stream: &mut TcpStream, message: &ClientMessage) -> io::Result<u64> {
    let frame = encode_frame(message).map_err(io::Error::other)?;
    stream.write_all(&frame)?;
    Ok(frame.len() as u64)
}

fn open(config: &UplinkConfig) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&config.addr, config.connect_timeout)?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

impl Uplink {
    /// Connects, opens a session and starts the worker thread. Blocks for at most about
    /// twice `connect_timeout`.
    ///
    /// # Errors
    /// The server is unreachable, refuses the session, or answers something else.
    pub fn connect(config: UplinkConfig) -> Result<(Self, SessionInfo, EpochSeed), UplinkError> {
        let mut stream = open(&config).map_err(UplinkError::Io)?;
        stream
            .set_read_timeout(Some(config.connect_timeout))
            .map_err(UplinkError::Io)?;
        let sent = write_frame(
            &mut stream,
            &ClientMessage::Hello {
                client: config.client_name.clone(),
                label: config.label.clone(),
            },
        )
        .map_err(UplinkError::Io)?;
        let (reply, received) = read_message(&mut stream).map_err(UplinkError::Io)?;
        let (info, token, seed) = match reply {
            ServerMessage::Welcome {
                session_id,
                token,
                match_id,
                player_id,
                epoch_seed,
                amplitude_ppm,
                ..
            } => (
                SessionInfo {
                    session_id,
                    match_id,
                    player_id,
                    amplitude_ppm,
                },
                token,
                epoch_seed.0,
            ),
            ServerMessage::Error { code } => return Err(UplinkError::Refused(code)),
            _ => return Err(UplinkError::Unexpected),
        };
        stream.set_read_timeout(None).map_err(UplinkError::Io)?;
        let state = Arc::new(Mutex::new(State {
            status: UplinkStatus::Connected,
            stats: UplinkStats {
                bytes_sent: sent,
                bytes_received: received,
                ..UplinkStats::default()
            },
            verdict: None,
            last_error: None,
        }));
        let (tx, rx) = mpsc::channel();
        let mut worker = Worker {
            config,
            session_id: info.session_id,
            token,
            commands: rx,
            loopback: tx.clone(),
            state: Arc::clone(&state),
            stream: None,
            connection: 0,
            batch: Vec::new(),
            batch_started: None,
            unacked: VecDeque::new(),
            buffered_bytes: 0,
            next_seq: 0,
            finish_requested: false,
            finish_sent: false,
            disconnected_at: None,
            next_attempt: Instant::now(),
        };
        worker.attach(stream);
        let handle = std::thread::Builder::new()
            .name("rearguard-uplink".into())
            .spawn(move || worker.run())
            .map_err(UplinkError::Io)?;
        Ok((
            Self {
                commands: tx,
                state,
                worker: Some(handle),
            },
            info,
            seed,
        ))
    }

    /// Queues one record. Never blocks.
    pub fn record(&self, record: Record) {
        let _ = self.commands.send(Command::Record(record));
    }

    /// Sends what is queued, then ends the session. The final verdict arrives later
    /// ([`Self::verdict`], [`Self::status`]).
    pub fn finish(&self) {
        let _ = self.commands.send(Command::Finish);
    }

    /// Asks the server for the session's current verdict.
    pub fn request_verdict(&self) {
        let _ = self.commands.send(Command::Verdict);
    }

    /// The connection state.
    #[must_use]
    pub fn status(&self) -> UplinkStatus {
        self.state
            .lock()
            .map_or(UplinkStatus::Lost("internal".into()), |s| s.status.clone())
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> UplinkStats {
        self.state.lock().map(|s| s.stats).unwrap_or_default()
    }

    /// The latest verdict received (live while open, final after finishing).
    #[must_use]
    pub fn verdict(&self) -> Option<VerdictReport> {
        self.state.lock().ok().and_then(|s| s.verdict.clone())
    }

    /// The last error code the server sent, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<ErrorCode> {
        self.state.lock().ok().and_then(|s| s.last_error)
    }
}

impl Drop for Uplink {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

/// A sent chunk the server has not acknowledged yet.
struct Pending {
    seq: u64,
    message: ClientMessage,
    bytes: usize,
}

struct Worker {
    config: UplinkConfig,
    session_id: u64,
    token: SessionToken,
    commands: Receiver<Command>,
    loopback: Sender<Command>,
    state: Arc<Mutex<State>>,
    stream: Option<TcpStream>,
    /// Identifies the reader thread of the current connection, so a stale reader's
    /// failure is ignored.
    connection: u64,
    batch: Vec<WireRecord>,
    batch_started: Option<Instant>,
    unacked: VecDeque<Pending>,
    buffered_bytes: usize,
    next_seq: u64,
    finish_requested: bool,
    finish_sent: bool,
    disconnected_at: Option<Instant>,
    next_attempt: Instant,
}

impl Worker {
    fn with_state(&self, f: impl FnOnce(&mut State)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
        }
    }

    fn lost(&mut self) -> bool {
        self.state
            .lock()
            .map_or(true, |s| matches!(s.status, UplinkStatus::Lost(_)))
    }

    /// Takes ownership of a fresh connection and starts its reader thread.
    fn attach(&mut self, stream: TcpStream) {
        self.connection += 1;
        let connection = self.connection;
        if let Ok(mut reader) = stream.try_clone() {
            let tx = self.loopback.clone();
            let _ = std::thread::Builder::new()
                .name("rearguard-uplink-read".into())
                .spawn(move || {
                    loop {
                        match read_message(&mut reader) {
                            Ok((message, n)) => {
                                if tx.send(Command::Server(message, n)).is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                let _ = tx.send(Command::ReadFailed(connection));
                                break;
                            }
                        }
                    }
                });
        }
        self.stream = Some(stream);
        self.disconnected_at = None;
    }

    fn detach(&mut self) {
        if let Some(s) = self.stream.take() {
            let _ = s.shutdown(Shutdown::Both);
        }
        if self.disconnected_at.is_none() {
            self.disconnected_at = Some(Instant::now());
            self.next_attempt = Instant::now() + self.config.retry_interval;
        }
        self.with_state(|s| {
            if !matches!(s.status, UplinkStatus::Lost(_) | UplinkStatus::Finished) {
                s.status = UplinkStatus::Reconnecting;
            }
        });
    }

    fn give_up(&mut self, reason: &str) {
        if let Some(s) = self.stream.take() {
            let _ = s.shutdown(Shutdown::Both);
        }
        let dropped = self.batch.len() as u64
            + self
                .unacked
                .iter()
                .map(|p| match &p.message {
                    ClientMessage::Telemetry { records, .. } => records.len() as u64,
                    _ => 0,
                })
                .sum::<u64>();
        self.batch.clear();
        self.unacked.clear();
        self.buffered_bytes = 0;
        let reason = reason.to_owned();
        self.with_state(|s| {
            s.status = UplinkStatus::Lost(reason);
            s.stats.records_dropped += dropped;
        });
    }

    fn send(&mut self, message: &ClientMessage) -> bool {
        let Some(stream) = self.stream.as_mut() else {
            return false;
        };
        match write_frame(stream, message) {
            Ok(n) => {
                let chunk = matches!(message, ClientMessage::Telemetry { .. });
                self.with_state(|s| {
                    s.stats.bytes_sent += n;
                    if chunk {
                        s.stats.chunks_sent += 1;
                    }
                });
                true
            }
            Err(_) => {
                self.detach();
                false
            }
        }
    }

    fn flush_batch(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let records = std::mem::take(&mut self.batch);
        self.batch_started = None;
        let seq = self.next_seq;
        self.next_seq += 1;
        let message = ClientMessage::Telemetry {
            session_id: self.session_id,
            seq,
            records,
        };
        let bytes = encode_frame(&message).map_or(0, |f| f.len());
        self.buffered_bytes += bytes;
        if self.stream.is_some() {
            self.send(&message);
        }
        self.unacked.push_back(Pending {
            seq,
            message,
            bytes,
        });
        if self.buffered_bytes > self.config.max_buffered_bytes {
            self.give_up("telemetry buffer full while disconnected");
        }
    }

    fn try_finish(&mut self) {
        if self.finish_requested && !self.finish_sent && self.stream.is_some() {
            self.flush_batch();
            let message = ClientMessage::Finish {
                session_id: self.session_id,
                seq: self.next_seq,
            };
            if self.send(&message) {
                self.finish_sent = true;
                self.with_state(|s| s.status = UplinkStatus::Finishing);
            }
        }
    }

    fn on_server(&mut self, message: ServerMessage, bytes: u64) {
        self.with_state(|s| s.stats.bytes_received += bytes);
        match message {
            ServerMessage::Ack { seq, .. } => self.acked(seq),
            ServerMessage::Verdict(v) => {
                let last = v.status != SessionStatus::Open;
                self.with_state(|s| {
                    s.verdict = Some(v);
                    if last {
                        s.status = UplinkStatus::Finished;
                    }
                });
            }
            // A resend of a chunk the server already had: as good as an ack.
            ServerMessage::Error {
                code: ErrorCode::Replayed,
            } => {
                if let Some(p) = self.unacked.front() {
                    let seq = p.seq;
                    self.acked(seq);
                }
            }
            ServerMessage::Error {
                code: code @ (ErrorCode::UnknownSession | ErrorCode::BadToken),
            } => {
                self.with_state(|s| s.last_error = Some(code));
                self.give_up("the server no longer has this session");
            }
            ServerMessage::Error { code } => self.with_state(|s| s.last_error = Some(code)),
            ServerMessage::Resumed { .. } | ServerMessage::Welcome { .. } => {}
        }
    }

    fn acked(&mut self, seq: u64) {
        while let Some(front) = self.unacked.front() {
            if front.seq > seq {
                break;
            }
            self.buffered_bytes -= front.bytes;
            self.unacked.pop_front();
            self.with_state(|s| s.stats.chunks_acked += 1);
        }
    }

    /// One reconnection attempt: connect, resume, resend what the server lacks.
    fn reconnect(&mut self) {
        self.next_attempt = Instant::now() + self.config.retry_interval;
        let Ok(mut stream) = open(&self.config) else {
            return;
        };
        let _ = stream.set_read_timeout(Some(self.config.connect_timeout));
        let resume = ClientMessage::Resume {
            session_id: self.session_id,
            token: self.token.clone(),
        };
        let Ok(sent) = write_frame(&mut stream, &resume) else {
            return;
        };
        let Ok((reply, received)) = read_message(&mut stream) else {
            return;
        };
        self.with_state(|s| {
            s.stats.bytes_sent += sent;
            s.stats.bytes_received += received;
        });
        match reply {
            ServerMessage::Resumed { next_seq, .. } => {
                let _ = stream.set_read_timeout(None);
                // The server already has every chunk before `next_seq`.
                if next_seq > 0 {
                    self.acked(next_seq - 1);
                }
                self.attach(stream);
                self.with_state(|s| {
                    s.stats.resumes += 1;
                    s.status = UplinkStatus::Connected;
                });
                let resend: Vec<ClientMessage> =
                    self.unacked.iter().map(|p| p.message.clone()).collect();
                for message in resend {
                    if !self.send(&message) {
                        return;
                    }
                }
                if self.finish_sent {
                    // The Finish may not have arrived; send it again (a repeat is refused
                    // harmlessly as a replay or unknown session).
                    self.finish_sent = false;
                }
            }
            ServerMessage::Error { code } => {
                self.with_state(|s| s.last_error = Some(code));
                self.give_up("the server refused to resume the session");
            }
            _ => {}
        }
    }

    /// Handles one command; `false` means stop.
    fn handle(&mut self, command: Command) -> bool {
        let lost = self.lost();
        match command {
            Command::Shutdown => return false,
            Command::Record(r) => {
                if lost {
                    self.with_state(|s| s.stats.records_dropped += 1);
                } else {
                    self.with_state(|s| s.stats.records_queued += 1);
                    self.batch.push(WireRecord::from(r));
                    self.batch_started.get_or_insert_with(Instant::now);
                    if self.batch.len() >= self.config.chunk_records {
                        self.flush_batch();
                    }
                }
            }
            Command::Finish => self.finish_requested = true,
            Command::Verdict => {
                let request = ClientMessage::Verdict {
                    session_id: self.session_id,
                };
                if self.stream.is_some() && !lost {
                    self.send(&request);
                }
            }
            Command::Server(message, bytes) => self.on_server(message, bytes),
            Command::ReadFailed(connection) => {
                if connection == self.connection && !lost {
                    self.detach();
                }
            }
        }
        true
    }

    fn run(mut self) {
        'outer: loop {
            match self
                .commands
                .recv_timeout(self.config.flush_interval.min(self.config.retry_interval))
            {
                Ok(c) => {
                    if !self.handle(c) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // Take everything already queued before anything slow (a reconnection
            // attempt can block for `connect_timeout`, and on some systems a refused
            // loopback connection takes seconds). This keeps the buffer accounting and
            // its cap current, and the channel short.
            while let Ok(c) = self.commands.try_recv() {
                if !self.handle(c) {
                    break 'outer;
                }
            }
            if self.lost() {
                continue;
            }
            if self
                .batch_started
                .is_some_and(|t| t.elapsed() >= self.config.flush_interval)
            {
                self.flush_batch();
            }
            self.try_finish();
            if self.stream.is_none() {
                if self
                    .disconnected_at
                    .is_some_and(|t| t.elapsed() >= self.config.give_up_after)
                {
                    self.give_up("the server is unreachable");
                } else if Instant::now() >= self.next_attempt {
                    self.reconnect();
                }
            }
        }
        if let Some(s) = self.stream.take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}
