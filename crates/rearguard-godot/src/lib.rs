// SPDX-License-Identifier: MIT OR Apache-2.0

//! Rearguard Godot binding: a thin gdext layer over `rearguard-core`.
//!
//! Three GDScript classes:
//!
//! - [`RearguardProbe`]: starts a probe session from seed material and answers "what is
//!   the sensitivity multiplier / recoil scale at this event timestamp?".
//! - [`RearguardRecorder`]: writes client telemetry in the `rearguard.telemetry` schema.
//! - [`RearguardClient`]: streams the same telemetry to a Rearguard server
//!   (`rearguard_core::uplink`) and hands the server-issued seed to a probe.
//!
//! The binding only converts between Godot and Rust types. All maths, serialisation and
//! networking live in `rearguard-core`. Seed material stays on the Rust side, in types
//! that redact themselves and are wiped on drop; it never reaches GDScript.
//!
//! Items marked "debug builds only" exist only when the library is built without
//! `--release` (the developer overlay's data).
//!
//! This is the only crate allowed to depend on Godot (gdext, MPL-2.0; decision D10).
//! Its one `unsafe` is the `unsafe impl ExtensionLibrary` gdext requires for the entry
//! point.

mod convert;

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::net::SocketAddr;

use godot::global::Error;
use godot::prelude::*;
use rearguard_core::probe::{Amplitude, EpochSeed, ProbeConfig, ProbeGenerator, Stream};
use rearguard_core::telemetry::{self, Record};
use rearguard_core::uplink::{SessionInfo, Uplink, UplinkConfig, UplinkStatus};

use crate::convert::{
    Field, SeedError, SeedSource, header_from, load_or_create_seed, load_root, records,
    root_from_bytes, to_u32, to_u64,
};

struct RearguardExtension;

// SAFETY: gdext's entry point. The trait is `unsafe` because gdext cannot check that the
// library is loaded only by a compatible Godot; the `.gdextension` file pins
// `compatibility_minimum = 4.7`, matching the `api-4-7` feature this crate is built with.
#[gdextension]
unsafe impl ExtensionLibrary for RearguardExtension {}

/// Loads a root key from a `res://` path, which may be inside an exported build's pack.
/// The bytes Godot returns are wiped after parsing.
fn load_packed_root(path: &GString) -> Result<rearguard_core::probe::RootSeed, SeedError> {
    if !godot::classes::FileAccess::file_exists(path) {
        return Err(SeedError::NotFound);
    }
    let mut bytes = godot::classes::FileAccess::get_file_as_bytes(path);
    if bytes.is_empty() {
        return Err(SeedError::Io);
    }
    root_from_bytes(bytes.as_mut_slice())
}

/// One active probe session.
struct Session {
    probe: ProbeGenerator,
    amplitude_ppm: u32,
    probe_start_us: u64,
}

/// The input probe for one player (decision D5: one epoch covering the session).
///
/// ```gdscript
/// var probe := RearguardProbe.new()
/// probe.start_session_from_client(client, start_us)   # or from a seed file
/// var k := probe.sensitivity_multiplier(event_ts_us)
/// ```
#[derive(GodotClass)]
#[class(base = RefCounted, init)]
pub struct RearguardProbe {
    session: Option<Session>,
}

impl RearguardProbe {
    fn start(&mut self, seed: &EpochSeed, amplitude_ppm: i64, probe_start_us: i64) -> Error {
        let (Some(ppm), Some(start)) = (to_u32(amplitude_ppm), to_u64(probe_start_us)) else {
            return Error::ERR_INVALID_PARAMETER;
        };
        let Ok(amplitude) = Amplitude::from_ppm(ppm) else {
            return Error::ERR_INVALID_PARAMETER;
        };
        let config = ProbeConfig {
            amplitude,
            ..ProbeConfig::default()
        };
        match ProbeGenerator::new(seed, &config) {
            Ok(probe) => {
                self.session = Some(Session {
                    probe,
                    amplitude_ppm: ppm,
                    probe_start_us: start,
                });
                Error::OK
            }
            Err(_) => Error::ERR_INVALID_PARAMETER,
        }
    }

    fn multiplier_at(&mut self, stream: Stream, tick: u64) -> f64 {
        self.session
            .as_mut()
            .map_or(1.0, |s| s.probe.drift(stream, tick).multiplier())
    }

