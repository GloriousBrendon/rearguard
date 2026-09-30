//! Client–server wire protocol, version 1.
//!
//! A connection carries frames in both directions. A frame is
//! `length: u32 little-endian` followed by `length` payload bytes. The payload is one
//! [`PROTOCOL_VERSION`] byte, then one message encoded with postcard (serde).
//!
//! The client opens a session with [`ClientMessage::Hello`]. The server answers
//! [`ServerMessage::Welcome`] with the session's epoch seed (decision D5: one epoch
//! covering the match in v0). Telemetry then flows in [`ClientMessage::Telemetry`]
//! chunks, numbered `seq = 0, 1, 2, ...` per session; the server rejects a repeated or
//! skipped number, which stops replays. [`ClientMessage::Resume`] re-attaches a session
//! after a dropped connection. [`ClientMessage::Finish`] ends it and returns the
//! verdict.
//!
//! There is no TLS or authentication yet (task 3.3): servers listen on loopback only.

use core::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::detect::Evidence;
use crate::probe::EpochSeed;
use crate::secret::SECRET_KEY_LEN;
use crate::telemetry::{self, Record};

/// First payload byte of every frame.
pub const PROTOCOL_VERSION: u8 = 1;

/// Bytes in a frame's length prefix.
pub const LENGTH_PREFIX: usize = 4;

/// Why a frame or payload was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolError {
    /// The payload is empty or cannot be decoded as a message.
    Malformed,
    /// The frame is longer than the receiver accepts.
    Oversized,
    /// The version byte is not [`PROTOCOL_VERSION`].
    UnsupportedVersion,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed message",
            Self::Oversized => "message too large",
            Self::UnsupportedVersion => "unsupported protocol version",
        })
    }
}

impl std::error::Error for ProtocolError {}

/// The payload length announced by a frame prefix, checked against `max_payload`
/// before anything is read or allocated.
///
/// # Errors
/// [`ProtocolError::Malformed`] for an empty payload, [`ProtocolError::Oversized`] above
/// `max_payload`.
pub fn payload_length(
    prefix: [u8; LENGTH_PREFIX],
    max_payload: usize,
) -> Result<usize, ProtocolError> {
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 {
        Err(ProtocolError::Malformed)
    } else if length > max_payload {
        Err(ProtocolError::Oversized)
    } else {
        Ok(length)
    }
}

/// Encodes one message as a complete frame (prefix, version byte, postcard body).
///
/// # Errors
/// [`ProtocolError::Oversized`] if the payload exceeds `u32::MAX` bytes;
/// [`ProtocolError::Malformed`] if serde refuses the value.
pub fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolError> {
    let body = postcard::to_allocvec(message).map_err(|_| ProtocolError::Malformed)?;
    let length = u32::try_from(body.len() + 1).map_err(|_| ProtocolError::Oversized)?;
    let mut frame = Vec::with_capacity(LENGTH_PREFIX + 1 + body.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.push(PROTOCOL_VERSION);
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Decodes a payload (the bytes after the length prefix). Every byte must belong to the
/// message: trailing bytes are malformed.
///
/// # Errors
/// [`ProtocolError::UnsupportedVersion`] or [`ProtocolError::Malformed`].
pub fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, ProtocolError> {
    let (&version, body) = payload.split_first().ok_or(ProtocolError::Malformed)?;
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }
    match postcard::take_from_bytes::<T>(body) {
        Ok((message, [])) => Ok(message),
        _ => Err(ProtocolError::Malformed),
    }
}

/// A telemetry record on the wire. Mirrors [`telemetry::Record`], whose JSON form uses a
/// `type` tag that postcard cannot encode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireRecord {
    /// [`telemetry::Header`].
    Header(telemetry::Header),
    /// [`telemetry::Move`].
    Move(telemetry::Move),
    /// [`telemetry::Button`].
    Button(telemetry::Button),
    /// [`telemetry::Fire`].
    Fire(telemetry::Fire),
    /// [`telemetry::Recoil`].
    Recoil(telemetry::Recoil),
    /// [`telemetry::Target`].
    Target(telemetry::Target),
    /// [`telemetry::End`].
    End(telemetry::End),
}

