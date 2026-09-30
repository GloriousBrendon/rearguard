//! Rearguard Godot binding: a thin gdext layer over `rearguard-core`.
//!
//! Two GDScript classes:
//!
//! - [`RearguardProbe`]: starts a probe session from seed material and answers "what is
//!   the sensitivity multiplier / recoil scale at this event timestamp?".
//! - [`RearguardRecorder`]: writes client telemetry in the `rearguard.telemetry` schema.
//!
//! The binding only converts between Godot and Rust types. All maths (the drift, the
//! probe-tick rule, serialisation) lives in `rearguard-core`. Seed material stays on the
//! Rust side, in types that redact themselves and are wiped on drop.
//!
//! This is the only crate allowed to depend on Godot (gdext, MPL-2.0; decision D10).
//! Its one `unsafe` is the `unsafe impl ExtensionLibrary` gdext requires for the entry
//! point.

mod convert;

use std::fs::File;
use std::io::{BufWriter, Write as _};

use godot::global::Error;
use godot::prelude::*;
use rearguard_core::probe::{Amplitude, EpochSeed, ProbeConfig, ProbeGenerator, Stream};
use rearguard_core::telemetry::{self, Record};

use crate::convert::{Field, SeedSource, header_from, load_or_create_seed, to_u32, to_u64};

struct RearguardExtension;

// SAFETY: gdext's entry point. The trait is `unsafe` because gdext cannot check that the
// library is loaded only by a compatible Godot; the `.gdextension` file pins
// `compatibility_minimum = 4.7`, matching the `api-4-7` feature this crate is built with.
#[gdextension]
unsafe impl ExtensionLibrary for RearguardExtension {}

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
/// probe.start_session_from_file(path, true, 5000, start_us)
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

    /// Starts a session from a 32-byte epoch seed, as a server would deliver it.
    /// `probe_start_us` is the client timestamp of probe tick 0. The bytes are copied
    /// into a wiped-on-drop secret; the caller's array cannot be wiped from here, so
    /// prefer [`Self::start_session_from_file`] or [`Self::start_session_random`].
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
    /// missing file is created with a fresh seed from the operating system's CSPRNG,
    /// readable only by its owner where the platform supports it. The seed never
    /// passes through GDScript.
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

    /// Starts a session from a fresh random seed held only in memory: the drift is
    /// applied but can never be analysed afterwards.
    #[func]
    fn start_session_random(&mut self, amplitude_ppm: i64, probe_start_us: i64) -> Error {
        match load_or_create_seed(&SeedSource::Random, true) {
            Ok(seed) => self.start(&seed, amplitude_ppm, probe_start_us),
            Err(e) => e.to_godot(),
        }
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
    /// session). Multiply the view change of that event's raw delta by it.
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
        let Some(header) = header_from(field) else {
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
        let r = (|| {
            Some(Record::Move(telemetry::Move {
                ts_us: to_u64(ts_us)?,
                frame: to_u64(frame)?,
                tick: to_u64(tick)?,
                dx,
                dy,
                yaw,
                pitch,
            }))
        })();
        self.write(r)
    }

    /// Fire button pressed or released.
    #[func]
    fn record_button(&mut self, ts_us: i64, frame: i64, tick: i64, pressed: bool) -> bool {
        let r = (|| {
            Some(Record::Button(telemetry::Button {
                ts_us: to_u64(ts_us)?,
                frame: to_u64(frame)?,
                tick: to_u64(tick)?,
                pressed,
            }))
        })();
        self.write(r)
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
        let r = (|| {
            Some(Record::Fire(telemetry::Fire {
                ts_us: to_u64(ts_us)?,
                frame: to_u64(frame)?,
                tick: to_u64(tick)?,
                shot: to_u64(shot)?,
                burst_shot: to_u32(burst_shot)?,
                yaw,
                pitch,
                target: to_u64(target)?,
                target_yaw,
                target_pitch,
                hit,
            }))
        })();
        self.write(r)
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
        let r = (|| {
            Some(Record::Recoil(telemetry::Recoil {
                ts_us: to_u64(ts_us)?,
                frame: to_u64(frame)?,
                tick: to_u64(tick)?,
                shot: to_u64(shot)?,
                burst_shot: to_u32(burst_shot)?,
                kick_yaw,
                kick_pitch,
                yaw,
                pitch,
            }))
        })();
        self.write(r)
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
        let r = (|| {
            Some(Record::Target(telemetry::Target {
                ts_us: to_u64(ts_us)?,
                frame: to_u64(frame)?,
                tick: to_u64(tick)?,
                target: to_u64(target)?,
                target_yaw,
                target_pitch,
            }))
        })();
        self.write(r)
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
        let r = (|| {
            Some(Record::End(telemetry::End {
                ts_us: to_u64(ts_us)?,
                frame: to_u64(frame)?,
                tick: to_u64(tick)?,
                shots: to_u64(shots)?,
                hits: to_u64(hits)?,
                complete,
            }))
        })();
        self.write(r)
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
