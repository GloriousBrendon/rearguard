// SPDX-License-Identifier: MIT OR Apache-2.0

//! Recorded sessions: human-study exports (task 1.9) and test-bot recordings from the
//! aim range (task 1.8). Each usable session is replayed through the detector with its
//! drift seed re-derived from the key the facilitator or the test server holds.
//!
//! A session is used only if the detector's replay reproduces every reported view angle
//! (`angle_mismatches == 0`): that is what shows the key, the amplitude and the
//! telemetry belong together. Everything left out is counted, with the reason.
//!
//! Keys are read into `RootSeed`, which redacts and wipes itself; nothing derived from
//! a key is written anywhere.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rearguard_core::probe::RootSeed;
use rearguard_core::study::session_match_id;
use rearguard_core::telemetry::{Record, read_jsonl};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::config::{EvalConfig, SCENARIOS};
use crate::trace::{Traced, trace};

/// Sessions left out, counted by source, reason, and whether leaving them out is by
/// design (a drift-off round) or a problem (a file that does not replay).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally(pub BTreeMap<(&'static str, &'static str, bool), u64>);

impl Tally {
    fn add(&mut self, source: &'static str, reason: &'static str, by_design: bool) {
        *self.0.entry((source, reason, by_design)).or_default() += 1;
    }

    /// Sessions left out because something is wrong with them.
    #[must_use]
    pub fn problems(&self) -> u64 {
        self.0.iter().filter(|(k, _)| !k.2).map(|(_, n)| n).sum()
    }
}

/// What was read from one kind of input.
#[derive(Debug, Default)]
pub struct Loaded {
    /// Usable sessions, traced.
    pub sessions: Vec<Traced>,
    /// Sessions left out.
    pub excluded: Tally,
    /// Exports or recordings read.
    pub units: usize,
    /// SHA-256 of everything read, in a fixed order, so a result file identifies its
    /// inputs without naming participants or paths.
    pub sha256: String,
    /// Exports that are not from people: automated self-test runs or debug builds.
    pub test_exports: usize,
}

