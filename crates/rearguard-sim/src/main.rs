//! `rearguard-sim`: generate simulated telemetry sessions for every player class.
//!
//! Runs only inside Rearguard's closed test environment; it writes files and nothing
//! else. See `README.md`.

use std::path::PathBuf;
use std::process::ExitCode;

use rearguard_core::probe::Amplitude;
use rearguard_sim::generate::{Options, generate};
use rearguard_sim::seed::SimSeed;
use rearguard_sim::session::Class;

const USAGE: &str = "\
Usage: rearguard-sim --out DIR --seed N [options]

Simulates N sessions per player class and scenario, writing JSON Lines telemetry to
DIR/<class>/<scenario>-<index>.jsonl and a manifest to DIR/manifest.json. The same
--seed and options reproduce every file byte for byte. The seed is never written out.

Options:
  --out DIR            Output directory (must not already hold a manifest)
  --seed N             Top-level seed, an unsigned 64-bit integer
  --sessions N         Sessions per class and scenario (default 10)
  --duration S         Session length in seconds (default 60)
  --amplitude-ppm P    Probe drift amplitude, 0 to 20000 ppm (default 5000)
  --classes A,B,...    Classes to simulate (default: all)
                       human, recoil-macro, flick-aimbot, humanised-aimbot,
                       adaptive-aimbot
  -h, --help           Show this help";

struct Args {
    out: PathBuf,
    seed: u64,
    options: Options,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let (mut out, mut seed) = (None, None);
    let mut options = Options {
        sessions: 10,
        duration_s: 60.0,
        amplitude: Amplitude::DEFAULT,
        classes: Class::ALL.to_vec(),
    };
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            return Ok(None);
        }
        let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
        let bad = |what: &str| format!("{arg}: {what}, got '{value}'");
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(&value)),
            "--seed" => {
                seed = Some(
                    value
                        .parse()
                        .map_err(|_| bad("expected an unsigned integer"))?,
                )
            }
            "--sessions" => {
                options.sessions = value
                    .parse()
                    .map_err(|_| bad("expected an unsigned integer"))?;
            }
            "--duration" => {
                let d: f64 = value.parse().map_err(|_| bad("expected seconds"))?;
                if !(d.is_finite() && d > 0.0 && d <= 3_600.0) {
                    return Err(bad("expected 0 < seconds <= 3600"));
                }
                options.duration_s = d;
            }
            "--amplitude-ppm" => {
                let ppm: u32 = value.parse().map_err(|_| bad("expected an integer"))?;
                options.amplitude = Amplitude::from_ppm(ppm).map_err(|e| format!("{arg}: {e}"))?;
            }
            "--classes" => {
                options.classes = value
                    .split(',')
                    .map(|name| Class::from_name(name.trim()).ok_or_else(|| bad("unknown class")))
                    .collect::<Result<_, _>>()?;
                options.classes.sort();
                options.classes.dedup();
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    Ok(Some(Args {
        out: out.ok_or("--out is required")?,
        seed: seed.ok_or("--seed is required")?,
        options,
    }))
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("rearguard-sim: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let seed = SimSeed::new(args.seed);
    match generate(&seed, &args.options, &args.out) {
        Ok(manifest) => {
            println!(
                "rearguard-sim: wrote {} sessions to {}",
                manifest.sessions.len(),
                args.out.display()
            );
            for class in &args.options.classes {
                for scenario in class.scenarios() {
                    let rows = manifest
                        .sessions
                        .iter()
                        .filter(|s| s.class == class.name() && s.scenario == scenario.name());
                    let (n, shots, hits) =
                        rows.fold((0, 0, 0), |(n, s, h), e| (n + 1, s + e.shots, h + e.hits));
                    println!(
                        "  {:<17} {:<6} {n:>4} sessions {shots:>7} shots {hits:>7} hits",
                        class.name(),
                        scenario.name()
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("rearguard-sim: {e}");
            ExitCode::FAILURE
        }
    }
}