    fn multiplier_at_ts(&mut self, stream: Stream, ts_us: i64) -> f64 {
        let Some(start) = self.session.as_ref().map(|s| s.probe_start_us) else {
            return 1.0;
        };
        let ts = to_u64(ts_us).unwrap_or(0);
        self.multiplier_at(stream, telemetry::probe_tick(ts, start))
    }
}

fn stream_from(stream: i64) -> Option<Stream> {
    match stream {
        0 => Some(Stream::Sensitivity),
        1 => Some(Stream::Recoil),
        _ => None,
    }
}

#[godot_api]
impl RearguardProbe {
    /// Stream index of the sensitivity drift, for the tick-based functions.
    #[constant]
    const STREAM_SENSITIVITY: i64 = 0;
    /// Stream index of the recoil drift.
    #[constant]
    const STREAM_RECOIL: i64 = 1;

    /// Starts a session from a 32-byte epoch seed. The bytes are copied into a
    /// wiped-on-drop secret; the caller's array cannot be wiped from here, so prefer
    /// the file, random or client sources.
    #[func]
    fn start_session(
        &mut self,
        epoch_seed: PackedByteArray,
        amplitude_ppm: i64,
        probe_start_us: i64,
    ) -> Error {
        let Ok(mut bytes) = <[u8; 32]>::try_from(epoch_seed.as_slice()) else {
            return Error::ERR_INVALID_DATA;
        };
        let seed = EpochSeed::from_bytes(&mut bytes);
        self.start(&seed, amplitude_ppm, probe_start_us)
    }

    /// Starts a session from a seed file (64 hex digits). With `create_if_missing`, a
    /// missing file is created with a fresh seed from the operating system's CSPRNG.
    #[func]
    fn start_session_from_file(
        &mut self,
        path: GString,
        create_if_missing: bool,
        amplitude_ppm: i64,
        probe_start_us: i64,
    ) -> Error {
        match load_or_create_seed(&SeedSource::File(path.to_string()), create_if_missing) {
            Ok(seed) => self.start(&seed, amplitude_ppm, probe_start_us),
            Err(e) => e.to_godot(),
        }
    }

    /// Starts a session from a fresh random seed held only in memory.
    #[func]
    fn start_session_random(&mut self, amplitude_ppm: i64, probe_start_us: i64) -> Error {
        match load_or_create_seed(&SeedSource::Random, true) {
            Ok(seed) => self.start(&seed, amplitude_ppm, probe_start_us),
            Err(e) => e.to_godot(),
        }
    }

    /// Starts a session whose seed is derived from a root key file (a study key, 64 hex
    /// digits) through the probe key hierarchy: `match_id` → `player_id` → epoch 0. The
    /// key and the seed stay inside Rust. Whoever holds the key file can re-derive the
    /// seed for analysis.
    ///
    /// `key_path` is a file-system path, or a `res://` path for a key packed into an
    /// exported build (read through Godot's `FileAccess`, still inside Rust).
    #[func]
    fn start_session_derived(
        &mut self,
        key_path: GString,
        match_id: GString,
        player_id: i64,
        amplitude_ppm: i64,
        probe_start_us: i64,
    ) -> Error {
        let Some(player) = to_u64(player_id) else {
            return Error::ERR_INVALID_PARAMETER;
        };
        let path = key_path.to_string();
        let root = if path.starts_with("res://") {
            load_packed_root(&key_path)
        } else {
            load_root(&path)
        };
        match root {
            Ok(root) => {
                let seed = root
                    .match_key(match_id.to_string().as_bytes())
                    .player_key(player)
                    .epoch_seed(0);
                self.start(&seed, amplitude_ppm, probe_start_us)
            }
            Err(e) => e.to_godot(),
        }
    }

    /// Starts a session with the seed and amplitude a connected [`RearguardClient`]
    /// received from the server. The seed moves from client to probe inside Rust; it can
    /// be taken once.
    #[func]
    fn start_session_from_client(
        &mut self,
        mut client: Gd<RearguardClient>,
        probe_start_us: i64,
    ) -> Error {
        let mut c = client.bind_mut();
        let (Some(seed), Some(amplitude)) = (
            c.seed.take(),
            c.info.as_ref().map(|i| i64::from(i.amplitude_ppm)),
        ) else {
            return Error::ERR_UNCONFIGURED;
        };
        drop(c);
        self.start(&seed, amplitude, probe_start_us)
    }

    /// Ends the session; the seed material is wiped.
    #[func]
    fn stop(&mut self) {
        self.session = None;
    }