/// Reads a key file (64 hex digits): a study key or a server master secret.
///
/// # Errors
/// An unreadable file, or one that is not 64 hex digits. The message names the file
/// and nothing of its contents.
pub fn load_root(path: &Path) -> Result<RootSeed, String> {
    let mut text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = RootSeed::from_hex(&text);
    text.zeroize();
    root.ok_or_else(|| format!("{}: not a key file (64 hex digits)", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Length-prefixed, so two files never hash like one.
fn absorb(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn close_to(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn records_of(bytes: &[u8]) -> Option<Vec<Record>> {
    read_jsonl(bytes).ok().filter(|r| !r.is_empty())
}

/// The fields of an export's `manifest.json` the evaluation reads. Godot writes whole
/// numbers either as integers or as floats, so numbers are read as floats.
#[derive(Deserialize)]
struct Manifest {
    format: String,
    version: f64,
    study_id: String,
    automated: bool,
    participant_id: String,
    input: ManifestInput,
    sessions: Vec<ManifestSession>,
}

#[derive(Deserialize)]
struct ManifestInput {
    debug_build: bool,
}

#[derive(Deserialize)]
struct ManifestSession {
    label: String,
    block: String,
    amplitude_ppm: f64,
    scenario: String,
    duration_s: f64,
    file: String,
    complete: bool,
    probe_applied: bool,
}

type Files = BTreeMap<String, Vec<u8>>;

/// An extracted export: `manifest.json` and `sessions/*.jsonl`.
fn read_export_dir(dir: &Path) -> Result<Files, String> {
    let read = |path: PathBuf| fs::read(&path).map_err(|e| format!("{}: {e}", path.display()));
    let mut files = Files::new();
    files.insert("manifest.json".to_owned(), read(dir.join("manifest.json"))?);
    if let Ok(entries) = fs::read_dir(dir.join("sessions")) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str().filter(|n| n.ends_with(".jsonl")) {
                files.insert(format!("sessions/{name}"), read(entry.path())?);
            }
        }
    }
    Ok(files)
}

/// Every export under `paths`: each path is an export zip, an extracted export, or a
/// directory holding either.
fn exports(paths: &[PathBuf]) -> Result<Vec<Files>, String> {
    let zip = |path: &Path| -> Result<Files, String> {
        let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        crate::zip::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    };
    let is_zip = |path: &Path| {
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    };
    let mut out = Vec::new();
    for path in paths {
        if path.is_file() {
            out.push(zip(path)?);
        } else if path.join("manifest.json").is_file() {
            out.push(read_export_dir(path)?);
        } else {
            let entries = fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            children.sort();
            let before = out.len();
            for child in children {
                if child.is_file() && is_zip(&child) {
                    out.push(zip(&child)?);
                } else if child.join("manifest.json").is_file() {
                    out.push(read_export_dir(&child)?);
                }
            }
            if out.len() == before {
                return Err(format!("{}: no study exports found", path.display()));
            }
        }
    }
    Ok(out)
}

/// Reads study exports and traces their baseline sessions with the drift on.
///
/// Each session's seed is re-derived from a study key (`keys`; a release has its own,
/// so several may be given): match `study:<study>:<participant>:<label>`, player 0,
/// epoch 0. The key that reproduces the session's view angles is the right one.
///
/// # Errors
/// An unreadable or malformed export, the same participant twice, or an export that is
/// not from a person (an automated self-test run or a debug build) unless
/// `allow_test_exports` is set.
pub fn load_humans(
    paths: &[PathBuf],
    keys: &[RootSeed],
    config: &EvalConfig,
    allow_test_exports: bool,
) -> Result<Loaded, String> {
    let detector = config.detector.config(None);
    let mut parsed = Vec::new();
    for files in exports(paths)? {
        let manifest = files
            .get("manifest.json")
            .ok_or("an export has no manifest.json")?;
        let manifest: Manifest = serde_json::from_slice(manifest)
            .map_err(|e| format!("an export's manifest.json: {e}"))?;
        if manifest.format != "rearguard.study.export" || !close_to(manifest.version, 1.0) {
            return Err("an export is not rearguard.study.export version 1".to_owned());
        }
        parsed.push((
            format!("{}/{}", manifest.study_id, manifest.participant_id),
            manifest,
            files,
        ));
    }
    // By participant, so neither the result nor the digest depends on how the exports
    // were named or listed.
    parsed.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(pair) = parsed.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(format!("participant {} appears in two exports", pair[0].0));
    }

    let mut out = Loaded {
        units: parsed.len(),
        ..Loaded::default()
    };
    let mut hash = Sha256::new();
    for (participant, manifest, files) in &parsed {
        if manifest.automated || manifest.input.debug_build {
            if !allow_test_exports {
                return Err(format!(
                    "the export of {participant} is not study data (an automated self-test run or a debug build); \
                     --allow-test-exports evaluates it anyway, marked as such"
                ));
            }
            out.test_exports += 1;
        }
        absorb(&mut hash, &files["manifest.json"]);
        for s in &manifest.sessions {
            let mut skip = |reason, by_design| out.excluded.add("human", reason, by_design);
            if s.block != "baseline" {
                skip("blind-comparison interval (not a baseline session)", true);
                continue;
            }
            if s.amplitude_ppm == 0.0 {
                skip(
                    "baseline session with the drift off (no drift to follow, so no score)",
                    true,
                );
                continue;
            }
            if !SCENARIOS.contains(&s.scenario.as_str()) {
                skip("scenario not evaluated", true);
                continue;
            }
            if !(s.complete && s.probe_applied) {
                skip("session incomplete or played without the probe", false);
                continue;
            }
            if !close_to(s.duration_s, config.horizon_s) {
                skip("planned length differs from horizon_s", false);
                continue;
            }
            // The manifest is the participant machine's claim; the file name is fixed
            // by the label, so nothing outside the export can be named.
            let label_ok = !s.label.is_empty()
                && s.label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            let Some(bytes) = files
                .get(&s.file)
                .filter(|_| label_ok && s.file == format!("sessions/{}.jsonl", s.label))
            else {
                skip("telemetry file missing", false);
                continue;
            };
            absorb(&mut hash, bytes);
            let Some(records) = records_of(bytes) else {
                skip("telemetry does not parse", false);
                continue;
            };
            let Some(Record::Header(header)) = records.first() else {
                skip("telemetry does not parse", false);
                continue;
            };
            let match_id = session_match_id(&manifest.study_id, &manifest.participant_id, &s.label);
            if header.match_id != match_id
                || header.player_id != 0
                || header.scenario != s.scenario
                || !close_to(header.duration_s, s.duration_s)
            {
                skip("telemetry header disagrees with the manifest", false);
                continue;
            }
            let amplitude_ppm = s.amplitude_ppm as u32;
            let replayed = keys.iter().find_map(|key| {
                let epoch = key
                    .match_key(match_id.as_bytes())
                    .player_key(0)
                    .epoch_seed(0);
                trace(&records, &epoch, amplitude_ppm, &detector)
                    .ok()
                    .filter(|t| t.shots > 0 && t.angle_mismatches == 0)
            });
            match replayed {
                Some(trace) => out.sessions.push(Traced {
                    source: "human",
                    class: "human".to_owned(),
                    scenario: s.scenario.clone(),
                    amplitude_ppm,
                    participant: participant.clone(),
                    trace,
                }),
                None => skip(
                    "replay does not reproduce the view (wrong study key, altered data, or no shots)",
                    false,
                ),
            }
        }
    }
    out.sha256 = hex(&hash.finalize());
    Ok(out)
}

/// The fields of a recording's `.env.json` the evaluation reads.
#[derive(Deserialize)]
struct Environment {
    ground_truth: Option<String>,
    #[serde(default)]
    probe_enabled: bool,
    #[serde(default)]
    probe_amplitude_ppm: f64,
    #[serde(default)]
    probe_seed: String,
}

/// Reads test-bot recordings from the aim range: every `NAME.jsonl` in `dir` with its
/// `NAME.jsonl.env.json`, which holds the ground-truth label (`cheat:<name>` or
/// `bot:scripted`) and the amplitude. The bots play under seeds issued by a local
/// `rearguard-server`; `root` is that server's master secret, from which each
/// session's seed is re-derived (match and player from the telemetry header, epoch 0).
///
/// # Errors
/// An unreadable directory, or one without recordings.
pub fn load_bots(dir: &Path, root: &RootSeed, config: &EvalConfig) -> Result<Loaded, String> {
    let detector = config.detector.config(None);
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let names: BTreeSet<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    if names.is_empty() {
        return Err(format!("{}: no recordings (*.jsonl) found", dir.display()));
    }
    let mut out = Loaded {
        units: names.len(),
        ..Loaded::default()
    };
    let mut hash = Sha256::new();
    for path in names {
        let mut skip = |reason, by_design| out.excluded.add("bot", reason, by_design);
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        absorb(&mut hash, &bytes);
        let mut env_path = path.clone().into_os_string();
        env_path.push(".env.json");
        let Ok(env_bytes) = fs::read(&env_path) else {
            skip("no .env.json beside the recording", false);
            continue;
        };
        absorb(&mut hash, &env_bytes);
        let Ok(env) = serde_json::from_slice::<Environment>(&env_bytes) else {
            skip(".env.json does not parse", false);
            continue;
        };
        let class = match env.ground_truth.as_deref() {
            Some("bot:scripted") => "scripted-bot".to_owned(),
            Some(label) => match label.strip_prefix("cheat:") {
                // A plain name only: it goes into CSV and Markdown as it is.
                Some(name)
                    if !name.is_empty()
                        && name.len() <= 40
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
                {
                    name.to_owned()
                }
                _ => {
                    skip("unknown ground-truth label", false);
                    continue;
                }
            },
            None => {
                skip("no ground-truth label (not a test-bot run)", false);
                continue;
            }
        };
        if !env.probe_enabled || env.probe_seed != "server" || env.probe_amplitude_ppm <= 0.0 {
            skip("not played under a server-issued drift", false);
            continue;
        }
        let Some(records) = records_of(&bytes) else {
            skip("telemetry does not parse", false);
            continue;
        };
        let Some(Record::Header(header)) = records.first() else {
            skip("telemetry does not parse", false);
            continue;
        };
        if !SCENARIOS.contains(&header.scenario.as_str()) {
            skip("scenario not evaluated", true);
            continue;
        }
        if !close_to(header.duration_s, config.horizon_s) {
            skip("planned length differs from horizon_s", false);
            continue;
        }
        let epoch = root
            .match_key(header.match_id.as_bytes())
            .player_key(header.player_id)
            .epoch_seed(0);
        let amplitude_ppm = env.probe_amplitude_ppm as u32;
        match trace(&records, &epoch, amplitude_ppm, &detector) {
            Ok(trace) if trace.shots > 0 && trace.angle_mismatches == 0 => {
                out.sessions.push(Traced {
                    source: "bot",
                    class,
                    scenario: header.scenario.clone(),
                    amplitude_ppm,
                    participant: String::new(),
                    trace,
                })
            }
            _ => skip(
                "replay does not reproduce the view (wrong master secret, altered data, or no shots)",
                false,
            ),
        }
    }
    out.sha256 = hex(&hash.finalize());
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use rearguard_core::probe::Amplitude;
    use rearguard_core::telemetry::write_jsonl;
    use rearguard_sim::seed::SimSeed;
    use rearguard_sim::session::{Class, ModelParams, SessionSpec, run_session_as};
    use rearguard_sim::world::Scenario;

    use super::*;
    use crate::config::tests::small;

    pub(crate) const KEY_HEX: &str =
        "5a17c0de5a17c0de5a17c0de5a17c0de5a17c0de5a17c0de5a17c0de5a17c0de";
    const OTHER_KEY_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/xtask-tests")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn key(hex: &str) -> RootSeed {
        RootSeed::from_hex(hex).unwrap()
    }

    /// Telemetry of a simulated player (`class`, `scenario`, session `index`, amplitude
    /// in ppm, length in seconds) under the seed `root` gives `match_id` and `player`,
    /// as the aim range would record it.
    fn telemetry(
        root: &RootSeed,
        match_id: &str,
        player: u64,
        (class, scenario, index, ppm, duration_s): (Class, Scenario, u32, u32, f64),
    ) -> Vec<u8> {
        let epoch = root
            .match_key(match_id.as_bytes())
            .player_key(player)
            .epoch_seed(0);
        let spec = SessionSpec {
            class,
            scenario,
            index,
            duration_s,
            amplitude: Amplitude::from_ppm(ppm).unwrap(),
            models: ModelParams::DEFAULT,
        };
        let session = run_session_as(&SimSeed::new(5), &epoch, match_id, player, spec);
        let mut bytes = Vec::new();
        write_jsonl(&mut bytes, &session.records).unwrap();
        bytes
    }

    /// One participant's export in the format of `demo/scripts/study.gd`, played by the
    /// simulator's human model: two flick and one spray baseline sessions with the
    /// drift on, one with it off, and a blind interval.
    pub(crate) fn export(participant: &str, index: u32, automated: bool) -> Vec<(String, Vec<u8>)> {
        let root = key(KEY_HEX);
        let plan = [
            (
                "baseline-01",
                "baseline",
                "flick",
                Scenario::Flick,
                20_000,
                10.0,
            ),
            (
                "baseline-02",
                "baseline",
                "spray",
                Scenario::Spray,
                20_000,
                10.0,
            ),
            ("baseline-03", "baseline", "flick", Scenario::Flick, 0, 10.0),
            (
                "baseline-04",
                "baseline",
                "flick",
                Scenario::Flick,
                20_000,
                10.0,
            ),
            ("trial-01-A", "blind", "flick", Scenario::Flick, 20_000, 4.0),
        ];
        let mut files = Vec::new();
        let mut sessions = Vec::new();
        for (i, (label, block, scenario, sim, ppm, duration)) in plan.into_iter().enumerate() {
            let match_id = session_match_id("test-study", participant, label);
            let player = (Class::Human, sim, index * 10 + i as u32, ppm, duration);
            let bytes = telemetry(&root, &match_id, 0, player);
            files.push((format!("sessions/{label}.jsonl"), bytes));
            // Whole numbers as Godot's JSON writes them: floats for some fields.
            sessions.push(format!(
                r#"{{"label": "{label}", "block": "{block}", "condition": "x", "amplitude_ppm": {ppm}, "scenario": "{scenario}",
                "scenario_seed": 1, "duration_s": {duration:?}, "file": "sessions/{label}.jsonl", "complete": true,
                "probe_applied": true, "shots": 1, "hits": 1, "trial": null, "interval": ""}}"#
            ));
        }
        let manifest = format!(
            r#"{{"format": "rearguard.study.export", "version": 1, "study_id": "test-study", "protocol_version": 1,
            "consent_version": "v1", "release": "r1", "automated": {automated}, "participant_id": "{participant}",
            "randomisation_seed": "1", "input": {{"debug_build": false, "os_name": "Linux"}},
            "sessions": [{}], "trials": []}}"#,
            sessions.join(",")
        );
        files.insert(0, ("manifest.json".to_owned(), manifest.into_bytes()));
        files
    }

    pub(crate) fn write_export_zip(
        dir: &Path,
        participant: &str,
        index: u32,
        automated: bool,
    ) -> PathBuf {
        let files = export(participant, index, automated);
        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let path = dir.join(format!("rearguard-study-test-study-{participant}.zip"));
        fs::write(&path, crate::zip::write(&borrowed, true)).unwrap();
        path
    }

    /// Test-bot recordings as the aim range writes them, under a server master secret.
    pub(crate) fn write_bot_recordings(dir: &Path) {
        let root = key(KEY_HEX);
        let runs = [
            (
                "flick-aimbot-1",
                Some("cheat:flick-aimbot"),
                Class::FlickAimbot,
                Scenario::Flick,
                10.0,
            ),
            (
                "flick-aimbot-2",
                Some("cheat:flick-aimbot"),
                Class::FlickAimbot,
                Scenario::Flick,
                10.0,
            ),
            (
                "recoil-macro-1",
                Some("cheat:recoil-macro"),
                Class::RecoilMacro,
                Scenario::Spray,
                10.0,
            ),
            (
                "scripted-1",
                Some("bot:scripted"),
                Class::Human,
                Scenario::Flick,
                10.0,
            ),
            (
                "short-1",
                Some("cheat:flick-aimbot"),
                Class::FlickAimbot,
                Scenario::Flick,
                5.0,
            ),
            ("unlabelled-1", None, Class::Human, Scenario::Flick, 10.0),
        ];
        for (i, (name, label, class, scenario, duration)) in runs.into_iter().enumerate() {
            let match_id = format!("m-{i:016x}");
            let player = (class, scenario, i as u32, 20_000, duration);
            let bytes = telemetry(&root, &match_id, 1, player);
            fs::write(dir.join(format!("{name}.jsonl")), bytes).unwrap();
            let label = label.map_or("null".to_owned(), |l| format!("\"{l}\""));
            let env = format!(
                r#"{{"source": "bot", "ground_truth": {label}, "probe_enabled": true, "probe_amplitude_ppm": 20000,
                "probe_seed": "server", "server_status": "connected", "debug_build": true}}"#
            );
            fs::write(dir.join(format!("{name}.jsonl.env.json")), env).unwrap();
        }
    }

    #[test]
    fn exports_load_from_zips_and_from_extracted_folders_alike() {
        let config = small();
        let dir = temp_dir("inputs-exports");
        let zips = dir.join("zips");
        fs::create_dir_all(&zips).unwrap();
        for (i, p) in ["P-AAAA", "P-BBBB"].into_iter().enumerate() {
            write_export_zip(&zips, p, i as u32, false);
            let folder = dir.join("folders").join(p);
            fs::create_dir_all(folder.join("sessions")).unwrap();
            for (name, bytes) in export(p, i as u32, false) {
                fs::write(folder.join(name), bytes).unwrap();
            }
        }
        // The right key may be any of those given.
        let keys = [key(OTHER_KEY_HEX), key(KEY_HEX)];
        let from_zips = load_humans(std::slice::from_ref(&zips), &keys, &config, false).unwrap();
        let from_folders = load_humans(&[dir.join("folders")], &keys, &config, false).unwrap();
        assert_eq!(from_zips.sessions, from_folders.sessions);
        assert_eq!(from_zips.sha256, from_folders.sha256);
        assert_eq!(from_zips.excluded, from_folders.excluded);
        assert_eq!((from_zips.units, from_zips.test_exports), (2, 0));

        // Per participant: three baseline sessions with the drift on are used; the
        // drift-off round and the blind interval are left out by design.
        assert_eq!(from_zips.sessions.len(), 6);
        assert!(
            from_zips
                .sessions
                .iter()
                .all(|s| s.trace.angle_mismatches == 0 && s.class == "human")
        );
        assert_eq!(
            from_zips
                .sessions
                .iter()
                .filter(|s| s.scenario == "spray")
                .count(),
            2
        );
        assert_eq!(from_zips.sessions[0].participant, "test-study/P-AAAA");
        assert_eq!(from_zips.excluded.problems(), 0);
        assert_eq!(from_zips.excluded.0.values().sum::<u64>(), 4);

        // Listing single files, in the other order, changes nothing.
        let listed = [
            zips.join("rearguard-study-test-study-P-BBBB.zip"),
            zips.join("rearguard-study-test-study-P-AAAA.zip"),
        ];
        let again = load_humans(&listed, &keys, &config, false).unwrap();
        assert_eq!(
            (again.sessions, again.sha256),
            (from_zips.sessions, from_zips.sha256)
        );
    }

    #[test]
    fn the_wrong_key_and_damaged_exports_are_left_out_or_refused() {
        let config = small();
        let dir = temp_dir("inputs-bad");
        let path = write_export_zip(&dir, "P-AAAA", 0, false);

        // Wrong key: nothing replays, and that is a problem, not a quiet skip.
        let wrong = load_humans(
            std::slice::from_ref(&path),
            &[key(OTHER_KEY_HEX)],
            &config,
            false,
        )
        .unwrap();
        assert!(wrong.sessions.is_empty());
        assert_eq!(wrong.excluded.problems(), 3);

        // Another horizon: the sessions are left out, and counted.
        let mut longer = small();
        longer.horizon_s = 60.0;
        let other =
            load_humans(std::slice::from_ref(&path), &[key(KEY_HEX)], &longer, false).unwrap();
        assert!(other.sessions.is_empty() && other.excluded.problems() == 3);

        // The same participant twice.
        let copy = dir.join("copy.zip");
        fs::copy(&path, &copy).unwrap();
        let twice = load_humans(&[path.clone(), copy], &[key(KEY_HEX)], &config, false);
        assert!(twice.unwrap_err().contains("two exports"));

        // A manifest pointing outside the export, a missing file, a corrupt archive.
        let mut files = export("P-CCCC", 2, false);
        let manifest = String::from_utf8(files[0].1.clone()).unwrap();
        files[0].1 = manifest
            .replacen("sessions/baseline-01.jsonl", "../../baseline-01.jsonl", 1)
            .into_bytes();
        files.retain(|(name, _)| name != "sessions/baseline-02.jsonl");
        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let odd = dir.join("odd.zip");
        fs::write(&odd, crate::zip::write(&borrowed, false)).unwrap();
        let loaded = load_humans(&[odd], &[key(KEY_HEX)], &config, false).unwrap();
        assert_eq!((loaded.sessions.len(), loaded.excluded.problems()), (1, 2));
        let broken = dir.join("broken.zip");
        fs::write(&broken, b"PK not really").unwrap();
        assert!(load_humans(&[broken], &[key(KEY_HEX)], &config, false).is_err());
        assert!(load_humans(&[dir.join("nowhere")], &[key(KEY_HEX)], &config, false).is_err());
    }

    #[test]
    fn automated_exports_are_refused_unless_allowed() {
        let config = small();
        let dir = temp_dir("inputs-automated");
        let path = write_export_zip(&dir, "P-AUTO", 0, true);
        let refused =
            load_humans(std::slice::from_ref(&path), &[key(KEY_HEX)], &config, false).unwrap_err();
        assert!(refused.contains("--allow-test-exports"), "{refused}");
        let allowed = load_humans(&[path], &[key(KEY_HEX)], &config, true).unwrap();
        assert_eq!((allowed.sessions.len(), allowed.test_exports), (3, 1));
    }

    #[test]
    fn bot_recordings_load_with_their_labels() {
        let config = small();
        let dir = temp_dir("inputs-bots");
        write_bot_recordings(&dir);
        let bots = load_bots(&dir, &key(KEY_HEX), &config).unwrap();
        let classes: Vec<&str> = bots.sessions.iter().map(|s| s.class.as_str()).collect();
        assert_eq!(
            classes,
            [
                "flick-aimbot",
                "flick-aimbot",
                "recoil-macro",
                "scripted-bot"
            ]
        );
        assert!(
            bots.sessions
                .iter()
                .all(|s| s.source == "bot" && s.amplitude_ppm == 20_000)
        );
        assert_eq!(bots.sessions[2].scenario, "spray");
        // The short run and the unlabelled one are left out, as problems.
        assert_eq!((bots.units, bots.excluded.problems()), (6, 2));
        // The wrong master secret replays nothing.
        let wrong = load_bots(&dir, &key(OTHER_KEY_HEX), &config).unwrap();
        assert!(wrong.sessions.is_empty() && wrong.excluded.problems() == 6);
        assert!(load_bots(&dir.join("none"), &key(KEY_HEX), &config).is_err());
    }

    #[test]
    fn key_files_are_read_and_errors_do_not_quote_them() {
        let dir = temp_dir("inputs-keys");
        let good = dir.join("key.hex");
        fs::write(&good, format!("{KEY_HEX}\n")).unwrap();
        assert!(load_root(&good).is_ok());
        let bad = dir.join("bad.hex");
        fs::write(&bad, &KEY_HEX[..40]).unwrap();
        let message = load_root(&bad).unwrap_err();
        assert!(!message.contains(&KEY_HEX[..8]), "{message}");
        assert!(load_root(&dir.join("missing.hex")).is_err());
    }
}
