//! `rearguard-eval`: offline evaluation of the input-probe detector on simulated
//! players (task 1.3). Writes ROC data, detection rates with bootstrap intervals, times
//! to detection, the slope-versus-significance comparison, the step-response results,
//! calibrated thresholds, two plots and a summary into an output directory.
//!
//! Runs only inside Rearguard's closed test environment. Results describe the
//! simulator's models, not real players.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use rearguard_core::detect::{DetectorConfig, Report};
use rearguard_sim::evaluate::{
    Plan, Results, alternative_z, detection_rate, first_detection, rate_above, roc, run, scores,
    threshold_at,
};
use rearguard_sim::seed::SimSeed;
use rearguard_sim::session::{Class, ModelParams};
use rearguard_sim::world::Scenario;

const USAGE: &str = "\
Usage: rearguard-eval --config FILE --out DIR --seed N [options]

Options:
  --config FILE              Detector configuration (JSON, rearguard_core::detect::DetectorConfig)
  --out DIR                  Output directory (created; files are overwritten)
  --seed N                   Top-level simulation seed (never written out)
  --humans N                 Human sessions per scenario and amplitude (default 5000)
  --cheats N                 Sessions per cheat class and amplitude (default 500)
  --bootstrap N              Bootstrap rounds (default 1000)
  --fpr F                    Target false-positive rate (default 0.001)
  --engagements-per-match E  Aimed engagements in one real match, for conversions (default 50)
  --threads N                Worker threads (default: all cores)";

const AMPLITUDES_PPM: [u32; 4] = [2_500, 5_000, 10_000, 20_000];
const OBSERVE_MS: [u64; 4] = [30_000, 120_000, 300_000, 900_000];
/// Slope bound for the slope-only comparison test (criterion 5): a generous ceiling
/// above the simulated human's measured flick slope of about 0.26.
const SLOPE_ONLY_BOUND: f64 = 0.5;