    /// Whether a session is running.
    #[func]
    fn is_active(&self) -> bool {
        self.session.is_some()
    }

    /// The session's amplitude in ppm, or 0 without a session.
    #[func]
    fn amplitude_ppm(&self) -> i64 {
        self.session
            .as_ref()
            .map_or(0, |s| i64::from(s.amplitude_ppm))
    }

    /// The session's probe start timestamp, or 0 without a session.
    #[func]
    fn probe_start_us(&self) -> i64 {
        self.session
            .as_ref()
            .map_or(0, |s| i64::try_from(s.probe_start_us).unwrap_or(i64::MAX))
    }

    /// Sensitivity multiplier for an event at client timestamp `ts_us` (1.0 without a
    /// session).
    #[func]
    fn sensitivity_multiplier(&mut self, ts_us: i64) -> f64 {
        self.multiplier_at_ts(Stream::Sensitivity, ts_us)
    }

    /// Recoil scale for a shot at client timestamp `ts_us` (1.0 without a session).
    #[func]
    fn recoil_scale(&mut self, ts_us: i64) -> f64 {
        self.multiplier_at_ts(Stream::Recoil, ts_us)
    }

    /// Drift in parts per billion at a probe tick, for tests against the core golden
    /// vectors. Returns 0 without a session or for an invalid stream or negative tick.
    #[func]
    fn drift_ppb(&mut self, stream: i64, tick: i64) -> i64 {
        match (stream_from(stream), to_u64(tick), self.session.as_mut()) {
            (Some(stream), Some(tick), Some(s)) => i64::from(s.probe.drift(stream, tick).ppb()),
            _ => 0,
        }
    }

    /// Multiplier at a probe tick (1.0 without a session or for invalid arguments).
    #[func]
    fn multiplier_at_tick(&mut self, stream: i64, tick: i64) -> f64 {
        match (stream_from(stream), to_u64(tick)) {
            (Some(stream), Some(tick)) => self.multiplier_at(stream, tick),
            _ => 1.0,
        }
    }

    /// The default amplitude (0.5%) in ppm.
    #[func]
    fn default_amplitude_ppm() -> i64 {
        i64::from(Amplitude::DEFAULT.ppm())
    }

    /// The maximum amplitude (2%) in ppm.
    #[func]
    fn max_amplitude_ppm() -> i64 {
        i64::from(Amplitude::MAX.ppm())
    }
}

/// Builds a telemetry header from a GDScript dictionary.
fn header_from_dictionary(header: &VarDictionary) -> Option<telemetry::Header> {
    let field = |key: &str| -> Option<Field> {
        let v = header.get(&key.to_variant())?;
        match v.get_type() {
            VariantType::INT => Some(Field::Int(v.to::<i64>())),
            VariantType::FLOAT => Some(Field::Float(v.to::<f64>())),
            VariantType::STRING | VariantType::STRING_NAME => {
                Some(Field::Str(v.to::<GString>().to_string()))
            }
            _ => None,
        }
    };
    header_from(field)
}

/// Writes client telemetry (`rearguard.telemetry`, JSON Lines) with exact floats.
///
/// Open with a header dictionary, record events in order, then close. Every
/// `record_*` returns `false` (and the recorder stops writing) after an error or an
/// invalid argument (a negative count or timestamp).
#[derive(GodotClass)]
#[class(base = RefCounted, init)]
pub struct RearguardRecorder {
    out: Option<BufWriter<File>>,
    failed: bool,
}

impl RearguardRecorder {
    fn write(&mut self, record: Option<Record>) -> bool {
        if self.failed {
            return false;
        }
        let (Some(record), Some(out)) = (record, self.out.as_mut()) else {
            self.failed = true;
            return false;
        };
        if telemetry::write_jsonl(out, std::slice::from_ref(&record)).is_err() {
            self.failed = true;
            return false;
        }
        true
    }
}

#[godot_api]
impl RearguardRecorder {
    /// Opens (truncating) `path` and writes the header. `header` holds the
    /// [`telemetry::Header`] fields except `format` and `version`, which the recorder
    /// fills in.
    #[func]
    fn open(&mut self, path: GString, header: VarDictionary) -> Error {
        let Some(header) = header_from_dictionary(&header) else {
            return Error::ERR_INVALID_PARAMETER;
        };
        let file = match File::create(path.to_string()) {
            Ok(f) => f,
            Err(_) => return Error::ERR_FILE_CANT_WRITE,
        };
        self.out = Some(BufWriter::new(file));
        self.failed = false;
        if self.write(Some(Record::Header(header))) {
            Error::OK
        } else {
            Error::ERR_FILE_CANT_WRITE
        }
    }

