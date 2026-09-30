// SPDX-License-Identifier: MIT OR Apache-2.0

//! Client telemetry schema, version 1 (decision D2).
//!
//! A session is a stream of [`Record`]s, one JSON object per line (JSON Lines, UTF-8,
//! LF). The first record is a [`Header`]; the last is normally an [`End`]. Every record
//! has a `type` tag and the common timing fields `ts_us`, `frame` and `tick`.
//!
//! The server treats every field as untrusted: it recomputes view angles from the raw
//! deltas and its own copy of the drift, and compares.
//!
//! # Conventions
//!
//! - **Angles** are degrees as `f64`. Yaw positive turns left and is never wrapped;
//!   pitch positive looks up and is clamped to ±89°. At yaw 0, pitch 0 the view looks
//!   along -Z. (The same convention as the Godot aim range, `demo/README.md`.)
//! - **Raw deltas** are mouse counts before sensitivity and acceleration. Mouse right
//!   (+dx) turns right (yaw decreases), mouse down (+dy) looks down. They are whole
//!   numbers on Windows raw input and X11, but carried as `f64` because some input
//!   paths (native Wayland) deliver fractions.
//! - **View update.** One `move` changes the view by
//!   `(-dx, -dy) × deg_per_count × sensitivity multiplier`, and one `recoil` by
//!   `(kick_yaw, kick_pitch) × recoil multiplier`. Both multipliers are the input-probe
//!   drift ([`crate::probe`]) at the event's probe tick.
//! - **Probe tick** of an event: `(ts_us - probe_start_us) / 1000`, rounded down. It is
//!   not stored per event, so a client cannot claim a different one.
//! - **Floats** are finite and written at full round-trip precision; a correctly
//!   rounded parser (for example `serde_json` with `float_roundtrip`) reads them back
//!   bit for bit.
//! - **Unknown fields are rejected.** Any change to the schema bumps [`VERSION`].

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

/// Value of [`Header::format`].
pub const FORMAT: &str = "rearguard.telemetry";

/// Schema version, in [`Header::version`]. Bumped on any change to the records.
pub const VERSION: u32 = 1;

/// The probe tick of an event: `(ts_us - probe_start_us) / 1000`, rounded down; 0 for
/// an event before the probe start. Client and server both use this rule, so the tick
/// is never sent.
#[must_use]
pub fn probe_tick(ts_us: u64, probe_start_us: u64) -> u64 {
    ts_us.saturating_sub(probe_start_us) / 1_000
}

/// One telemetry record. Serialised with a `type` field naming the variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    /// First record of a session.
    Header(Header),
    /// One raw mouse motion event.
    Move(Move),
    /// Fire button pressed or released.
    Button(Button),
    /// One shot.
    Fire(Fire),
    /// Recoil applied after a shot.
    Recoil(Recoil),
    /// A new static target appeared.
    Target(Target),
    /// Last record of a session.
    End(End),
}

impl Record {
    /// The record's client timestamp, in microseconds.
    #[must_use]
    pub fn ts_us(&self) -> u64 {
        match self {
            Self::Header(r) => r.ts_us,
            Self::Move(r) => r.ts_us,
            Self::Button(r) => r.ts_us,
            Self::Fire(r) => r.ts_us,
            Self::Recoil(r) => r.ts_us,
            Self::Target(r) => r.ts_us,
            Self::End(r) => r.ts_us,
        }
    }
}

/// Session metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    /// Client timestamp: microseconds from a monotonic clock whose origin is
    /// arbitrary (for example, engine start). Only differences are meaningful.
    pub ts_us: u64,
    /// Render frame counter. Events with the same `frame` were delivered in one batch.
    pub frame: u64,
    /// Game physics tick at the time of the record.
    pub tick: u64,
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`VERSION`] for this schema.
    pub version: u32,
    /// Identifier of the match, as used for the probe key hierarchy
    /// ([`crate::probe::RootSeed::match_key`], UTF-8 bytes).
    pub match_id: String,
    /// Identifier of the player within the match ([`crate::probe::MatchKey::player_key`]).
    pub player_id: u64,
    /// What produced the stream: `"client"` for a real game client, `"sim"` for
    /// `rearguard-sim`. Never names a simulated player's class.
    pub source: String,
    /// `ts_us` at which probe tick 0 of epoch 0 falls.
    pub probe_start_us: u64,
    /// Degrees of view per raw count at a multiplier of exactly 1.
    pub deg_per_count: f64,
    /// Physics ticks per second.
    pub physics_hz: u32,
    /// Scenario name: `"flick"`, `"spray"` or `"tracking"`.
    pub scenario: String,
    /// Seed of the scenario's target sequence (not a secret).
    pub scenario_seed: u64,
    /// Planned session length in seconds of game time.
    pub duration_s: f64,
    /// Identifier of the fixed recoil pattern, for example `"v1"`.
    pub recoil_pattern: String,
    /// Distance from the eye to a target, in metres.
    pub target_distance_m: f64,
    /// Target sphere radius, in metres.
    pub target_radius_m: f64,
}