impl From<Record> for WireRecord {
    fn from(r: Record) -> Self {
        match r {
            Record::Header(x) => Self::Header(x),
            Record::Move(x) => Self::Move(x),
            Record::Button(x) => Self::Button(x),
            Record::Fire(x) => Self::Fire(x),
            Record::Recoil(x) => Self::Recoil(x),
            Record::Target(x) => Self::Target(x),
            Record::End(x) => Self::End(x),
        }
    }
}

impl From<WireRecord> for Record {
    fn from(r: WireRecord) -> Self {
        match r {
            WireRecord::Header(x) => Self::Header(x),
            WireRecord::Move(x) => Self::Move(x),
            WireRecord::Button(x) => Self::Button(x),
            WireRecord::Fire(x) => Self::Fire(x),
            WireRecord::Recoil(x) => Self::Recoil(x),
            WireRecord::Target(x) => Self::Target(x),
            WireRecord::End(x) => Self::End(x),
        }
    }
}

/// An epoch seed in a [`ServerMessage::Welcome`]: 32 bytes on the wire, an [`EpochSeed`]
/// (redacted, wiped on drop) in memory. The only place a seed crosses the wire.
pub struct WireSeed(pub EpochSeed);

impl fmt::Debug for WireSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WireSeed(<redacted>)")
    }
}

impl Serialize for WireSeed {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.expose_secret().serialize(s)
    }
}

impl<'de> Deserialize<'de> for WireSeed {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut bytes = <[u8; SECRET_KEY_LEN]>::deserialize(d)?;
        Ok(Self(EpochSeed::from_bytes(&mut bytes)))
    }
}

/// Capability to resume a session after a dropped connection. Redacted in `Debug`,
/// compared in constant time.
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionToken(pub [u8; 16]);

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(<redacted>)")
    }
}

impl PartialEq for SessionToken {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(&other.0)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl Eq for SessionToken {}

/// Messages from a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Open a new session. `client` names the client software (for the record only).
    Hello {
        /// Client software name and version.
        client: String,
    },
    /// Re-attach a session whose connection dropped.
    Resume {
        /// The session.
        session_id: u64,
        /// The token from its `Welcome`.
        token: SessionToken,
    },
    /// The next chunk of telemetry, in stream order. The first chunk starts with the
    /// session's header record.
    Telemetry {
        /// The session.
        session_id: u64,
        /// Chunk number, from 0 with no gaps.
        seq: u64,
        /// Records.
        records: Vec<WireRecord>,
    },
    /// End the session; the server answers with the final verdict.
    Finish {
        /// The session.
        session_id: u64,
        /// The next chunk number (one past the last telemetry chunk).
        seq: u64,
    },
    /// Ask for a session's current or final verdict.
    Verdict {
        /// The session.
        session_id: u64,
    },
}

/// A session's lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Receiving telemetry.
    Open,
    /// Ended by the client with `Finish`.
    Finished,
    /// Ended by the server: the client disconnected and never resumed, or the server
    /// shut down.
    Abandoned,
}

impl SessionStatus {
    /// Lower-case name, as stored.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Finished => "finished",
            Self::Abandoned => "abandoned",
        }
    }

    /// Parses [`Self::name`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "open" => Some(Self::Open),
            "finished" => Some(Self::Finished),
            "abandoned" => Some(Self::Abandoned),
            _ => None,
        }
    }
}