    /// Whether the recorder is open and has not failed.
    #[func]
    fn is_open(&self) -> bool {
        self.out.is_some() && !self.failed
    }

    /// One raw mouse motion event and the view that resulted.
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn record_move(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        dx: f64,
        dy: f64,
        yaw: f64,
        pitch: f64,
    ) -> bool {
        self.write(records::movement(ts_us, frame, tick, dx, dy, yaw, pitch))
    }

    /// Fire button pressed or released.
    #[func]
    fn record_button(&mut self, ts_us: i64, frame: i64, tick: i64, pressed: bool) -> bool {
        self.write(records::button(ts_us, frame, tick, pressed))
    }

    /// One shot.
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn record_fire(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        shot: i64,
        burst_shot: i64,
        yaw: f64,
        pitch: f64,
        target: i64,
        target_yaw: f64,
        target_pitch: f64,
        hit: bool,
    ) -> bool {
        self.write(records::fire(
            ts_us,
            frame,
            tick,
            shot,
            burst_shot,
            yaw,
            pitch,
            target,
            target_yaw,
            target_pitch,
            hit,
        ))
    }

    /// Recoil kick applied after a shot (`kick_*` is the nominal pattern kick).
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn record_recoil(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        shot: i64,
        burst_shot: i64,
        kick_yaw: f64,
        kick_pitch: f64,
        yaw: f64,
        pitch: f64,
    ) -> bool {
        self.write(records::recoil(
            ts_us, frame, tick, shot, burst_shot, kick_yaw, kick_pitch, yaw, pitch,
        ))
    }

    /// A new static target appeared.
    #[func]
    fn record_target(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        target: i64,
        target_yaw: f64,
        target_pitch: f64,
    ) -> bool {
        self.write(records::target(
            ts_us,
            frame,
            tick,
            target,
            target_yaw,
            target_pitch,
        ))
    }

    /// End of session.
    #[func]
    fn record_end(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        shots: i64,
        hits: i64,
        complete: bool,
    ) -> bool {
        self.write(records::end(ts_us, frame, tick, shots, hits, complete))
    }

    /// Pushes buffered records to disk.
    #[func]
    fn flush(&mut self) -> Error {
        match self.out.as_mut().map(BufWriter::flush) {
            Some(Ok(())) => Error::OK,
            Some(Err(_)) => Error::ERR_FILE_CANT_WRITE,
            None => Error::ERR_UNCONFIGURED,
        }
    }

    /// Flushes and closes the file.
    #[func]
    fn close(&mut self) -> Error {
        let result = self.flush();
        self.out = None;
        result
    }
}

/// A live link to a Rearguard server (`rearguard_core::uplink`): opens a session,
/// streams telemetry from a background thread, and fetches the verdict. Recording
/// calls never block; see the uplink documentation for what happens when the server
/// goes away.
#[derive(GodotClass)]
#[class(base = RefCounted, init)]
pub struct RearguardClient {
    uplink: Option<Uplink>,
    info: Option<SessionInfo>,
    /// The server-issued seed, until a probe takes it.
    seed: Option<EpochSeed>,
}

impl RearguardClient {
    fn connect(&mut self, address: GString, client_name: GString, label: Option<String>) -> Error {
        let Ok(addr) = address.to_string().parse::<SocketAddr>() else {
            return Error::ERR_INVALID_PARAMETER;
        };
        let config = UplinkConfig {
            label,
            ..UplinkConfig::local(addr, &client_name.to_string())
        };
        match Uplink::connect(config) {
            Ok((uplink, info, seed)) => {
                self.uplink = Some(uplink);
                self.info = Some(info);
                self.seed = Some(seed);
                Error::OK
            }
            Err(_) => Error::ERR_CANT_CONNECT,
        }
    }

    fn send(&self, record: Option<Record>) -> bool {
        match (record, self.uplink.as_ref()) {
            (Some(r), Some(u)) => {
                u.record(r);
                true
            }
            _ => false,
        }
    }
}

#[godot_api]
impl RearguardClient {
    /// Connects to `address` (`host:port`, loopback), opens a session and starts
    /// streaming. Blocks for at most a few seconds.
    #[func]
    fn connect_to(&mut self, address: GString, client_name: GString) -> Error {
        self.connect(address, client_name, None)
    }