/// One raw relative mouse motion event and the view that resulted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Move {
    /// Client timestamp (see [`Header::ts_us`]). Non-decreasing through a session.
    pub ts_us: u64,
    /// Render frame in which the event was delivered.
    pub frame: u64,
    /// Physics tick at delivery.
    pub tick: u64,
    /// Raw horizontal counts, before sensitivity. +dx is mouse right.
    pub dx: f64,
    /// Raw vertical counts, before sensitivity. +dy is mouse down.
    pub dy: f64,
    /// Resulting yaw, degrees.
    pub yaw: f64,
    /// Resulting pitch, degrees.
    pub pitch: f64,
}

/// Fire button pressed or released.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Button {
    /// Client timestamp.
    pub ts_us: u64,
    /// Render frame.
    pub frame: u64,
    /// Physics tick.
    pub tick: u64,
    /// `true` on press, `false` on release.
    pub pressed: bool,
}

/// One shot. Fired on the press itself and then every fire interval while held.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fire {
    /// Client timestamp.
    pub ts_us: u64,
    /// Render frame.
    pub frame: u64,
    /// Physics tick.
    pub tick: u64,
    /// Session-wide shot index, from 0.
    pub shot: u64,
    /// Index of the shot within the current trigger hold, from 0.
    pub burst_shot: u32,
    /// View yaw when fired, before this shot's recoil.
    pub yaw: f64,
    /// View pitch when fired, before this shot's recoil.
    pub pitch: f64,
    /// Index of the current target.
    pub target: u64,
    /// Target yaw.
    pub target_yaw: f64,
    /// Target pitch.
    pub target_pitch: f64,
    /// Whether the shot hit, as judged by the client.
    pub hit: bool,
}

/// Recoil kick applied after a shot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recoil {
    /// Client timestamp.
    pub ts_us: u64,
    /// Render frame.
    pub frame: u64,
    /// Physics tick.
    pub tick: u64,
    /// The shot this kick follows.
    pub shot: u64,
    /// That shot's index within its trigger hold.
    pub burst_shot: u32,
    /// Nominal kick from the pattern, before the recoil multiplier, degrees.
    pub kick_yaw: f64,
    /// Nominal kick pitch, degrees. Positive kicks the view up.
    pub kick_pitch: f64,
    /// Yaw after the kick.
    pub yaw: f64,
    /// Pitch after the kick.
    pub pitch: f64,
}

/// A static target appeared (flick and spray scenarios).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// Client timestamp.
    pub ts_us: u64,
    /// Render frame.
    pub frame: u64,
    /// Physics tick.
    pub tick: u64,
    /// Target index, from 0.
    pub target: u64,
    /// Target yaw.
    pub target_yaw: f64,
    /// Target pitch.
    pub target_pitch: f64,
}

/// End of session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct End {
    /// Client timestamp.
    pub ts_us: u64,
    /// Render frame.
    pub frame: u64,
    /// Physics tick.
    pub tick: u64,
    /// Shots fired.
    pub shots: u64,
    /// Shots that hit.
    pub hits: u64,
    /// `false` if the session stopped before its planned duration.
    pub complete: bool,
}

