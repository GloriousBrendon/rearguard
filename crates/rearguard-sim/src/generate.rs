//! Generating a batch of sessions into a directory.
//!
//! Layout: `<dir>/<class>/<scenario>-<index>.jsonl` (telemetry, one file per session)
//! and `<dir>/manifest.json` (what each file is, including its class label). The
//! seed is not written anywhere: the same `--seed` and options reproduce every file
//! byte for byte.

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rearguard_core::probe::Amplitude;
use rearguard_core::telemetry;
use serde::Serialize;

use crate::seed::SimSeed;
use crate::session::{Class, ModelParams, SessionSpec, run_session};

/// Value of [`Manifest::format`].
pub const MANIFEST_FORMAT: &str = "rearguard.sim.manifest";
/// Value of [`Manifest::version`].
/// Version 2 (task 1.2a) added `smoothing` and `estimation_window_ms`.
pub const MANIFEST_VERSION: u32 = 2;

/// What to generate.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Sessions per (class, scenario) pair.
    pub sessions: u32,
    /// Session length, seconds of game time.
    pub duration_s: f64,
    /// Probe drift amplitude.
    pub amplitude: Amplitude,
    /// Classes to simulate.
    pub classes: Vec<Class>,
    /// Tuning of the configurable cheat models.
    pub models: ModelParams,
}

/// Describes a generated batch. Holds the class labels, so keep it away from anything
/// that is meant to judge sessions blind.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Manifest {
    /// Always [`MANIFEST_FORMAT`].
    pub format: String,
    /// Always [`MANIFEST_VERSION`].
    pub version: u32,
    /// Telemetry format of the session files.
    pub telemetry_format: String,
    /// Telemetry version of the session files.
    pub telemetry_version: u32,
    /// Sessions per (class, scenario) pair.
    pub sessions_per_class: u32,
    /// Session length, seconds.
    pub duration_s: f64,
    /// Probe amplitude, parts per million.
    pub amplitude_ppm: u32,
    /// Probe signal shape.
    pub probe_shape: String,
    /// Smoothing factor of `smoothing-aimbot` sessions.
    pub smoothing: f64,
    /// Estimation window of `fast-adaptive-aimbot` sessions, milliseconds.
    pub estimation_window_ms: u32,
    /// One entry per session file.
    pub sessions: Vec<ManifestEntry>,
}

/// One session file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ManifestEntry {
    /// Path relative to the batch directory, with `/` separators.
    pub file: String,
    /// Player class (ground-truth label).
    pub class: String,
    /// Scenario.
    pub scenario: String,
    /// Match identifier in the session header.
    pub match_id: String,
    /// Player identifier in the session header.
    pub player_id: u64,
    /// Shots fired.
    pub shots: usize,
    /// Shots that hit.
    pub hits: usize,
}

/// Simulates `options` and writes the batch into `dir`, which is created if needed.
///
/// # Errors
/// I/O errors, or `dir` already holding a manifest (batches are never overwritten).
pub fn generate(seed: &SimSeed, options: &Options, dir: &Path) -> io::Result<Manifest> {
    let manifest_path = dir.join("manifest.json");
    if manifest_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{} already exists; choose a new directory",
                manifest_path.display()
            ),
        ));
    }
    let root = seed.probe_root();
    let mut entries = Vec::new();
    for &class in &options.classes {
        fs::create_dir_all(dir.join(class.name()))?;
        for &scenario in class.scenarios() {
            for index in 0..options.sessions {
                let spec = SessionSpec {
                    class,
                    scenario,
                    index,
                    duration_s: options.duration_s,
                    amplitude: options.amplitude,
                    models: options.models,
                };
                let session = run_session(seed, &root, spec);
                let file = format!("{}/{}-{index:04}.jsonl", class.name(), scenario.name());
                let mut out = BufWriter::new(fs::File::create(dir.join(&file))?);
                telemetry::write_jsonl(&mut out, &session.records)?;
                out.flush()?;
                entries.push(ManifestEntry {
                    file,
                    class: class.name().to_owned(),
                    scenario: scenario.name().to_owned(),
                    match_id: session.match_id,
                    player_id: session.player_id,
                    shots: session.truth.len(),
                    hits: session.truth.iter().filter(|t| t.hit).count(),
                });
            }
        }
    }
    let manifest = Manifest {
        format: MANIFEST_FORMAT.to_owned(),
        version: MANIFEST_VERSION,
        telemetry_format: telemetry::FORMAT.to_owned(),
        telemetry_version: telemetry::VERSION,
        sessions_per_class: options.sessions,
        duration_s: options.duration_s,
        amplitude_ppm: options.amplitude.ppm(),
        probe_shape: "band-limited noise (default)".to_owned(),
        smoothing: options.models.smoothing,
        estimation_window_ms: options.models.estimation_window_ms,
        sessions: entries,
    };
    let mut text = serde_json::to_string_pretty(&manifest).map_err(io::Error::other)?;
    text.push('\n');
    fs::write(&manifest_path, text)?;
    Ok(manifest)
}