    /// Like `connect_to`, with a ground-truth `label` for the session (task 1.8's test
    /// bots, in Rearguard's own test environment only). The server stores it for the
    /// evaluation harness; the detector never reads it.
    #[func]
    fn connect_labelled(
        &mut self,
        address: GString,
        client_name: GString,
        label: GString,
    ) -> Error {
        self.connect(address, client_name, Some(label.to_string()))
    }

    /// `"not connected"`, `"connected"`, `"reconnecting"`, `"finishing"`, `"finished"`
    /// or `"lost: <reason>"`.
    #[func]
    fn status(&self) -> GString {
        match self.uplink.as_ref().map(Uplink::status) {
            None => "not connected".into(),
            Some(UplinkStatus::Connected) => "connected".into(),
            Some(UplinkStatus::Reconnecting) => "reconnecting".into(),
            Some(UplinkStatus::Finishing) => "finishing".into(),
            Some(UplinkStatus::Finished) => "finished".into(),
            Some(UplinkStatus::Lost(why)) => format!("lost: {why}").as_str().into(),
        }
    }

    /// The session identifier (0 before connecting).
    #[func]
    fn session_id(&self) -> i64 {
        self.info
            .as_ref()
            .map_or(0, |i| i64::try_from(i.session_id).unwrap_or(i64::MAX))
    }

    /// The match identifier, for the telemetry header.
    #[func]
    fn match_id(&self) -> GString {
        self.info
            .as_ref()
            .map_or_else(GString::new, |i| i.match_id.as_str().into())
    }

    /// The player identifier, for the telemetry header.
    #[func]
    fn player_id(&self) -> i64 {
        self.info
            .as_ref()
            .map_or(0, |i| i64::try_from(i.player_id).unwrap_or(i64::MAX))
    }

    /// The amplitude the server asked for, ppm.
    #[func]
    fn amplitude_ppm(&self) -> i64 {
        self.info.as_ref().map_or(0, |i| i64::from(i.amplitude_ppm))
    }

    /// Sends the session header (the first record). Same fields as
    /// [`RearguardRecorder::open`].
    #[func]
    fn record_header(&mut self, header: VarDictionary) -> bool {
        self.send(header_from_dictionary(&header).map(Record::Header))
    }

    /// See [`RearguardRecorder`].
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn record_move(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        dx: f64,
        dy: f64,
        yaw: f64,
        pitch: f64,
    ) -> bool {
        self.send(records::movement(ts_us, frame, tick, dx, dy, yaw, pitch))
    }

    /// See [`RearguardRecorder`].
    #[func]
    fn record_button(&mut self, ts_us: i64, frame: i64, tick: i64, pressed: bool) -> bool {
        self.send(records::button(ts_us, frame, tick, pressed))
    }

    /// See [`RearguardRecorder`].
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn record_fire(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        shot: i64,
        burst_shot: i64,
        yaw: f64,
        pitch: f64,
        target: i64,
        target_yaw: f64,
        target_pitch: f64,
        hit: bool,
    ) -> bool {
        self.send(records::fire(
            ts_us,
            frame,
            tick,
            shot,
            burst_shot,
            yaw,
            pitch,
            target,
            target_yaw,
            target_pitch,
            hit,
        ))
    }

    /// See [`RearguardRecorder`].
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn record_recoil(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        shot: i64,
        burst_shot: i64,
        kick_yaw: f64,
        kick_pitch: f64,
        yaw: f64,
        pitch: f64,
    ) -> bool {
        self.send(records::recoil(
            ts_us, frame, tick, shot, burst_shot, kick_yaw, kick_pitch, yaw, pitch,
        ))
    }

    /// See [`RearguardRecorder`].
    #[func]
    fn record_target(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        target: i64,
        target_yaw: f64,
        target_pitch: f64,
    ) -> bool {
        self.send(records::target(
            ts_us,
            frame,
            tick,
            target,
            target_yaw,
            target_pitch,
        ))
    }

    /// See [`RearguardRecorder`].
    #[func]
    fn record_end(
        &mut self,
        ts_us: i64,
        frame: i64,
        tick: i64,
        shots: i64,
        hits: i64,
        complete: bool,
    ) -> bool {
        self.send(records::end(ts_us, frame, tick, shots, hits, complete))
    }