/// Writes records as JSON Lines.
///
/// # Errors
/// I/O errors from `out`, or a non-finite float (which JSON cannot represent).
pub fn write_jsonl<W: Write>(mut out: W, records: &[Record]) -> io::Result<()> {
    for record in records {
        serde_json::to_writer(&mut out, record)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// Reads a JSON Lines session and checks its header's format and version.
///
/// # Errors
/// I/O errors, a malformed line (reported with its 1-based line number), a first
/// record that is not a header, or a header of another format or version.
pub fn read_jsonl<R: BufRead>(input: R) -> io::Result<Vec<Record>> {
    let mut records = Vec::new();
    for (i, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: Record = serde_json::from_str(&line).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("line {}: {e}", i + 1))
        })?;
        records.push(record);
    }
    match records.first() {
        Some(Record::Header(h)) if h.format == FORMAT && h.version == VERSION => Ok(records),
        Some(Record::Header(h)) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported telemetry {} version {} (expected {FORMAT} version {VERSION})",
                h.format, h.version
            ),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "telemetry must start with a header record",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            ts_us: 1_000,
            frame: 3,
            tick: 0,
            format: FORMAT.to_owned(),
            version: VERSION,
            match_id: "m-1".to_owned(),
            player_id: 7,
            source: "sim".to_owned(),
            probe_start_us: 1_000,
            deg_per_count: 0.022,
            physics_hz: 120,
            scenario: "spray".to_owned(),
            scenario_seed: u64::MAX,
            duration_s: 60.0,
            recoil_pattern: "v1".to_owned(),
            target_distance_m: 10.0,
            target_radius_m: 0.3,
        }
    }

    /// One record of every kind, with floats that a lossy parser gets wrong.
    fn sample() -> Vec<Record> {
        vec![
            Record::Header(header()),
            Record::Target(Target {
                ts_us: 1_000,
                frame: 3,
                tick: 0,
                target: 0,
                target_yaw: -12.345_678_901_234_567_f64.next_up(),
                target_pitch: 0.1 + 0.2,
            }),
            Record::Move(Move {
                ts_us: 9_337,
                frame: 4,
                tick: 1,
                dx: -3.0,
                dy: 0.5,
                yaw: 0.022 * 3.0,
                pitch: -0.0,
            }),
            Record::Move(Move {
                ts_us: 9_341,
                frame: 4,
                tick: 1,
                dx: 1e-300,
                dy: f64::MAX,
                yaw: f64::MIN_POSITIVE,
                pitch: 89.0_f64.next_down(),
            }),
            Record::Button(Button {
                ts_us: 17_670,
                frame: 5,
                tick: 2,
                pressed: true,
            }),
            Record::Fire(Fire {
                ts_us: 17_670,
                frame: 5,
                tick: 2,
                shot: 0,
                burst_shot: 0,
                yaw: 5e-324,
                pitch: 1.0 / 3.0,
                target: 0,
                target_yaw: -12.345_678_901_234_567_f64.next_up(),
                target_pitch: 0.1 + 0.2,
                hit: true,
            }),
            Record::Recoil(Recoil {
                ts_us: 17_670,
                frame: 5,
                tick: 2,
                shot: 0,
                burst_shot: 0,
                kick_yaw: 0.05,
                kick_pitch: 0.9,
                yaw: 0.05_f64.next_up().next_up(),
                pitch: 0.9 + 1.0 / 3.0,
            }),
            Record::Button(Button {
                ts_us: 99_000,
                frame: 15,
                tick: 12,
                pressed: false,
            }),
            Record::End(End {
                ts_us: 60_001_000,
                frame: 7_203,
                tick: 7_200,
                shots: 1,
                hits: 1,
                complete: true,
            }),
        ]
    }

    fn round_trip(records: &[Record]) -> Vec<Record> {
        let mut bytes = Vec::new();
        write_jsonl(&mut bytes, records).unwrap();
        read_jsonl(bytes.as_slice()).unwrap()
    }

    #[test]
    fn probe_tick_rounds_down_from_the_start() {
        assert_eq!(probe_tick(5_000_000, 5_000_000), 0);
        assert_eq!(probe_tick(5_000_999, 5_000_000), 0);
        assert_eq!(probe_tick(5_001_000, 5_000_000), 1);
        assert_eq!(probe_tick(4_000_000, 5_000_000), 0);
        assert_eq!(probe_tick(u64::MAX, 0), u64::MAX / 1_000);
    }

    #[test]
    fn round_trip_is_exact() {
        let records = sample();
        let back = round_trip(&records);
        assert_eq!(back, records);
        // PartialEq treats 0.0 == -0.0; check the bits of every float explicitly.
        let bits = |r: &[Record]| serde_json::to_string(r).unwrap();
        assert_eq!(bits(&back), bits(&records));
        let (Record::Move(a), Record::Move(b)) = (&back[2], &records[2]) else {
            unreachable!()
        };
        assert_eq!(a.pitch.to_bits(), b.pitch.to_bits());
    }

    #[test]
    fn writes_one_tagged_object_per_line() {
        let mut bytes = Vec::new();
        write_jsonl(&mut bytes, &sample()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), sample().len());
        assert!(text.ends_with('\n') && !text.contains('\r'));
        assert!(lines[0].starts_with(r#"{"type":"header","#), "{}", lines[0]);
        assert!(lines[0].contains(r#""format":"rearguard.telemetry","version":1"#));
        assert!(lines[2].starts_with(r#"{"type":"move","#), "{}", lines[2]);
    }

    #[test]
    fn rejects_other_versions_and_formats() {
        for (format, version) in [(FORMAT, VERSION + 1), ("rearguard.aimrange.recording", 1)] {
            let mut h = header();
            h.format = format.to_owned();
            h.version = version;
            let mut bytes = Vec::new();
            write_jsonl(&mut bytes, &[Record::Header(h)]).unwrap();
            assert!(read_jsonl(bytes.as_slice()).is_err(), "{format} v{version}");
        }
    }

    #[test]
    fn rejects_missing_header_unknown_fields_and_bad_lines() {
        let bad = [
            // No header.
            r#"{"type":"button","ts_us":1,"frame":1,"tick":1,"pressed":true}"#.to_owned(),
            // Unknown field on a record.
            {
                let mut s = serde_json::to_string(&Record::Header(header())).unwrap();
                s.insert_str(s.len() - 1, r#","extra":1"#);
                s
            },
            // Unknown record type.
            r#"{"type":"teleport","ts_us":1}"#.to_owned(),
            // Not JSON.
            "move 1 2".to_owned(),
        ];
        for text in bad {
            assert!(read_jsonl(text.as_bytes()).is_err(), "{text}");
        }
    }

    #[test]
    fn non_finite_floats_are_refused_on_write() {
        let mut h = header();
        h.deg_per_count = f64::NAN;
        let mut bytes = Vec::new();
        // serde_json writes NaN as null, which then fails to read as a number.
        let written = write_jsonl(&mut bytes, &[Record::Header(h)]);
        assert!(written.is_err() || read_jsonl(bytes.as_slice()).is_err());
    }
}
