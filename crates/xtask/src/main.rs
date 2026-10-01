// SPDX-License-Identifier: MIT OR Apache-2.0

//! `cargo xtask`: repository tasks. See the crate README.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use rearguard_sim::seed::SimSeed;
use xtask::config::EvalConfig;
use xtask::eval::{Inputs, evaluate};
use xtask::inputs::{load_bots, load_humans, load_root};
use xtask::report::render;

const USAGE: &str = "\
Usage: cargo xtask eval --seed N [options]

Runs simulated, test-bot and human sessions through the detector and reports detection
rate and false-positive rate together (task 1.12).

Options:
  --seed N                    Simulation seed (required; never written out)
  --config FILE               Evaluation configuration (default: crates/xtask/eval/eval.json)
  --out DIR                   Output directory (default: target/eval; files are overwritten)
  --humans PATH               Study export: a zip, an extracted export, or a folder of
                              either. May be repeated
  --study-key FILE            Study key of the release the exports came from (64 hex
                              digits). May be repeated, one per release
  --bots DIR                  Test-bot recordings from the aim range (NAME.jsonl with
                              NAME.jsonl.env.json; see scripts/record-test-bots.sh)
  --bots-master-secret FILE   Master secret of the server the bots played against
  --allow-test-exports        Accept exports that are not from people (automated
                              self-test runs, debug builds); the report says so
  --strict                    Fail if any recorded session had to be left out because
                              something is wrong with it
  --threads N                 Worker threads (default: all cores; results do not depend on it)";

struct Args {
    seed: u64,
    config: PathBuf,
    out: PathBuf,
    humans: Vec<PathBuf>,
    study_keys: Vec<PathBuf>,
    bots: Option<PathBuf>,
    bots_secret: Option<PathBuf>,
    allow_test_exports: bool,
    strict: bool,
    threads: usize,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut args = Args {
        seed: 0,
        config: Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/eval.json"),
        out: PathBuf::from("target/eval"),
        humans: Vec::new(),
        study_keys: Vec::new(),
        bots: None,
        bots_secret: None,
        allow_test_exports: false,
        strict: false,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    let mut have_seed = false;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--allow-test-exports" => args.allow_test_exports = true,
            "--strict" => args.strict = true,
            _ => {
                let value = it.next().ok_or_else(|| format!("{arg} needs a value"))?;
                // The value may be a seed: never repeat it.
                let bad = || format!("{arg}: invalid value");
                match arg.as_str() {
                    "--seed" => (args.seed, have_seed) = (value.parse().map_err(|_| bad())?, true),
                    "--config" => args.config = value.into(),
                    "--out" => args.out = value.into(),
                    "--humans" => args.humans.push(value.into()),
                    "--study-key" => args.study_keys.push(value.into()),
                    "--bots" => args.bots = Some(value.into()),
                    "--bots-master-secret" => args.bots_secret = Some(value.into()),
                    "--threads" => args.threads = value.parse().map_err(|_| bad())?,
                    _ => return Err(format!("unknown option {arg}")),
                }
            }
        }
    }
    if !have_seed {
        return Err("--seed is required".to_owned());
    }
    if args.humans.is_empty() != args.study_keys.is_empty() {
        return Err("--humans and --study-key go together".to_owned());
    }
    if args.bots.is_some() != args.bots_secret.is_some() {
        return Err("--bots and --bots-master-secret go together".to_owned());
    }
    if args.threads == 0 {
        return Err("--threads must be at least 1".to_owned());
    }
    Ok(Some(args))
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
    )
}