    /// Sends what is queued and ends the session; the final verdict arrives later.
    #[func]
    fn finish(&mut self) {
        if let Some(u) = self.uplink.as_ref() {
            u.finish();
        }
    }

    /// Whether the final verdict has arrived (or the session was lost, so none will).
    #[func]
    fn is_done(&self) -> bool {
        self.uplink
            .as_ref()
            .is_none_or(|u| matches!(u.status(), UplinkStatus::Finished | UplinkStatus::Lost(_)))
    }

    /// Stops the uplink's worker and closes the connection.
    #[func]
    fn close(&mut self) {
        self.uplink = None;
        self.seed = None;
    }

    /// Asks for a live verdict (debug builds only, for the developer overlay).
    #[cfg(debug_assertions)]
    #[func]
    fn request_verdict(&mut self) {
        if let Some(u) = self.uplink.as_ref() {
            u.request_verdict();
        }
    }

    /// The latest verdict as a dictionary, empty if none (debug builds only).
    #[cfg(debug_assertions)]
    #[func]
    fn verdict(&self) -> VarDictionary {
        let mut d = VarDictionary::new();
        let Some(v) = self.uplink.as_ref().and_then(Uplink::verdict) else {
            return d;
        };
        let evidence = |e: &rearguard_core::protocol::EvidenceSummary| {
            let mut x = VarDictionary::new();
            x.set("pairs", i64::try_from(e.pairs).unwrap_or(i64::MAX));
            x.set("r", e.r);
            x.set("slope", e.slope);
            x.set("kappa", e.kappa);
            x.set("z", e.z);
            x.set("score", e.score);
            x.set("confidence", e.confidence);
            x.set("flagged", e.flagged);
            x
        };
        let count = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
        d.set("status", v.status.name());
        d.set("score", v.score);
        d.set("confidence", v.confidence);
        d.set("flagged", v.flagged);
        d.set("records", count(v.records));
        d.set("shots", count(v.shots));
        d.set("engagements", count(v.engagements));
        d.set("angle_mismatches", count(v.angle_mismatches));
        d.set("windows", count(v.windows));
        d.set("flagged_windows", count(v.flagged_windows));
        d.set("error", &evidence(&v.error));
        d.set("steps", &evidence(&v.steps));
        d
    }

    /// Uplink counters as a dictionary (debug builds only).
    #[cfg(debug_assertions)]
    #[func]
    fn stats(&self) -> VarDictionary {
        let mut d = VarDictionary::new();
        let s = self.uplink.as_ref().map(Uplink::stats).unwrap_or_default();
        let count = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
        d.set("bytes_sent", count(s.bytes_sent));
        d.set("bytes_received", count(s.bytes_received));
        d.set("records_queued", count(s.records_queued));
        d.set("chunks_sent", count(s.chunks_sent));
        d.set("chunks_acked", count(s.chunks_acked));
        d.set("resumes", count(s.resumes));
        d.set("records_dropped", count(s.records_dropped));
        d
    }
}

/// Human-study helpers (`rearguard_core::study`).
#[derive(GodotClass)]
#[class(base = RefCounted, init)]
pub struct RearguardStudy {}

#[godot_api]
impl RearguardStudy {
    /// A participant's plan as JSON, from the protocol JSON and a randomisation seed (a
    /// decimal `u64` string). On error, a JSON object `{"error": "..."}`.
    #[func]
    fn plan(protocol_json: GString, seed: GString) -> GString {
        let error = |m: String| serde_json::json!({ "error": m }).to_string();
        let text = match seed.to_string().trim().parse::<u64>() {
            Err(_) => error("the seed must be a decimal u64".into()),
            Ok(seed) => match serde_json::from_str::<rearguard_core::study::StudyProtocol>(
                &protocol_json.to_string(),
            ) {
                Err(e) => error(format!("protocol: {e}")),
                Ok(protocol) => match rearguard_core::study::plan(&protocol, seed) {
                    Ok(plan) => {
                        serde_json::to_string(&plan).unwrap_or_else(|e| error(e.to_string()))
                    }
                    Err(e) => error(e.to_string()),
                },
            },
        };
        text.as_str().into()
    }

    /// The match identifier that seeds a study session's drift.
    #[func]
    fn match_id(study_id: GString, participant_id: GString, label: GString) -> GString {
        rearguard_core::study::session_match_id(
            &study_id.to_string(),
            &participant_id.to_string(),
            &label.to_string(),
        )
        .as_str()
        .into()
    }
}