/// One statistic's evidence, as reported (see [`crate::detect::Evidence`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSummary {
    /// Pairs used.
    pub pairs: u64,
    /// Pearson correlation.
    pub r: f64,
    /// Least-squares slope.
    pub slope: f64,
    /// Standard error of the slope.
    pub slope_se: f64,
    /// Residual standard deviation.
    pub residual_sd: f64,
    /// Slope per residual sd.
    pub kappa: f64,
    /// Test statistic against the null.
    pub z: f64,
    /// Signed log-likelihood-ratio score.
    pub score: f64,
    /// `Φ(z)`.
    pub confidence: f64,
    /// Whether the score reached the configured threshold.
    pub flagged: bool,
}

impl From<&Evidence> for EvidenceSummary {
    fn from(e: &Evidence) -> Self {
        Self {
            pairs: e.pairs,
            r: e.r,
            slope: e.slope,
            slope_se: e.slope_se,
            residual_sd: e.residual_sd,
            kappa: e.kappa,
            z: e.z,
            score: e.score,
            confidence: e.confidence,
            flagged: e.flagged,
        }
    }
}

/// A session's verdict: the headline score and the evidence behind it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerdictReport {
    /// The session.
    pub session_id: u64,
    /// Lifecycle state when the verdict was produced.
    pub status: SessionStatus,
    /// The higher of the two statistics' scores.
    pub score: f64,
    /// The higher of the two statistics' confidences.
    pub confidence: f64,
    /// Whether either statistic reached its threshold.
    pub flagged: bool,
    /// Telemetry records accepted.
    pub records: u64,
    /// Shots seen.
    pub shots: u64,
    /// Engagements seen.
    pub engagements: u64,
    /// Shots whose reported view disagreed with the server's replay.
    pub angle_mismatches: u64,
    /// Fire-time drift correlation.
    pub error: EvidenceSummary,
    /// Per-frame step response.
    pub steps: EvidenceSummary,
    /// Completed evidence windows.
    pub windows: u64,
    /// Completed windows in which either statistic was flagged.
    pub flagged_windows: u64,
}

/// Error codes a server sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The frame could not be decoded, was too large, or had another version.
    Protocol(ProtocolError),
    /// No such session, or it is no longer open.
    UnknownSession,
    /// The resume token does not match.
    BadToken,
    /// The session is attached to another connection.
    SessionBusy,
    /// A chunk number was already used: a replayed message.
    Replayed,
    /// A chunk number was skipped.
    OutOfOrder,
    /// The records are not valid telemetry (bad header, time going backwards,
    /// non-finite numbers, too many records).
    InvalidTelemetry,
    /// The connection exceeded its rate limit.
    RateLimited,
    /// The server has too many open sessions.
    ServerBusy,
    /// A message was sent that is not valid at this point (for example telemetry for a
    /// session this connection does not hold).
    NotAllowed,
    /// Something failed on the server.
    Internal,
}