struct Args {
    config: PathBuf,
    out: PathBuf,
    seed: u64,
    humans: u32,
    cheats: u32,
    bootstrap: usize,
    fpr: f64,
    per_match: f64,
    threads: usize,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut a = Args {
        config: PathBuf::new(),
        out: PathBuf::new(),
        seed: 0,
        humans: 5_000,
        cheats: 500,
        bootstrap: 1_000,
        fpr: 0.001,
        per_match: 50.0,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    let (mut have_config, mut have_out, mut have_seed) = (false, false, false);
    while let Some(arg) = it.next() {
        if arg == "-h" || arg == "--help" {
            return Ok(None);
        }
        let v = it.next().ok_or_else(|| format!("{arg} needs a value"))?;
        let bad = || format!("{arg}: invalid value '{v}'");
        match arg.as_str() {
            "--config" => (a.config, have_config) = (PathBuf::from(&v), true),
            "--out" => (a.out, have_out) = (PathBuf::from(&v), true),
            "--seed" => (a.seed, have_seed) = (v.parse().map_err(|_| bad())?, true),
            "--humans" => a.humans = v.parse().map_err(|_| bad())?,
            "--cheats" => a.cheats = v.parse().map_err(|_| bad())?,
            "--bootstrap" => a.bootstrap = v.parse().map_err(|_| bad())?,
            "--fpr" => a.fpr = v.parse().map_err(|_| bad())?,
            "--engagements-per-match" => a.per_match = v.parse().map_err(|_| bad())?,
            "--threads" => a.threads = v.parse().map_err(|_| bad())?,
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    if !(have_config && have_out && have_seed) {
        return Err("--config, --out and --seed are required".to_owned());
    }
    if a.humans < 10
        || a.cheats < 1
        || a.bootstrap < 10
        || !(a.fpr > 0.0 && a.fpr < 1.0)
        || a.per_match <= 0.0
    {
        return Err("out-of-range option".to_owned());
    }
    Ok(Some(a))
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("rearguard-eval: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match evaluate(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rearguard-eval: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Cheat classes and the human scenario they are compared with.
fn cheats() -> Vec<(Class, Scenario)> {
    Class::ALL
        .into_iter()
        .filter(|c| *c != Class::Human)
        .flat_map(|c| c.scenarios().iter().map(move |s| (c, *s)))
        .collect()
}

fn pct(ppm: u32) -> f64 {
    f64::from(ppm) / 10_000.0
}

fn secs(ms: u64) -> u64 {
    ms / 1_000
}

fn error_score(r: &Report) -> f64 {
    r.error.score
}

fn steps_score(r: &Report) -> f64 {
    r.steps.score
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

fn evaluate(args: &Args) -> Result<(), String> {
    let text =
        fs::read_to_string(&args.config).map_err(|e| format!("{}: {e}", args.config.display()))?;
    let config: DetectorConfig =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", args.config.display()))?;
    fs::create_dir_all(&args.out).map_err(|e| e.to_string())?;
    let seed = SimSeed::new(args.seed);
    let plan = Plan {
        amplitudes_ppm: AMPLITUDES_PPM.to_vec(),
        observe_ms: OBSERVE_MS.to_vec(),
        humans: args.humans,
        cheats: args.cheats,
        models: ModelParams::DEFAULT,
        threads: args.threads,
    };
    let started = std::time::Instant::now();
    eprintln!(
        "rearguard-eval: simulating {} humans per scenario and {} sessions per cheat class, at {} amplitudes, on {} threads",
        args.humans,
        args.cheats,
        AMPLITUDES_PPM.len(),
        args.threads
    );
    let results = run(&seed, &config, &plan);
    eprintln!(
        "rearguard-eval: simulated in {:.0} s",
        started.elapsed().as_secs_f64()
    );
    let mut rng = seed.rng(0x0000_E0A1_0000_0000);

    let out = |name: &str| args.out.join(name);
    let write = |path: PathBuf, body: String| {
        fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))
    };

    // 1. ROC data (fire-time score; step-response score for the smoothing aimbot).
    let mut roc_csv =
        String::from("amplitude_pct,observe_s,statistic,class,scenario,threshold,fpr,tpr\n");
    let mut roc_curves: Vec<Curve> = Vec::new();
    for &ppm in &AMPLITUDES_PPM {
        for (k, &ms) in OBSERVE_MS.iter().enumerate() {
            for (class, scenario) in cheats() {
                let human = results.traces(ppm, Class::Human, scenario);
                let cheat = results.traces(ppm, class, scenario);
                let mut statistics: Vec<Statistic> = vec![("error", error_score)];
                if class == Class::SmoothingAimbot {
                    statistics.push(("steps", steps_score));
                }
                for (name, pick) in statistics {
                    let points = roc(&scores(human, k, pick), &scores(cheat, k, pick));
                    // Every point in the low-FPR region; a coarse grid above 1%.
                    let mut last_grid = -1.0;
                    for &(t, fpr, tpr) in &points {
                        let keep = fpr <= 0.01 || fpr - last_grid >= 0.02 || fpr >= 1.0;
                        if keep {
                            if fpr > 0.01 {
                                last_grid = fpr;
                            }
                            let _ = writeln!(
                                roc_csv,
                                "{},{},{name},{},{},{t},{fpr},{tpr}",
                                pct(ppm),
                                secs(ms),
                                class.name(),
                                scenario.name()
                            );
                        }
                    }
                    roc_curves.push((
                        ppm,
                        ms,
                        name,
                        class,
                        points.iter().map(|p| (p.1, p.2)).collect(),
                    ));
                }
            }
        }
    }
    write(out("roc.csv"), roc_csv)?;

    // 2. Detection rate at the target FPR, with bootstrap intervals.
    let mut det_csv = String::from(
        "amplitude_pct,observe_s,statistic,class,scenario,fpr_target,threshold,measured_human_fpr,detection_rate,ci_low,ci_high,humans,cheats\n",
    );
    let mut det_rows: Vec<(u32, u64, &'static str, Class, f64, f64, f64)> = Vec::new();
    let mut thresholds_json = Vec::new();
    for &ppm in &AMPLITUDES_PPM {
        for (k, &ms) in OBSERVE_MS.iter().enumerate() {
            for scenario in [Scenario::Flick, Scenario::Spray] {
                let human = results.traces(ppm, Class::Human, scenario);
                let t_err = threshold_at(&scores(human, k, error_score), args.fpr);
                let t_steps = threshold_at(&scores(human, k, steps_score), args.fpr);
                thresholds_json.push(format!(
                    "    {{\"amplitude_pct\": {}, \"observe_s\": {}, \"scenario\": \"{}\", \"error_flag_score\": {t_err}, \"steps_flag_score\": {t_steps}}}",
                    pct(ppm), secs(ms), scenario.name()
                ));
            }
            for (class, scenario) in cheats() {
                let human = results.traces(ppm, Class::Human, scenario);
                let cheat = results.traces(ppm, class, scenario);
                let mut statistics: Vec<Statistic> = vec![("error", error_score)];
                if class == Class::SmoothingAimbot {
                    statistics.push(("steps", steps_score));
                }
                for (name, pick) in statistics {
                    let h = scores(human, k, pick);
                    let c = scores(cheat, k, pick);
                    let r = detection_rate(&h, &c, args.fpr, args.bootstrap, &mut rng);
                    let measured = rate_above(&h, r.threshold);
                    let _ = writeln!(
                        det_csv,
                        "{},{},{name},{},{},{},{},{measured},{},{},{},{},{}",
                        pct(ppm),
                        secs(ms),
                        class.name(),
                        scenario.name(),
                        args.fpr,
                        r.threshold,
                        r.rate,
                        r.low,
                        r.high,
                        h.len(),
                        c.len()
                    );
                    det_rows.push((ppm, ms, name, class, r.rate, r.low, r.high));
                }
            }
        }
    }
    write(out("detection.csv"), det_csv)?;
    write(
        out("thresholds.json"),
        format!(
            "{{\n  \"note\": \"Flag scores calibrated on simulated humans at a false-positive rate of {} for a fixed observation time, per amplitude and scenario. kappa_bound, min_pairs, window_ms and min_step_counts as in the input config. Simulated humans only: recalibrate on real players (task 1.10).\",\n  \"kappa_bound\": {},\n  \"min_step_counts\": {},\n  \"calibrated\": [\n{}\n  ]\n}}\n",
            args.fpr,
            config.kappa_bound,
            config.min_step_counts,
            thresholds_json.join(",\n")
        ),
    )?;

    // 3. Time to detection: sequential thresholds over the longest observation, the
    // target FPR split evenly between the two statistics (union bound).
    let last = OBSERVE_MS.len() - 1;
    let mut ttd_csv = String::from(
        "amplitude_pct,class,scenario,sequential_error_threshold,sequential_steps_threshold,measured_human_sequential_fpr,detected_fraction,median_shots,p90_shots,median_engagements,p90_engagements,median_matches\n",
    );
    let mut ttd_rows = Vec::new();
    for &ppm in &AMPLITUDES_PPM {
        for scenario in [Scenario::Flick, Scenario::Spray] {
            let human = results.traces(ppm, Class::Human, scenario);
            let max_err: Vec<f64> = human
                .iter()
                .map(|t| t.max_error[last].max(t.snapshots[last].error.score))
                .collect();
            let max_steps: Vec<f64> = human
                .iter()
                .map(|t| t.max_steps[last].max(t.snapshots[last].steps.score))
                .collect();
            let te = threshold_at(&max_err, args.fpr / 2.0);
            let ts = threshold_at(&max_steps, args.fpr / 2.0);
            let fp = max_err
                .iter()
                .zip(&max_steps)
                .filter(|(e, s)| **e > te || **s > ts)
                .count() as f64
                / human.len() as f64;
            for (class, s) in cheats() {
                if s != scenario {
                    continue;
                }
                let found: Vec<(u32, u32)> = results
                    .traces(ppm, class, s)
                    .iter()
                    .filter_map(|t| first_detection(&t.path, te, ts))
                    .collect();
                let n = results.traces(ppm, class, s).len() as f64;
                let mut shots: Vec<f64> = found.iter().map(|f| f64::from(f.0)).collect();
                let mut eng: Vec<f64> = found.iter().map(|f| f64::from(f.1)).collect();
                let (ms, ps) = (median(&mut shots), quantile(&mut shots, 0.9));
                let (me, pe) = (median(&mut eng), quantile(&mut eng, 0.9));
                let _ = writeln!(
                    ttd_csv,
                    "{},{},{},{te},{ts},{fp},{},{ms},{ps},{me},{pe},{}",
                    pct(ppm),
                    class.name(),
                    s.name(),
                    found.len() as f64 / n,
                    me / args.per_match
                );
                ttd_rows.push((ppm, class, fp, found.len() as f64 / n, ms, ps, me, pe));
            }
        }
    }
    write(out("ttd.csv"), ttd_csv)?;

    // 4. Criterion 5: slope against significance, leaky human against fast adaptive.
    let mut sep_csv = String::from(
        "amplitude_pct,observe_s,class,scenario,median_slope,median_kappa,median_r,median_z,median_plain_z,dr_kappa_test,dr_plain_significance,dr_slope_only\n",
    );
    let mut sep_rows = Vec::new();
    for &ppm in &AMPLITUDES_PPM {
        for (k, &ms) in OBSERVE_MS.iter().enumerate() {
            let human = results.traces(ppm, Class::Human, Scenario::Flick);
            let h_kappa = scores(human, k, error_score);
            let h_plain: Vec<f64> = human
                .iter()
                .map(|t| alternative_z(&t.snapshots[k].error, SLOPE_ONLY_BOUND).0)
                .collect();
            let h_slope: Vec<f64> = human
                .iter()
                .map(|t| alternative_z(&t.snapshots[k].error, SLOPE_ONLY_BOUND).1)
                .collect();
            let (tk, tp, tsl) = (
                threshold_at(&h_kappa, args.fpr),
                threshold_at(&h_plain, args.fpr),
                threshold_at(&h_slope, args.fpr),
            );
            for class in [
                Class::Human,
                Class::FastAdaptiveAimbot,
                Class::AdaptiveAimbot,
                Class::HumanisedAimbot,
            ] {
                let t = results.traces(ppm, class, Scenario::Flick);
                let ev = |f: fn(&rearguard_core::detect::Evidence) -> f64| -> f64 {
                    let mut v: Vec<f64> = t.iter().map(|x| f(&x.snapshots[k].error)).collect();
                    median(&mut v)
                };
                let kappa = scores(t, k, error_score);
                let plain: Vec<f64> = t
                    .iter()
                    .map(|x| alternative_z(&x.snapshots[k].error, SLOPE_ONLY_BOUND).0)
                    .collect();
                let slope: Vec<f64> = t
                    .iter()
                    .map(|x| alternative_z(&x.snapshots[k].error, SLOPE_ONLY_BOUND).1)
                    .collect();
                let mut plain_sorted = plain.clone();
                let row = (
                    ppm,
                    ms,
                    class,
                    ev(|e| e.slope),
                    ev(|e| e.kappa),
                    ev(|e| e.r),
                    ev(|e| e.z),
                    rate_above(&kappa, tk),
                    rate_above(&plain, tp),
                    rate_above(&slope, tsl),
                    median(&mut plain_sorted),
                );
                let _ = writeln!(
                    sep_csv,
                    "{},{},{},flick,{},{},{},{},{},{},{},{}",
                    pct(ppm),
                    secs(ms),
                    class.name(),
                    row.3,
                    row.4,
                    row.5,
                    row.6,
                    row.10,
                    row.7,
                    row.8,
                    row.9
                );
                sep_rows.push(row);
            }
        }
    }
    write(out("separation.csv"), sep_csv)?;

    // 5. Criterion 6: the step-response statistic against the smoothing aimbot.
    let mut steps_csv = String::from(
        "amplitude_pct,observe_s,fpr_target,threshold,measured_human_flick_fpr,human_spray_fpr_at_same_threshold,detection_rate,ci_low,ci_high,detection_rate_at_1pct_fpr\n",
    );
    let mut steps_rows = Vec::new();
    for &ppm in &AMPLITUDES_PPM {
        for (k, &ms) in OBSERVE_MS.iter().enumerate() {
            let h = scores(
                results.traces(ppm, Class::Human, Scenario::Flick),
                k,
                steps_score,
            );
            let hs = scores(
                results.traces(ppm, Class::Human, Scenario::Spray),
                k,
                steps_score,
            );
            let c = scores(
                results.traces(ppm, Class::SmoothingAimbot, Scenario::Flick),
                k,
                steps_score,
            );
            let r = detection_rate(&h, &c, args.fpr, args.bootstrap, &mut rng);
            let at1 = rate_above(&c, threshold_at(&h, 0.01));
            let _ = writeln!(
                steps_csv,
                "{},{},{},{},{},{},{},{},{},{at1}",
                pct(ppm),
                secs(ms),
                args.fpr,
                r.threshold,
                rate_above(&h, r.threshold),
                rate_above(&hs, r.threshold),
                r.rate,
                r.low,
                r.high
            );
            steps_rows.push((
                ppm,
                ms,
                r.rate,
                r.low,
                r.high,
                at1,
                rate_above(&hs, r.threshold),
            ));
        }
    }
    write(out("steps.csv"), steps_csv)?;

    // 6. Plots.
    write(out("roc.svg"), roc_svg(&roc_curves, args.fpr, false))?;
    write(out("roc-steps.svg"), roc_svg(&roc_curves, args.fpr, true))?;

    // 7. Monotonicity across the grid (criterion 4), and the summary.
    write(
        out("summary.md"),
        summary(args, &results, &det_rows, &ttd_rows, &sep_rows, &steps_rows),
    )?;
    eprintln!(
        "rearguard-eval: wrote {} in {:.0} s",
        args.out.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// (amplitude ppm, observation ms, statistic, class, ROC points (fpr, tpr)).
type Curve = (u32, u64, &'static str, Class, Vec<(f64, f64)>);
/// A named score picked from a report.
type Statistic = (&'static str, fn(&Report) -> f64);
type DetRow = (u32, u64, &'static str, Class, f64, f64, f64);
type TtdRow = (u32, Class, f64, f64, f64, f64, f64, f64);
type SepRow = (u32, u64, Class, f64, f64, f64, f64, f64, f64, f64, f64);
type StepsRow = (u32, u64, f64, f64, f64, f64, f64);

fn fmt_rate(rate: f64, low: f64, high: f64) -> String {
    format!(
        "{:.1}% [{:.1}, {:.1}]",
        rate * 100.0,
        low * 100.0,
        high * 100.0
    )
}

fn summary(
    args: &Args,
    results: &Results,
    det: &[DetRow],
    ttd: &[TtdRow],
    sep: &[SepRow],
    steps: &[StepsRow],
) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Detector evaluation (task 1.3)\n");
    let _ = writeln!(
        s,
        "Generated by `rearguard-eval`. Simulated players only: every number describes the simulator's models, not real people (validation: task 1.10).\n"
    );
    let _ = writeln!(
        s,
        "- Humans: {} sessions per scenario (flick, spray) per amplitude; cheats: {} sessions per class per amplitude; each session 15 minutes of aim-range time, scored at 30 s, 2 min, 5 min and 15 min.",
        args.humans, args.cheats
    );
    let _ = writeln!(
        s,
        "- False-positive rate target: {}%. Thresholds are calibrated on the simulated humans of the same scenario, amplitude and observation time. Intervals: {} bootstrap rounds resampling humans and cheats (threshold re-calibrated each round), 95% percentile interval.",
        args.fpr * 100.0,
        args.bootstrap
    );
    let _ = writeln!(
        s,
        "- Real-match conversion: {} aimed engagements per match (assumption; see the assumptions note).\n",
        args.per_match
    );

    let _ = writeln!(
        s,
        "## Detection rate at {}% FPR, fire-time statistic\n",
        args.fpr * 100.0
    );
    for (class, scenario) in cheats() {
        let _ = writeln!(s, "**{}** ({} scenario)\n", class.name(), scenario.name());
        let _ = writeln!(
            s,
            "| Amplitude | 30 s | 2 min | 5 min | 15 min |\n|---|---|---|---|---|"
        );
        for &ppm in &AMPLITUDES_PPM {
            let cells: Vec<String> = OBSERVE_MS
                .iter()
                .map(|&ms| {
                    det.iter()
                        .find(|r| r.0 == ppm && r.1 == ms && r.2 == "error" && r.3 == class)
                        .map_or("-".to_owned(), |r| fmt_rate(r.4, r.5, r.6))
                })
                .collect();
            let _ = writeln!(s, "| {}% | {} |", pct(ppm), cells.join(" | "));
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## Monotonicity (criterion 4)\n");
    let mut violations = Vec::new();
    for (class, _) in cheats() {
        for &ppm in &AMPLITUDES_PPM {
            for w in OBSERVE_MS.windows(2) {
                let a = det
                    .iter()
                    .find(|r| r.0 == ppm && r.1 == w[0] && r.2 == "error" && r.3 == class);
                let b = det
                    .iter()
                    .find(|r| r.0 == ppm && r.1 == w[1] && r.2 == "error" && r.3 == class);
                if let (Some(a), Some(b)) = (a, b)
                    && b.4 < a.4
                    && b.6 < a.5
                {
                    violations.push(format!(
                        "{} at {}%: {} s -> {} s",
                        class.name(),
                        pct(ppm),
                        secs(w[0]),
                        secs(w[1])
                    ));
                }
            }
        }
        for &ms in &OBSERVE_MS {
            for w in AMPLITUDES_PPM.windows(2) {
                let a = det
                    .iter()
                    .find(|r| r.0 == w[0] && r.1 == ms && r.2 == "error" && r.3 == class);
                let b = det
                    .iter()
                    .find(|r| r.0 == w[1] && r.1 == ms && r.2 == "error" && r.3 == class);
                if let (Some(a), Some(b)) = (a, b)
                    && b.4 < a.4
                    && b.6 < a.5
                {
                    violations.push(format!(
                        "{} at {} s: {}% -> {}%",
                        class.name(),
                        secs(ms),
                        pct(w[0]),
                        pct(w[1])
                    ));
                }
            }
        }
    }
    // The step-response statistic against the smoothing aimbot, its intended target.
    for &ppm in &AMPLITUDES_PPM {
        for w in OBSERVE_MS.windows(2) {
            let a = steps.iter().find(|r| r.0 == ppm && r.1 == w[0]);
            let b = steps.iter().find(|r| r.0 == ppm && r.1 == w[1]);
            if let (Some(a), Some(b)) = (a, b)
                && b.2 < a.2
                && b.4 < a.3
            {
                violations.push(format!(
                    "steps statistic, smoothing-aimbot at {}%: {} s -> {} s",
                    pct(ppm),
                    secs(w[0]),
                    secs(w[1])
                ));
            }
        }
    }
    for &ms in &OBSERVE_MS {
        for w in AMPLITUDES_PPM.windows(2) {
            let a = steps.iter().find(|r| r.0 == w[0] && r.1 == ms);
            let b = steps.iter().find(|r| r.0 == w[1] && r.1 == ms);
            if let (Some(a), Some(b)) = (a, b)
                && b.2 < a.2
                && b.4 < a.3
            {
                violations.push(format!(
                    "steps statistic, smoothing-aimbot at {} s: {}% -> {}%",
                    secs(ms),
                    pct(w[0]),
                    pct(w[1])
                ));
            }
        }
    }
    let _ = writeln!(
        s,
        "Checked: the fire-time statistic for every cheat class, and the step-response statistic for the smoothing aimbot. Tolerance: a step up in amplitude or observation time counts as a violation only if the detection rate falls **and** the two 95% intervals do not overlap. The fire-time rows of the smoothing aimbot are listed too, although that statistic is not meant to catch it.\n"
    );
    if violations.is_empty() {
        let _ = writeln!(s, "No violations.\n");
    } else {
        for v in &violations {
            let _ = writeln!(s, "- {v}");
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## Time to detection (sequential, 15-minute horizon)\n");
    let _ = writeln!(
        s,
        "Both statistics run continuously; each has a sequential threshold at half the FPR target, set on the simulated humans' highest score over 15 minutes, so the union flags at most {}% of humans over the whole horizon. Shots and engagements (flick targets, spray bursts) until the first flag, for sessions flagged within 15 minutes.\n",
        args.fpr * 100.0
    );
    let _ = writeln!(
        s,
        "| Amplitude | Class | Human FPR (measured) | Flagged within 15 min | Median shots | 90th pct shots | Median engagements | 90th pct engagements | Median matches |\n|---|---|---|---|---|---|---|---|---|"
    );
    for r in ttd {
        let _ = writeln!(
            s,
            "| {}% | {} | {:.2}% | {:.1}% | {:.0} | {:.0} | {:.0} | {:.0} | {:.2} |",
            pct(r.0),
            r.1.name(),
            r.2 * 100.0,
            r.3 * 100.0,
            r.4,
            r.5,
            r.6,
            r.7,
            r.6 / args.per_match
        );
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "## Slope against significance (criterion 5)\n");
    let _ = writeln!(
        s,
        "Flick scenario. Median evidence per class, and the detection rate at {}% FPR of three tests on the same pairs: the detector's kappa test (null: kappa <= kappa_bound), plain significance (null: r = 0), and a slope-only test (null: slope <= {SLOPE_ONLY_BOUND}). The human row's rates are its false-positive rates (by construction, at most the target).\n",
        args.fpr * 100.0
    );
    let _ = writeln!(
        s,
        "| Amplitude | Observe | Class | Slope | Kappa (1/deg) | r | z (kappa test) | z (plain) | DR kappa test | DR plain significance | DR slope only |\n|---|---|---|---|---|---|---|---|---|---|---|"
    );
    for r in sep {
        let _ = writeln!(
            s,
            "| {}% | {} s | {} | {:+.3} | {:.2} | {:+.3} | {:+.1} | {:+.1} | {:.1}% | {:.1}% | {:.1}% |",
            pct(r.0),
            secs(r.1),
            r.2.name(),
            r.3,
            r.4,
            r.5,
            r.6,
            r.10,
            r.7 * 100.0,
            r.8 * 100.0,
            r.9 * 100.0
        );
    }
    let _ = writeln!(s);

    let _ = writeln!(
        s,
        "## Step response against the smoothing aimbot (criterion 6)\n"
    );
    let _ = writeln!(
        s,
        "Threshold calibrated on simulated flick humans. The spray column applies the same threshold to simulated spray humans, to show the false-positive cost carries over.\n"
    );
    let _ = writeln!(
        s,
        "| Amplitude | Observe | DR at {}% FPR | DR at 1% FPR | Human spray FPR at the same threshold |\n|---|---|---|---|---|",
        args.fpr * 100.0
    );
    for r in steps {
        let _ = writeln!(
            s,
            "| {}% | {} s | {} | {:.1}% | {:.2}% |",
            pct(r.0),
            secs(r.1),
            fmt_rate(r.2, r.3, r.4),
            r.5 * 100.0,
            r.6 * 100.0
        );
    }
    let _ = writeln!(s);
    let shots_per_min = |class: Class, scenario: Scenario| {
        let t = results.traces(AMPLITUDES_PPM[1], class, scenario);
        let total: u64 = t
            .iter()
            .map(|x| x.snapshots[OBSERVE_MS.len() - 1].shots)
            .sum();
        total as f64 / t.len().max(1) as f64 / (OBSERVE_MS[OBSERVE_MS.len() - 1] as f64 / 60_000.0)
    };
    let _ = writeln!(s, "## Aim-range pace (shots per minute, 0.5%)\n");
    let _ = writeln!(s, "| Class | Scenario | Shots per minute |\n|---|---|---|");
    for (class, scenario) in std::iter::once((Class::Human, Scenario::Flick))
        .chain(std::iter::once((Class::Human, Scenario::Spray)))
        .chain(cheats())
    {
        let _ = writeln!(
            s,
            "| {} | {} | {:.0} |",
            class.name(),
            scenario.name(),
            shots_per_min(class, scenario)
        );
    }
    s
}

/// A 4 x 4 grid of ROC panels (rows: amplitude, columns: observation time), log FPR
/// axis. `steps`: the step-response statistic for the smoothing aimbot only; otherwise
/// the fire-time statistic for the three cheats that are not trivially separated.
fn roc_svg(curves: &[Curve], fpr_target: f64, steps: bool) -> String {
    let series: Vec<(Class, &str, &str)> = if steps {
        vec![(Class::SmoothingAimbot, "series-1", "")]
    } else {
        vec![
            (Class::HumanisedAimbot, "series-1", ""),
            (Class::RecoilMacro, "series-2", "6 3"),
            (Class::FastAdaptiveAimbot, "series-3", "2 3"),
        ]
    };
    let statistic = if steps { "steps" } else { "error" };
    let (pw, ph, left, top, gap_x, gap_y) = (200.0, 170.0, 70.0, 70.0, 34.0, 44.0);
    let width = left + 4.0 * pw + 3.0 * gap_x + 20.0;
    let height = top + 4.0 * ph + 3.0 * gap_y + 70.0;
    let x_of = |fpr: f64| (libm::log10(fpr.max(1e-4)) + 4.0) / 4.0 * pw;
    let y_of = |tpr: f64| (1.0 - tpr) * ph;
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" font-family="system-ui, -apple-system, Segoe UI, sans-serif" font-size="11">
<style>
  .bg {{ fill: #fcfcfb; }} .t1 {{ fill: #0b0b0b; }} .t2 {{ fill: #52514e; }}
  .grid {{ stroke: #e4e3df; stroke-width: 1; }} .axis {{ stroke: #b9b8b3; stroke-width: 1; fill: none; }}
  .ref {{ stroke: #52514e; stroke-width: 1; stroke-dasharray: 3 3; }}
  .chance {{ stroke: #b9b8b3; stroke-width: 1; fill: none; }}
  .series-1 {{ stroke: #2a78d6; }} .series-2 {{ stroke: #eb6834; }} .series-3 {{ stroke: #1baf7a; }}
  .line {{ fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }}
  @media (prefers-color-scheme: dark) {{
    .bg {{ fill: #1a1a19; }} .t1 {{ fill: #ffffff; }} .t2 {{ fill: #c3c2b7; }}
    .grid {{ stroke: #33322f; }} .axis {{ stroke: #5c5b57; }} .ref {{ stroke: #c3c2b7; }} .chance {{ stroke: #5c5b57; }}
    .series-1 {{ stroke: #3987e5; }} .series-2 {{ stroke: #d95926; }} .series-3 {{ stroke: #199e70; }}
  }}
</style>
<rect class="bg" x="0" y="0" width="{width}" height="{height}"/>
"#
    );
    let title = if steps {
        "ROC: step-response statistic, smoothing aimbot vs simulated humans (flick)"
    } else {
        "ROC: fire-time drift correlation, cheats vs simulated humans"
    };
    let _ = writeln!(
        s,
        r#"<text class="t1" x="{left}" y="24" font-size="15" font-weight="600">{title}</text>"#
    );
    let _ = writeln!(
        s,
        r#"<text class="t2" x="{left}" y="42">Rows: drift amplitude. Columns: observation time. x: false-positive rate (log), y: detection rate. Dashed line: {}% FPR.</text>"#,
        fpr_target * 100.0
    );
    for (row, &ppm) in AMPLITUDES_PPM.iter().enumerate() {
        for (col, &ms) in OBSERVE_MS.iter().enumerate() {
            let (ox, oy) = (
                left + col as f64 * (pw + gap_x),
                top + row as f64 * (ph + gap_y),
            );
            let _ = writeln!(s, r#"<g transform="translate({ox},{oy})">"#);
            for i in 0..=4 {
                let x = f64::from(i) / 4.0 * pw;
                let _ = writeln!(
                    s,
                    r#"<line class="grid" x1="{x}" y1="0" x2="{x}" y2="{ph}"/>"#
                );
                let y = f64::from(i) / 4.0 * ph;
                let _ = writeln!(
                    s,
                    r#"<line class="grid" x1="0" y1="{y}" x2="{pw}" y2="{y}"/>"#
                );
            }
            // Chance line on a log axis: tpr = fpr.
            let chance: Vec<String> = (0..=40)
                .map(|i| {
                    let f = libm::pow(10.0, -4.0 + f64::from(i) / 10.0);
                    format!("{:.1},{:.1}", x_of(f), y_of(f))
                })
                .collect();
            let _ = writeln!(
                s,
                r#"<polyline class="chance" points="{}"/>"#,
                chance.join(" ")
            );
            let xr = x_of(fpr_target);
            let _ = writeln!(
                s,
                r#"<line class="ref" x1="{xr}" y1="0" x2="{xr}" y2="{ph}"/>"#
            );
            let _ = writeln!(
                s,
                r#"<rect class="axis" x="0" y="0" width="{pw}" height="{ph}"/>"#
            );
            for (class, css, dash) in &series {
                if let Some(c) = curves
                    .iter()
                    .find(|c| c.0 == ppm && c.1 == ms && c.2 == statistic && c.3 == *class)
                {
                    let mut pts: Vec<String> = Vec::new();
                    let mut prev: Option<(f64, f64)> = None;
                    let mut kept: Option<(f64, f64)> = None;
                    let last_point = c.4.len().saturating_sub(1);
                    for (i, &(fpr, tpr)) in c.4.iter().enumerate() {
                        let p = (x_of(fpr), y_of(tpr));
                        // Drop points within half a pixel of the last one drawn.
                        if let Some(k) = kept
                            && i != last_point
                            && (p.0 - k.0).abs() < 0.5
                            && (p.1 - k.1).abs() < 0.5
                        {
                            continue;
                        }
                        kept = Some(p);
                        // Step shape: move right, then up.
                        if let Some(q) = prev {
                            pts.push(format!("{:.1},{:.1}", p.0, q.1));
                        }
                        pts.push(format!("{:.1},{:.1}", p.0, p.1));
                        prev = Some(p);
                    }
                    let dash_attr = if dash.is_empty() {
                        String::new()
                    } else {
                        format!(r#" stroke-dasharray="{dash}""#)
                    };
                    let _ = writeln!(
                        s,
                        r#"<polyline class="line {css}"{dash_attr} points="{}"><title>{} - {}% amplitude, {} s</title></polyline>"#,
                        pts.join(" "),
                        class.name(),
                        pct(ppm),
                        secs(ms)
                    );
                }
            }
            if row == 0 {
                let label = if ms < 60_000 {
                    format!("{} s", secs(ms))
                } else {
                    format!("{} min", ms / 60_000)
                };
                let _ = writeln!(
                    s,
                    r#"<text class="t1" x="{}" y="-10" text-anchor="middle" font-weight="600">{label}</text>"#,
                    pw / 2.0
                );
            }
            if col == 0 {
                let _ = writeln!(
                    s,
                    r#"<text class="t1" x="-44" y="{}" text-anchor="middle" font-weight="600" transform="rotate(-90 -44 {})">{}% amplitude</text>"#,
                    ph / 2.0,
                    ph / 2.0,
                    pct(ppm)
                );
                for (v, label) in [(1.0, "1"), (0.5, "0.5"), (0.0, "0")] {
                    let _ = writeln!(
                        s,
                        r#"<text class="t2" x="-6" y="{}" text-anchor="end" dominant-baseline="middle">{label}</text>"#,
                        y_of(v)
                    );
                }
            }
            if row == AMPLITUDES_PPM.len() - 1 {
                for (f, label) in [
                    (1e-4, "0.01%"),
                    (1e-3, "0.1%"),
                    (1e-2, "1%"),
                    (1e-1, "10%"),
                    (1.0, "100%"),
                ] {
                    let _ = writeln!(
                        s,
                        r#"<text class="t2" x="{}" y="{}" text-anchor="middle">{label}</text>"#,
                        x_of(f),
                        ph + 14.0
                    );
                }
            }
            let _ = writeln!(s, "</g>");
        }
    }
    // Legend (identity never by colour alone: each series also has its own dash).
    let ly = top + 4.0 * ph + 3.0 * gap_y + 40.0;
    let mut lx = left;
    for (class, css, dash) in &series {
        let dash_attr = if dash.is_empty() {
            String::new()
        } else {
            format!(r#" stroke-dasharray="{dash}""#)
        };
        let _ = writeln!(
            s,
            r#"<line class="line {css}"{dash_attr} x1="{lx}" y1="{ly}" x2="{}" y2="{ly}"/>"#,
            lx + 28.0
        );
        let _ = writeln!(
            s,
            r#"<text class="t1" x="{}" y="{ly}" dominant-baseline="middle">{}</text>"#,
            lx + 34.0,
            class.name()
        );
        lx += 34.0 + 8.0 * class.name().len() as f64 + 30.0;
    }
    let _ = writeln!(
        s,
        r#"<line class="chance" x1="{lx}" y1="{ly}" x2="{}" y2="{ly}"/><text class="t2" x="{}" y="{ly}" dominant-baseline="middle">chance</text>"#,
        lx + 28.0,
        lx + 34.0
    );
    let _ = writeln!(s, "</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Option<Args>, String> {
        parse(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn requires_config_out_and_seed() {
        assert!(args(&["--out", "d", "--seed", "1"]).is_err());
        let a = args(&["--config", "c.json", "--out", "d", "--seed", "1"])
            .unwrap()
            .unwrap();
        assert_eq!((a.humans, a.cheats, a.fpr), (5_000, 500, 0.001));
        assert!(args(&["--config", "c", "--out", "d", "--seed", "1", "--fpr", "0"]).is_err());
    }

    #[test]
    fn svg_is_well_formed_enough() {
        let curves = vec![(
            5_000,
            30_000,
            "error",
            Class::HumanisedAimbot,
            vec![(0.0, 0.0), (0.001, 0.5), (1.0, 1.0)],
        )];
        let svg = roc_svg(&curves, 0.001, false);
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("<g ").count(), 16);
        assert!(svg.contains("humanised-aimbot"));
    }
}
