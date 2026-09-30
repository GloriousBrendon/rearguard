// SPDX-License-Identifier: MIT OR Apache-2.0

//! `rearguard-sim`: generate simulated telemetry sessions for every player class.
//!
//! Runs only inside Rearguard's closed test environment; it writes files and nothing
//! else. See `README.md`.

use std::path::PathBuf;
use std::process::ExitCode;

use rearguard_core::probe::Amplitude;
use rearguard_sim::generate::{Options, generate};
use rearguard_sim::seed::SimSeed;
use rearguard_sim::session::{Class, ModelParams};

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
                       adaptive-aimbot, smoothing-aimbot, fast-adaptive-aimbot
  --smoothing F        smoothing-aimbot: fraction of the remaining error moved
                       per frame, 0 < F <= 1 (default 0.2)
  --estimation-window-ms W
                       fast-adaptive-aimbot: drift estimation window in
                       milliseconds, 1 to 60000 (default 250)
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
        models: ModelParams::DEFAULT,
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
            "--smoothing" => {
                let f: f64 = value.parse().map_err(|_| bad("expected a number"))?;
                if !(f > 0.0 && f <= 1.0) {
                    return Err(bad("expected 0 < F <= 1"));
                }
                options.models.smoothing = f;
            }
            "--estimation-window-ms" => {
                let w: u32 = value.parse().map_err(|_| bad("expected milliseconds"))?;
                if !(1..=60_000).contains(&w) {
                    return Err(bad("expected 1 to 60000"));
                }
                options.models.estimation_window_ms = w;
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
                        "  {:<20} {:<6} {n:>4} sessions {shots:>7} shots {hits:>7} hits",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Option<Args>, String> {
        parse(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn parses_the_model_options() {
        let a = args(&[
            "--out",
            "d",
            "--seed",
            "3",
            "--smoothing",
            "0.35",
            "--estimation-window-ms",
            "80",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(a.options.models.smoothing, 0.35);
        assert_eq!(a.options.models.estimation_window_ms, 80);
        let defaults = args(&["--out", "d", "--seed", "3"]).unwrap().unwrap();
        assert_eq!(defaults.options.models, ModelParams::DEFAULT);
        assert_eq!(defaults.options.classes, Class::ALL.to_vec());
    }

    #[test]
    fn rejects_out_of_range_model_options() {
        for bad in [
            &["--smoothing", "0"][..],
            &["--smoothing", "1.5"],
            &["--smoothing", "NaN"],
            &["--estimation-window-ms", "0"],
            &["--estimation-window-ms", "60001"],
        ] {
            let mut list = vec!["--out", "d", "--seed", "3"];
            list.extend_from_slice(bad);
            assert!(args(&list).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn accepts_the_new_class_names() {
        let a = args(&[
            "--out",
            "d",
            "--seed",
            "1",
            "--classes",
            "smoothing-aimbot,fast-adaptive-aimbot",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            a.options.classes,
            vec![Class::SmoothingAimbot, Class::FastAdaptiveAimbot]
        );
    }
}