/// The revision the results come from: the commit hash, with `-dirty` if tracked or
/// untracked files differ from it. Files in the output directory do not count, so
/// writing the results does not change the next run's answer.
fn revision(out: &Path) -> String {
    let Some(hash) = git(&["rev-parse", "HEAD"]) else {
        return "unknown (not a git checkout)".to_owned();
    };
    let inside = git(&["rev-parse", "--show-toplevel"]).and_then(|top| {
        let top = fs::canonicalize(top).ok()?;
        let out = fs::canonicalize(out).ok()?;
        let relative = out.strip_prefix(top).ok()?.to_owned();
        let parts: Vec<String> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        Some(format!("{}/", parts.join("/")))
    });
    let status = git(&["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default();
    let dirty = status.lines().any(|line| {
        let path = line.get(3..).unwrap_or("").trim_matches('"');
        !inside
            .as_ref()
            .is_some_and(|prefix| path.starts_with(prefix.as_str()))
    });
    if dirty { format!("{hash}-dirty") } else { hash }
}

fn eval(args: &Args) -> Result<(), String> {
    let text =
        fs::read_to_string(&args.config).map_err(|e| format!("{}: {e}", args.config.display()))?;
    let config =
        EvalConfig::from_json(&text).map_err(|e| format!("{}: {e}", args.config.display()))?;
    fs::create_dir_all(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;

    let humans = if args.humans.is_empty() {
        None
    } else {
        let keys = args
            .study_keys
            .iter()
            .map(|path| load_root(path))
            .collect::<Result<Vec<_>, _>>()?;
        Some(load_humans(
            &args.humans,
            &keys,
            &config,
            args.allow_test_exports,
        )?)
    };
    let bots = match (&args.bots, &args.bots_secret) {
        (Some(dir), Some(secret)) => Some(load_bots(dir, &load_root(secret)?, &config)?),
        _ => None,
    };
    for (what, loaded) in [("human", &humans), ("test-bot", &bots)] {
        if let Some(loaded) = loaded {
            eprintln!(
                "xtask eval: {what} sessions: {} used, {} left out ({} of them with a problem)",
                loaded.sessions.len(),
                loaded.excluded.0.values().sum::<u64>(),
                loaded.excluded.problems()
            );
        }
    }
    eprintln!(
        "xtask eval: simulating {} + {} humans per scenario and {} sessions per cheat class, at {} amplitudes, {} s each, on {} threads",
        config.sim.calibration_humans,
        config.sim.held_out_humans,
        config.sim.cheats,
        config.amplitudes_ppm.len(),
        config.horizon_s,
        args.threads
    );
    let results = evaluate(
        &config,
        Inputs {
            seed: SimSeed::new(args.seed),
            humans,
            bots,
            threads: args.threads,
        },
    )?;
    let files = render(&revision(&args.out), &config, &results);
    for (name, body) in &files {
        let path = args.out.join(name);
        fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    eprintln!(
        "xtask eval: wrote {} files to {} (start with report.md)",
        files.len(),
        args.out.display()
    );
    let problems = results.excluded.problems();
    if args.strict && problems > 0 {
        return Err(format!(
            "--strict: {problems} recorded sessions were left out with a problem (see report.md, \"Sessions\")"
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("eval") => {}
        Some("-h" | "--help") | None => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("xtask: unknown task '{other}'\n\n{USAGE}");
            return ExitCode::from(2);
        }
    }
    let args = match parse(args) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("xtask eval: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match eval(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask eval: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Option<Args>, String> {
        parse(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn the_seed_is_required_and_inputs_come_with_their_keys() {
        assert!(args(&[]).is_err());
        let a = args(&["--seed", "5"]).unwrap().unwrap();
        assert_eq!((a.seed, a.out.to_str()), (5, Some("target/eval")));
        assert!(a.config.ends_with("eval/eval.json") && a.config.is_file());
        assert!(args(&["--help"]).unwrap().is_none());
        assert!(args(&["--seed", "5", "--humans", "x"]).is_err());
        assert!(args(&["--seed", "5", "--bots", "x"]).is_err());
        assert!(args(&["--seed", "5", "--threads", "0"]).is_err());
        assert!(args(&["--seed", "5", "--nope", "1"]).is_err());
        let a = args(&[
            "--seed",
            "5",
            "--humans",
            "a",
            "--humans",
            "b",
            "--study-key",
            "k",
            "--bots",
            "d",
            "--bots-master-secret",
            "m",
            "--strict",
            "--allow-test-exports",
        ])
        .unwrap()
        .unwrap();
        assert_eq!((a.humans.len(), a.study_keys.len()), (2, 1));
        assert!(a.strict && a.allow_test_exports && a.bots.is_some());
    }

    #[test]
    fn a_bad_seed_is_not_echoed() {
        let message = args(&["--seed", "12345x"]).err().unwrap();
        assert!(!message.contains("12345"), "{message}");
    }
}
