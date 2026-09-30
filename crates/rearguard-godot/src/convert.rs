//! Conversions with no Godot types in them, so `cargo test` can check them without a
//! running engine (constructing a Godot `Variant` needs one).

use std::fs;
use std::io::{self, Write as _};

use rearguard_core::probe::EpochSeed;
use rearguard_core::secret::SECRET_KEY_LEN;
use rearguard_core::telemetry::{self, Header};
use zeroize::Zeroize;

/// Where seed material comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SeedSource {
    /// A seed file: 64 hexadecimal digits, surrounding whitespace ignored.
    File(String),
    /// A fresh seed from the operating system's CSPRNG, kept only in memory.
    Random,
}

/// Why seed material could not be loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeedError {
    /// The file does not exist and creating it was not allowed.
    NotFound,
    /// The file is not 64 hexadecimal digits.
    Invalid,
    /// Reading or creating the file failed.
    Io,
    /// The operating system's random source failed.
    Random,
}

impl SeedError {
    pub(crate) fn to_godot(self) -> godot::global::Error {
        use godot::global::Error;
        match self {
            Self::NotFound => Error::ERR_FILE_NOT_FOUND,
            Self::Invalid => Error::ERR_INVALID_DATA,
            Self::Io => Error::ERR_FILE_CANT_OPEN,
            Self::Random => Error::ERR_CANT_CREATE,
        }
    }
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decodes 64 hexadecimal digits into a seed. The input is not kept.
pub(crate) fn parse_seed_hex(text: &str) -> Result<EpochSeed, SeedError> {
    let digits = text.trim().as_bytes();
    if digits.len() != 2 * SECRET_KEY_LEN {
        return Err(SeedError::Invalid);
    }
    let mut bytes = [0u8; SECRET_KEY_LEN];
    for (i, pair) in digits.chunks_exact(2).enumerate() {
        match (hex_value(pair[0]), hex_value(pair[1])) {
            (Some(hi), Some(lo)) => bytes[i] = hi << 4 | lo,
            _ => {
                bytes.zeroize();
                return Err(SeedError::Invalid);
            }
        }
    }
    Ok(EpochSeed::from_bytes(&mut bytes))
}

fn random_seed() -> Result<[u8; SECRET_KEY_LEN], SeedError> {
    let mut bytes = [0u8; SECRET_KEY_LEN];
    getrandom::fill(&mut bytes).map_err(|_| SeedError::Random)?;
    Ok(bytes)
}

/// Creates `path` (it must not exist) holding `bytes` as hex, owner-only on Unix.
fn write_new_seed_file(path: &str, bytes: &[u8; SECRET_KEY_LEN]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let mut text = String::with_capacity(2 * SECRET_KEY_LEN + 1);
    for b in bytes {
        text.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
        text.push(char::from(b"0123456789abcdef"[usize::from(b & 0xf)]));
    }
    text.push('\n');
    let result = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    text.zeroize();
    result
}

/// Loads (or, if allowed, creates) the seed material from `source`.
pub(crate) fn load_or_create_seed(
    source: &SeedSource,
    create_if_missing: bool,
) -> Result<EpochSeed, SeedError> {
    match source {
        SeedSource::Random => {
            let mut bytes = random_seed()?;
            Ok(EpochSeed::from_bytes(&mut bytes))
        }
        SeedSource::File(path) => match fs::read_to_string(path) {
            Ok(mut text) => {
                let seed = parse_seed_hex(&text);
                text.zeroize();
                seed
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound && create_if_missing => {
                let mut bytes = random_seed()?;
                let written = write_new_seed_file(path, &bytes);
                let seed = EpochSeed::from_bytes(&mut bytes);
                written.map_err(|_| SeedError::Io)?;
                Ok(seed)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(SeedError::NotFound),
            Err(_) => Err(SeedError::Io),
        },
    }
}

/// Loads a root key (for example a study key) from a file of 64 hex digits. The file's
/// text is wiped from memory after parsing.
pub(crate) fn load_root(path: &str) -> Result<rearguard_core::probe::RootSeed, SeedError> {
    match fs::read(path) {
        Ok(mut bytes) => root_from_bytes(&mut bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(SeedError::NotFound),
        Err(_) => Err(SeedError::Io),
    }
}

/// Parses a root key from a key file's contents (64 hex digits, surrounding whitespace
/// ignored), then wipes the contents.
pub(crate) fn root_from_bytes(
    bytes: &mut [u8],
) -> Result<rearguard_core::probe::RootSeed, SeedError> {
    let root = std::str::from_utf8(bytes)
        .ok()
        .and_then(rearguard_core::probe::RootSeed::from_hex);
    bytes.zeroize();
    root.ok_or(SeedError::Invalid)
}

/// A non-negative GDScript integer as `u64`.
pub(crate) fn to_u64(v: i64) -> Option<u64> {
    u64::try_from(v).ok()
}

/// A GDScript integer as `u32`, if it fits.
pub(crate) fn to_u32(v: i64) -> Option<u32> {
    u32::try_from(v).ok()
}

/// A header field as GDScript supplies it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Field {
    Int(i64),
    Float(f64),
    Str(String),
}

/// Builds a telemetry header from named fields (`format` and `version` are filled in).
/// Integers are accepted where floats are expected, since GDScript dictionaries may
/// hold `60` for `60.0`.
pub(crate) fn header_from(get: impl Fn(&str) -> Option<Field>) -> Option<Header> {
    let int = |k: &str| match get(k)? {
        Field::Int(v) => to_u64(v),
        _ => None,
    };
    let float = |k: &str| match get(k)? {
        Field::Float(v) if v.is_finite() => Some(v),
        Field::Int(v) => Some(v as f64),
        _ => None,
    };
    let text = |k: &str| match get(k)? {
        Field::Str(v) => Some(v),
        _ => None,
    };
    Some(Header {
        ts_us: int("ts_us")?,
        frame: int("frame")?,
        tick: int("tick")?,
        format: telemetry::FORMAT.to_owned(),
        version: telemetry::VERSION,
        match_id: text("match_id")?,
        player_id: int("player_id")?,
        source: text("source")?,
        probe_start_us: int("probe_start_us")?,
        deg_per_count: float("deg_per_count")?,
        physics_hz: u32::try_from(int("physics_hz")?).ok()?,
        scenario: text("scenario")?,
        scenario_seed: int("scenario_seed")?,
        duration_s: float("duration_s")?,
        recoil_pattern: text("recoil_pattern")?,
        target_distance_m: float("target_distance_m")?,
        target_radius_m: float("target_radius_m")?,
    })
}

/// Builders for telemetry records from GDScript arguments. `None` for a negative count or
/// timestamp, or an index too large for its field.
pub(crate) mod records {
    use rearguard_core::telemetry::{Button, End, Fire, Move, Recoil, Record, Target};

    use super::{to_u32, to_u64};

    pub(crate) fn movement(
        ts_us: i64,
        frame: i64,
        tick: i64,
        dx: f64,
        dy: f64,
        yaw: f64,
        pitch: f64,
    ) -> Option<Record> {
        Some(Record::Move(Move {
            ts_us: to_u64(ts_us)?,
            frame: to_u64(frame)?,
            tick: to_u64(tick)?,
            dx,
            dy,
            yaw,
            pitch,
        }))
    }

    pub(crate) fn button(ts_us: i64, frame: i64, tick: i64, pressed: bool) -> Option<Record> {
        Some(Record::Button(Button {
            ts_us: to_u64(ts_us)?,
            frame: to_u64(frame)?,
            tick: to_u64(tick)?,
            pressed,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fire(
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
    ) -> Option<Record> {
        Some(Record::Fire(Fire {
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
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn recoil(
        ts_us: i64,
        frame: i64,
        tick: i64,
        shot: i64,
        burst_shot: i64,
        kick_yaw: f64,
        kick_pitch: f64,
        yaw: f64,
        pitch: f64,
    ) -> Option<Record> {
        Some(Record::Recoil(Recoil {
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
    }

    pub(crate) fn target(
        ts_us: i64,
        frame: i64,
        tick: i64,
        target: i64,
        target_yaw: f64,
        target_pitch: f64,
    ) -> Option<Record> {
        Some(Record::Target(Target {
            ts_us: to_u64(ts_us)?,
            frame: to_u64(frame)?,
            tick: to_u64(tick)?,
            target: to_u64(target)?,
            target_yaw,
            target_pitch,
        }))
    }

    pub(crate) fn end(
        ts_us: i64,
        frame: i64,
        tick: i64,
        shots: i64,
        hits: i64,
        complete: bool,
    ) -> Option<Record> {
        Some(Record::End(End {
            ts_us: to_u64(ts_us)?,
            frame: to_u64(frame)?,
            tick: to_u64(tick)?,
            shots: to_u64(shots)?,
            hits: to_u64(hits)?,
            complete,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rearguard_core::probe::{ProbeConfig, ProbeGenerator, RootSeed, Stream};

    use super::*;

    const HEX: &str = "90a6e8a8811eec0eea229215d0c8442573f50f9969b7ab748288ba03f90ad4f8";

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rearguard-godot-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn parses_hex_seeds_and_rejects_anything_else() {
        let seed = parse_seed_hex(&format!("  {HEX}\n")).unwrap();
        assert_eq!(seed.expose_secret()[0], 0x90);
        assert_eq!(seed.expose_secret()[31], 0xf8);
        assert!(parse_seed_hex(&HEX.to_uppercase()).is_ok());
        for bad in ["", &HEX[..62], &format!("{HEX}00"), &HEX.replace('9', "g")] {
            assert_eq!(parse_seed_hex(bad).err(), Some(SeedError::Invalid), "{bad}");
        }
    }

    #[test]
    fn seed_file_is_created_once_then_reused() {
        let path = temp_path("seed.hex");
        let path_str = path.to_string_lossy().to_string();
        let source = SeedSource::File(path_str.clone());
        assert_eq!(
            load_or_create_seed(&source, false).err(),
            Some(SeedError::NotFound)
        );
        let first = load_or_create_seed(&source, true).unwrap();
        let second = load_or_create_seed(&source, false).unwrap();
        assert_eq!(first.expose_secret(), second.expose_secret());
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.trim().len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // An existing file is never overwritten.
        let third = load_or_create_seed(&source, true).unwrap();
        assert_eq!(first.expose_secret(), third.expose_secret());
        fs::write(&path, "not hex").unwrap();
        assert_eq!(
            load_or_create_seed(&source, true).err(),
            Some(SeedError::Invalid)
        );
    }

    #[test]
    fn root_keys_load_from_hex_files_only() {
        let path = temp_path("study.hex");
        assert_eq!(
            load_root(&path.to_string_lossy()).err(),
            Some(SeedError::NotFound)
        );
        fs::write(&path, format!("{HEX}\n")).unwrap();
        let root = load_root(&path.to_string_lossy()).unwrap();
        let a = root.match_key(b"m").player_key(0).epoch_seed(0);
        let b = rearguard_core::probe::RootSeed::from_hex(HEX)
            .unwrap()
            .match_key(b"m")
            .player_key(0)
            .epoch_seed(0);
        assert_eq!(a.expose_secret(), b.expose_secret());
        fs::write(&path, "nope").unwrap();
        assert_eq!(
            load_root(&path.to_string_lossy()).err(),
            Some(SeedError::Invalid)
        );
    }

    #[test]
    fn root_keys_parse_from_bytes_which_are_then_wiped() {
        let mut bytes = format!("{HEX}\n").into_bytes();
        let root = root_from_bytes(&mut bytes).unwrap();
        assert!(bytes.iter().all(|&b| b == 0), "contents wiped");
        let expected = rearguard_core::probe::RootSeed::from_hex(HEX).unwrap();
        assert_eq!(
            root.match_key(b"m")
                .player_key(0)
                .epoch_seed(0)
                .expose_secret(),
            expected
                .match_key(b"m")
                .player_key(0)
                .epoch_seed(0)
                .expose_secret()
        );
        let mut bad = b"not a key".to_vec();
        assert_eq!(root_from_bytes(&mut bad).err(), Some(SeedError::Invalid));
        assert!(bad.iter().all(|&b| b == 0), "invalid contents wiped too");
        let mut not_utf8 = vec![0xff; 64];
        assert_eq!(
            root_from_bytes(&mut not_utf8).err(),
            Some(SeedError::Invalid)
        );
    }

    #[test]
    fn random_seeds_differ() {
        let a = load_or_create_seed(&SeedSource::Random, true).unwrap();
        let b = load_or_create_seed(&SeedSource::Random, true).unwrap();
        assert_ne!(a.expose_secret(), b.expose_secret());
    }

    #[test]
    fn record_builders_refuse_negative_fields() {
        assert!(records::movement(1, 2, 3, -1.0, 0.5, 0.0, 0.0).is_some());
        assert!(records::movement(-1, 2, 3, 0.0, 0.0, 0.0, 0.0).is_none());
        assert!(records::fire(1, 1, 1, 0, 1 << 33, 0.0, 0.0, 0, 0.0, 0.0, true).is_none());
        assert!(records::end(1, 1, 1, 3, -2, true).is_none());
        assert!(records::target(1, 1, 1, 4, 1.0, 2.0).is_some());
    }

    #[test]
    fn integer_conversions_refuse_negatives_and_overflow() {
        assert_eq!(to_u64(-1), None);
        assert_eq!(to_u64(i64::MAX), Some(i64::MAX as u64));
        assert_eq!(to_u32(1 << 32), None);
        assert_eq!(to_u32(20_000), Some(20_000));
    }

    fn fields() -> HashMap<&'static str, Field> {
        HashMap::from([
            ("ts_us", Field::Int(10)),
            ("frame", Field::Int(2)),
            ("tick", Field::Int(0)),
            ("match_id", Field::Str("aimrange-local".into())),
            ("player_id", Field::Int(0)),
            ("source", Field::Str("client".into())),
            ("probe_start_us", Field::Int(10)),
            ("deg_per_count", Field::Float(0.022)),
            ("physics_hz", Field::Int(120)),
            ("scenario", Field::Str("flick".into())),
            ("scenario_seed", Field::Int(7)),
            ("duration_s", Field::Int(60)),
            ("recoil_pattern", Field::Str("v1".into())),
            ("target_distance_m", Field::Float(10.0)),
            ("target_radius_m", Field::Float(0.3)),
        ])
    }

    #[test]
    fn header_is_built_from_named_fields() {
        let f = fields();
        let h = header_from(|k| f.get(k).cloned()).unwrap();
        assert_eq!(h.format, telemetry::FORMAT);
        assert_eq!(h.version, telemetry::VERSION);
        assert_eq!(h.duration_s, 60.0);
        assert_eq!(h.deg_per_count, 0.022);
        for missing in ["ts_us", "scenario", "deg_per_count"] {
            let mut f = fields();
            f.remove(missing);
            assert!(header_from(|k| f.get(k).cloned()).is_none(), "{missing}");
        }
        let mut f = fields();
        f.insert("frame", Field::Int(-1));
        assert!(header_from(|k| f.get(k).cloned()).is_none());
        f = fields();
        f.insert("deg_per_count", Field::Float(f64::NAN));
        assert!(header_from(|k| f.get(k).cloned()).is_none());
    }

    // The golden data the headless Godot test compares the extension with
    // (demo/tests/probe_golden.json) must be exactly the core golden vectors. This test
    // regenerates it from rearguard-core and compares byte for byte; regenerate with
    // `cargo test -p rearguard-godot print_probe_golden -- --ignored --nocapture`.

    const GOLDEN_TICKS: [u64; 10] = [
        0,
        1,
        250,
        499,
        500,
        777,
        12_345,
        1_000_000,
        86_400_000,
        1 << 40,
    ];

    fn golden_json() -> String {
        // The core golden seed: test root bytes 0..32, match "rearguard-golden",
        // player 42, epoch 0 (crates/rearguard-core/src/probe/tests.rs).
        let mut root = [0u8; 32];
        for (i, b) in root.iter_mut().enumerate() {
            *b = i as u8;
        }
        let seed = RootSeed::from_bytes(&mut root)
            .match_key(b"rearguard-golden")
            .player_key(42)
            .epoch_seed(0);
        let hex: String = seed
            .expose_secret()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut probe = ProbeGenerator::new(&seed, &ProbeConfig::default()).unwrap();
        let mut rows = Vec::new();
        for (index, stream) in [(0, Stream::Sensitivity), (1, Stream::Recoil)] {
            for tick in GOLDEN_TICKS {
                let d = probe.drift(stream, tick);
                rows.push(format!(
                    "    {{\"stream\": {index}, \"tick\": \"{tick}\", \"ppb\": {}, \"multiplier_bits\": \"{:016x}\"}}",
                    d.ppb(),
                    d.multiplier().to_bits()
                ));
            }
        }
        format!(
            "{{\n  \"note\": \"Core probe golden vectors (noise-default: amplitude 5000 ppm, default shape) for the headless Godot test. Test data from a public test seed, not a secret. Generated by rearguard-godot's print_probe_golden test.\",\n  \"epoch_seed_hex\": \"{hex}\",\n  \"amplitude_ppm\": 5000,\n  \"rows\": [\n{}\n  ]\n}}\n",
            rows.join(",\n")
        )
    }

    fn golden_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/tests/probe_golden.json")
    }

    #[test]
    #[ignore = "prints demo/tests/probe_golden.json"]
    fn print_probe_golden() {
        print!("{}", golden_json());
    }

    #[test]
    fn godot_golden_file_matches_core() {
        let committed = fs::read_to_string(golden_path())
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(committed, golden_json());
        // And the seed is the one the core golden test pins.
        assert!(committed.contains(HEX));
        // Spot-check two rows against the literal core golden table.
        assert!(committed.contains("\"stream\": 0, \"tick\": \"0\", \"ppb\": 3547080, \"multiplier_bits\": \"3ff00e8762098a6d\""));
        assert!(committed.contains("\"stream\": 1, \"tick\": \"1099511627776\", \"ppb\": 4201896, \"multiplier_bits\": \"3ff0113601de6b4a\""));
    }
}