/// Messages from the server.
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    /// A new session.
    Welcome {
        /// The session.
        session_id: u64,
        /// Capability for [`ClientMessage::Resume`].
        token: SessionToken,
        /// Match identifier (the probe key hierarchy's match).
        match_id: String,
        /// Player identifier within the match.
        player_id: u64,
        /// Epoch number of `epoch_seed` (always 0 in v0).
        epoch: u64,
        /// The session's probe seed.
        epoch_seed: WireSeed,
        /// Drift amplitude to apply, ppm.
        amplitude_ppm: u32,
    },
    /// A session was re-attached.
    Resumed {
        /// The session.
        session_id: u64,
        /// The next chunk number the server expects.
        next_seq: u64,
    },
    /// A telemetry chunk was accepted.
    Ack {
        /// The session.
        session_id: u64,
        /// The accepted chunk number.
        seq: u64,
    },
    /// A verdict.
    Verdict(VerdictReport),
    /// A request was refused. After a protocol or rate-limit error the server closes
    /// the connection.
    Error {
        /// What went wrong.
        code: ErrorCode,
    },
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::probe::RootSeed;
    use crate::telemetry::{Button, Fire, Move};

    const MAX: usize = 1 << 16;

    fn payload(frame: &[u8]) -> &[u8] {
        let length = payload_length(frame[..4].try_into().unwrap(), usize::MAX).unwrap();
        assert_eq!(frame.len(), LENGTH_PREFIX + length);
        &frame[LENGTH_PREFIX..]
    }

    fn records() -> Vec<WireRecord> {
        vec![
            WireRecord::Move(Move {
                ts_us: 1,
                frame: 2,
                tick: 3,
                dx: -4.0,
                dy: 0.5,
                yaw: 0.1 + 0.2,
                pitch: -0.0,
            }),
            WireRecord::Button(Button {
                ts_us: 5,
                frame: 2,
                tick: 3,
                pressed: true,
            }),
            WireRecord::Fire(Fire {
                ts_us: 5,
                frame: 2,
                tick: 3,
                shot: 0,
                burst_shot: 0,
                yaw: 1.0,
                pitch: 2.0,
                target: 9,
                target_yaw: 3.0,
                target_pitch: 4.0,
                hit: false,
            }),
        ]
    }

    #[test]
    fn client_messages_round_trip() {
        let messages = [
            ClientMessage::Hello {
                client: "sim/0.0.0".into(),
            },
            ClientMessage::Resume {
                session_id: 7,
                token: SessionToken([3; 16]),
            },
            ClientMessage::Telemetry {
                session_id: 7,
                seq: 2,
                records: records(),
            },
            ClientMessage::Finish {
                session_id: 7,
                seq: 3,
            },
            ClientMessage::Verdict {
                session_id: u64::MAX,
            },
        ];
        for m in messages {
            let frame = encode_frame(&m).unwrap();
            assert_eq!(frame[LENGTH_PREFIX], PROTOCOL_VERSION);
            assert_eq!(decode_payload::<ClientMessage>(payload(&frame)).unwrap(), m);
        }
    }

    #[test]
    fn welcome_carries_the_seed_but_never_shows_it() {
        let seed = RootSeed::from_bytes(&mut [8; 32])
            .match_key(b"m")
            .player_key(1)
            .epoch_seed(0);
        let bytes = *seed.expose_secret();
        let welcome = ServerMessage::Welcome {
            session_id: 1,
            token: SessionToken([1; 16]),
            match_id: "m".into(),
            player_id: 1,
            epoch: 0,
            epoch_seed: WireSeed(seed),
            amplitude_ppm: 5000,
        };
        let shown = format!("{welcome:?}");
        assert!(shown.contains("<redacted>"));
        assert!(
            !shown.contains(
                &format!("{:?}", &bytes[..4])
                    .trim_end_matches(']')
                    .to_owned()
            )
        );
        let frame = encode_frame(&welcome).unwrap();
        match decode_payload::<ServerMessage>(payload(&frame)).unwrap() {
            ServerMessage::Welcome { epoch_seed, .. } => {
                assert_eq!(epoch_seed.0.expose_secret(), &bytes)
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn payload_length_is_checked_before_reading() {
        assert_eq!(
            payload_length(0u32.to_le_bytes(), MAX),
            Err(ProtocolError::Malformed)
        );
        assert_eq!(
            payload_length((MAX as u32 + 1).to_le_bytes(), MAX),
            Err(ProtocolError::Oversized)
        );
        assert_eq!(
            payload_length(u32::MAX.to_le_bytes(), MAX),
            Err(ProtocolError::Oversized)
        );
        assert_eq!(payload_length(10u32.to_le_bytes(), MAX), Ok(10));
    }

    #[test]
    fn version_trailing_bytes_and_truncation_are_rejected() {
        let frame = encode_frame(&ClientMessage::Finish {
            session_id: 1,
            seq: 2,
        })
        .unwrap();
        let mut p = payload(&frame).to_vec();
        p[0] = 2;
        assert_eq!(
            decode_payload::<ClientMessage>(&p),
            Err(ProtocolError::UnsupportedVersion)
        );
        let mut p = payload(&frame).to_vec();
        p.push(0);
        assert_eq!(
            decode_payload::<ClientMessage>(&p),
            Err(ProtocolError::Malformed)
        );
        let p = payload(&frame);
        for cut in 0..p.len() {
            assert!(
                decode_payload::<ClientMessage>(&p[..cut]).is_err(),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn tokens_compare_by_value_and_hide_in_debug() {
        assert_eq!(SessionToken([1; 16]), SessionToken([1; 16]));
        assert_ne!(SessionToken([1; 16]), SessionToken([2; 16]));
        assert_eq!(
            format!("{:?}", SessionToken([0xAB; 16])),
            "SessionToken(<redacted>)"
        );
    }

    // Acceptance criterion 2: property tests for the parser.

    fn any_record() -> impl Strategy<Value = WireRecord> {
        let f = any::<f64>();
        prop_oneof![
            (any::<u64>(), any::<u64>(), f, f, f).prop_map(|(ts, frame, dx, dy, yaw)| {
                WireRecord::Move(Move {
                    ts_us: ts,
                    frame,
                    tick: 0,
                    dx,
                    dy,
                    yaw,
                    pitch: 0.0,
                })
            }),
            (any::<u64>(), any::<bool>()).prop_map(|(ts, pressed)| {
                WireRecord::Button(Button {
                    ts_us: ts,
                    frame: 0,
                    tick: 0,
                    pressed,
                })
            }),
        ]
    }

    fn any_client_message() -> impl Strategy<Value = ClientMessage> {
        prop_oneof![
            ".{0,40}".prop_map(|client| ClientMessage::Hello { client }),
            (any::<u64>(), any::<[u8; 16]>()).prop_map(|(session_id, t)| ClientMessage::Resume {
                session_id,
                token: SessionToken(t)
            }),
            (
                any::<u64>(),
                any::<u64>(),
                prop::collection::vec(any_record(), 0..20)
            )
                .prop_map(|(session_id, seq, records)| ClientMessage::Telemetry {
                    session_id,
                    seq,
                    records
                }),
            (any::<u64>(), any::<u64>())
                .prop_map(|(session_id, seq)| ClientMessage::Finish { session_id, seq }),
            any::<u64>().prop_map(|session_id| ClientMessage::Verdict { session_id }),
        ]
    }

    fn same_bits(a: &ClientMessage, b: &ClientMessage) -> bool {
        // NaN != NaN, so compare encodings, which are exact.
        postcard::to_allocvec(a).unwrap() == postcard::to_allocvec(b).unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
            let _ = decode_payload::<ClientMessage>(&bytes);
            let _ = decode_payload::<ServerMessage>(&bytes);
            if bytes.len() >= 4 {
                let _ = payload_length(bytes[..4].try_into().unwrap(), MAX);
            }
        }

        #[test]
        fn every_message_round_trips(m in any_client_message()) {
            let frame = encode_frame(&m).unwrap();
            let back: ClientMessage = decode_payload(payload(&frame)).unwrap();
            prop_assert!(same_bits(&m, &back));
        }

        #[test]
        fn corrupted_frames_are_rejected_or_decode_to_something(m in any_client_message(), at in any::<usize>(), byte in any::<u8>()) {
            let frame = encode_frame(&m).unwrap();
            let mut p = payload(&frame).to_vec();
            let i = at % p.len();
            p[i] = byte;
            // Must not panic; any result is acceptable.
            let _ = decode_payload::<ClientMessage>(&p);
        }

        #[test]
        fn truncated_payloads_are_rejected(m in any_client_message(), cut in any::<usize>()) {
            let frame = encode_frame(&m).unwrap();
            let p = payload(&frame);
            let cut = cut % p.len();
            prop_assert!(decode_payload::<ClientMessage>(&p[..cut]).is_err());
        }
    }
}
