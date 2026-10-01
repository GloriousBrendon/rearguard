// SPDX-License-Identifier: MIT OR Apache-2.0

//! The result files: a Markdown report, CSV tables, the calibrated thresholds as JSON,
//! and two plots. Every file records the git revision, the configuration and the
//! inputs it was made from, and nothing that changes from run to run.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::json;

use crate::analysis::{Count, quantile};
use crate::config::{EvalConfig, SCENARIOS, STATISTICS};
use crate::eval::{Group, Kind, RateRow, Results, RocRow, TtdRow};
use crate::svg::{self, Grid, SERIES, Series};
use crate::wilson;

/// Steps of the cumulative time-to-detection curve.
const CURVE_STEPS: u32 = 20;

fn amplitude(ppm: u32) -> String {
    format!("{}%", f64::from(ppm) / 10_000.0)
}

fn percent(rate: f64) -> String {
    if rate == 0.0 {
        "0%".to_owned()
    } else if rate == 1.0 {
        "100%".to_owned()
    } else if rate < 0.01 {
        format!("{:.2}%", rate * 100.0)
    } else {
        format!("{:.1}%", rate * 100.0)
    }
}

/// A number as CSV: the shortest text that reads back to the same value (exponent
/// form for very large and very small ones), `inf` for a threshold that never flags.
fn num(value: f64) -> String {
    if value.is_finite() {
        serde_json::to_string(&value).expect("finite numbers serialise")
    } else {
        "inf".to_owned()
    }
}

fn opt(value: Option<f64>) -> String {
    value.map_or(String::new(), num)
}

/// `rate [low, high]` with the 95% Wilson interval, or `-` without sessions.
fn rate_cell(flagged: u64, sessions: u64) -> String {
    match wilson::interval(flagged, sessions) {
        Some((low, high)) => format!(
            "{} [{}, {}]",
            percent(flagged as f64 / sessions as f64),
            percent(low),
            percent(high)
        ),
        None => "-".to_owned(),
    }
}

fn header(git: &str, config: &EvalConfig, results: &Results) -> String {
    format!(
        "# Rearguard detector evaluation (cargo xtask eval, task 1.12). Lines starting with # are not data.\n# git: {git}\n# config: {}\n# inputs: {}\n",
        config.to_json(),
        results.inputs_json
    )
}

fn group_columns(g: &Group) -> String {
    format!(
        "{},{},{},{},{}",
        g.set, g.amplitude_ppm, g.scenario, g.source, g.class
    )
}

fn rates_csv(config: &EvalConfig, results: &Results) -> String {
    let mut s = String::from(
        "threshold_set,amplitude_ppm,scenario,source,class,kind,sessions,flagged,rate,wilson95_low,wilson95_high,flagged_by_error,flagged_by_steps,flagged_by_change,flagged_by_spray\n",
    );
    for row in &results.rates {
        // No interval for the calibration sessions under their own thresholds: the
        // thresholds were fitted to them.
        let interval = match row.kind {
            Kind::CalibrationCheck => None,
            _ => wilson::interval(row.count.flagged, row.count.sessions),
        };
        let stats = config.detector.applicable(row.group.scenario);
        let by: Vec<String> = (0..STATISTICS.len())
            .map(|i| {
                if stats.contains(&i) {
                    row.count.by_statistic[i].to_string()
                } else {
                    String::new()
                }
            })
            .collect();
        let _ = writeln!(
            s,
            "{},{},{},{},{},{},{},{}",
            group_columns(&row.group),
            row.kind.name(),
            row.count.sessions,
            row.count.flagged,
            opt(row.count.rate()),
            opt(interval.map(|i| i.0)),
            opt(interval.map(|i| i.1)),
            by.join(",")
        );
    }
    s
}

fn kappa_bound(config: &EvalConfig, statistic: usize) -> Option<f64> {
    match statistic {
        0 => Some(config.detector.kappa_bound),
        2 => config.detector.change_kappa_bound,
        3 => config.detector.spray_kappa_bound,
        _ => None,
    }
}

fn thresholds_csv(config: &EvalConfig, results: &Results) -> String {
    let mut s = String::from(
        "threshold_set,amplitude_ppm,scenario,statistic,kappa_bound,threshold,flag_score,calibration_sessions,calibration_participants,fpr_target,fpr_share,smallest_resolvable_fpr\n",
    );
    for row in &results.thresholds {
        let t = &row.thresholds;
        let flags = t.flag_scores(f64::MAX);
        for &i in &t.stats {
            let _ = writeln!(
                s,
                "{},{},{},{},{},{},{},{},{},{},{},{}",
                row.group.set,
                row.group.amplitude_ppm,
                row.group.scenario,
                STATISTICS[i],
                opt(kappa_bound(config, i)),
                num(t.values[i]),
                num(flags[i]),
                t.sessions,
                row.participants,
                num(config.fpr_target),
                num(t.share),
                num(1.0 / t.sessions as f64)
            );
        }
    }
    s
}

/// The server's `detector` setting for one scenario: the configuration's parameters
/// with the calibrated flag scores.
fn server_scenario(config: &EvalConfig, scenario: &str, flags: [f64; 4]) -> serde_json::Value {
    let d = &config.detector;
    let mut set = json!({
        "kappa_bound": d.kappa_bound,
        "error_flag_score": flags[0],
        "steps_flag_score": flags[1],
        "min_pairs": d.min_pairs,
        "window_ms": d.window_ms,
        "min_step_counts": d.min_step_counts,
    });
    let extra = match scenario {
        "flick" => d.change_kappa_bound.map(|k| ("change", k, flags[2])),
        _ => d.spray_kappa_bound.map(|k| ("spray", k, flags[3])),
    };
    if let Some((name, kappa_bound, flag_score)) = extra {
        set[name] = json!({"kappa_bound": kappa_bound, "flag_score": flag_score});
    }
    set
}

fn thresholds_json(git: &str, config: &EvalConfig, results: &Results) -> String {
    let thresholds: Vec<serde_json::Value> = results
        .thresholds
        .iter()
        .map(|row| {
            let t = &row.thresholds;
            let flags = t.flag_scores(f64::MAX);
            let statistics: serde_json::Map<String, serde_json::Value> = t
                .stats
                .iter()
                .map(|&i| {
                    (
                        STATISTICS[i].to_owned(),
                        // An uncalibrated statistic (never flags) has no threshold.
                        json!({
                            "calibrated": t.values[i].is_finite(),
                            "threshold": t.values[i].is_finite().then_some(t.values[i]),
                            "flag_score": flags[i],
                        }),
                    )
                })
                .collect();
            json!({
                "threshold_set": row.group.set,
                "amplitude_ppm": row.group.amplitude_ppm,
                "scenario": row.group.scenario,
                "calibration_sessions": t.sessions,
                "calibration_participants": row.participants,
                "statistics": statistics,
            })
        })
        .collect();
    // A server runs one amplitude, with a threshold set per scenario: one entry per
    // threshold set and amplitude that has both scenarios.
    let keys: BTreeSet<(&str, u32)> = results
        .thresholds
        .iter()
        .map(|t| (t.group.set, t.group.amplitude_ppm))
        .collect();
    let mut server = Vec::new();
    for (set, ppm) in keys {
        let scenarios: serde_json::Map<String, serde_json::Value> = results
            .thresholds
            .iter()
            .filter(|t| t.group.set == set && t.group.amplitude_ppm == ppm)
            .map(|t| {
                let flags = t.thresholds.flag_scores(f64::MAX);
                (
                    t.group.scenario.to_owned(),
                    server_scenario(config, t.group.scenario, flags),
                )
            })
            .collect();
        if scenarios.len() == SCENARIOS.len() {
            server.push(json!({
                "threshold_set": set,
                "amplitude_ppm": ppm,
                "detector": {"scenarios": scenarios},
            }));
        }
    }
    let value = json!({
        "about": "Rearguard detector evaluation (cargo xtask eval, task 1.12). A session is flagged when a statistic's session score is above its threshold; flag_score is the next number above the threshold, because the detector flags at score >= flag score. Each server_detector entry's detector object is a rearguard-server `detector` setting for that amplitude.",
        "git": git,
        "config": serde_json::to_value(config).expect("plain data serialises"),
        "inputs": serde_json::from_str::<serde_json::Value>(&results.inputs_json).expect("written as JSON"),
        "fpr_target": config.fpr_target,
        "horizon_s": config.horizon_s,
        "thresholds": thresholds,
        "server_detector": server,
    });
    let mut text = serde_json::to_string_pretty(&value).expect("plain data serialises");
    text.push('\n');
    text
}

fn roc_csv(results: &Results) -> String {
    let mut s = String::from(
        "threshold_set,amplitude_ppm,scenario,source,class,fpr_target,held_out_humans,held_out_flagged,fpr,fpr_wilson95_low,fpr_wilson95_high,sessions,flagged,detection_rate,dr_wilson95_low,dr_wilson95_high\n",
    );
    let triple = |c: &Count| {
        let interval = wilson::interval(c.flagged, c.sessions);
        format!(
            "{},{},{},{},{}",
            c.sessions,
            c.flagged,
            opt(c.rate()),
            opt(interval.map(|i| i.0)),
            opt(interval.map(|i| i.1))
        )
    };
    for row in &results.roc {
        let _ = writeln!(
            s,
            "{},{},{},{}",
            group_columns(&row.group),
            num(row.target),
            triple(&row.negatives),
            triple(&row.positives)
        );
    }
    s
}

fn ttd_csv(results: &Results) -> String {
    let mut s = String::from(
        "threshold_set,amplitude_ppm,scenario,source,class,sessions,flagged,detection_rate,wilson95_low,wilson95_high,seconds_p10,seconds_p25,seconds_p50,seconds_p75,seconds_p90,shots_p50,engagements_p50\n",
    );
    for row in &results.ttd {
        let flagged = row.seconds.len() as u64;
        let interval = wilson::interval(flagged, row.sessions as u64);
        let quantiles: Vec<String> = [0.1, 0.25, 0.5, 0.75, 0.9]
            .iter()
            .map(|q| opt(quantile(&row.seconds, *q)))
            .collect();
        let _ = writeln!(
            s,
            "{},{},{},{},{},{},{},{},{}",
            group_columns(&row.group),
            row.sessions,
            flagged,
            opt((row.sessions > 0).then(|| flagged as f64 / row.sessions as f64)),
            opt(interval.map(|i| i.0)),
            opt(interval.map(|i| i.1)),
            quantiles.join(","),
            opt(quantile(&row.shots, 0.5)),
            opt(quantile(&row.engagements, 0.5))
        );
    }
    s
}

fn ttd_curve_csv(config: &EvalConfig, results: &Results) -> String {
    let mut s = String::from(
        "threshold_set,amplitude_ppm,scenario,source,class,seconds,sessions,flagged_by_then,fraction\n",
    );
    for row in &results.ttd {
        for step in 1..=CURVE_STEPS {
            let seconds = config.horizon_s * f64::from(step) / f64::from(CURVE_STEPS);
            let flagged = row.seconds.iter().filter(|t| **t <= seconds).count();
            let _ = writeln!(
                s,
                "{},{},{},{flagged},{}",
                group_columns(&row.group),
                num(seconds),
                row.sessions,
                opt((row.sessions > 0).then(|| flagged as f64 / row.sessions as f64))
            );
        }
    }
    s
}

/// The threshold sets a plot has rows for, with their row labels.
fn plot_rows(sets: impl Iterator<Item = &'static str>) -> Vec<(&'static str, String)> {
    let present: BTreeSet<&str> = sets.collect();
    [
        ("sim", "thresholds: simulated humans"),
        ("human", "thresholds: people"),
    ]
    .into_iter()
    .filter(|(set, _)| present.contains(set))
    .map(|(set, label)| (set, label.to_owned()))
    .collect()
}

fn plot_metadata(git: &str, config: &EvalConfig, results: &Results) -> String {
    format!(
        r#"{{"git":"{git}","config":{},"inputs":{}}}"#,
        config.to_json(),
        results.inputs_json
    )
}

fn short(git: &str) -> &str {
    &git[..git.len().min(12)]
}

fn roc_svg(git: &str, config: &EvalConfig, results: &Results) -> String {
    // Log axis from 0.01% to 100%; a rate of zero sits on the left edge.
    let x = |fpr: f64| (libm::log10(fpr.max(1e-4)) + 4.0) / 4.0;
    let rows = plot_rows(results.roc.iter().map(|r| r.group.set));
    let panels = rows
        .iter()
        .map(|(set, _)| {
            config
                .amplitudes_ppm
                .iter()
                .map(|&ppm| {
                    SERIES
                        .iter()
                        .enumerate()
                        .filter_map(|(slot, class)| {
                            let curve: Vec<&RocRow> = results
                                .roc
                                .iter()
                                .filter(|r| {
                                    r.group.set == *set
                                        && r.group.amplitude_ppm == ppm
                                        && r.group.source == "sim"
                                        && r.group.class == *class
                                })
                                .collect();
                            (!curve.is_empty()).then(|| Series {
                                slot,
                                points: curve
                                    .iter()
                                    .map(|r| {
                                        (
                                            x(r.negatives.rate().unwrap_or(0.0)),
                                            r.positives.rate().unwrap_or(0.0),
                                        )
                                    })
                                    .collect(),
                                steps: false,
                                labels: curve
                                    .iter()
                                    .map(|r| {
                                        format!(
                                            "{class}, {} amplitude, calibrated at {}: FPR {}/{}, detection {}/{}",
                                            amplitude(ppm),
                                            percent(r.target),
                                            r.negatives.flagged,
                                            r.negatives.sessions,
                                            r.positives.flagged,
                                            r.positives.sessions
                                        )
                                    })
                                    .collect(),
                            })
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    svg::render(&Grid {
        title: format!(
            "Detection rate against false-positive rate, {} s sessions",
            config.horizon_s
        ),
        subtitle: vec![
            "Rows: what the thresholds were calibrated on. Columns: drift amplitude.".to_owned(),
            "x: false-positive rate on held-out humans (log scale; a rate of 0 sits on the left edge). y: detection rate.".to_owned(),
        ],
        rows: rows.into_iter().map(|r| r.1).collect(),
        columns: config.amplitudes_ppm.iter().map(|a| format!("{} amplitude", amplitude(*a))).collect(),
        x_ticks: [(1e-4, "0.01%"), (1e-3, "0.1%"), (1e-2, "1%"), (1e-1, "10%"), (1.0, "100%")]
            .into_iter()
            .map(|(v, label)| (x(v), label.to_owned()))
            .collect(),
        reference: Some((x(config.fpr_target), format!("target false-positive rate ({})", percent(config.fpr_target)))),
        baseline: Some((
            (0..=40).map(|i| {
                let fpr = libm::pow(10.0, -4.0 + f64::from(i) / 10.0);
                (x(fpr), fpr)
            }).collect(),
            "chance".to_owned(),
        )),
        panels,
        footer: format!(
            "Simulated cheats; each point is one calibration target. Numbers: roc.csv. Revision {}.",
            short(git)
        ),
        metadata: plot_metadata(git, config, results),
    })
}

fn ttd_svg(git: &str, config: &EvalConfig, results: &Results) -> String {
    let rows = plot_rows(results.ttd.iter().map(|r| r.group.set));
    let panels = rows
        .iter()
        .map(|(set, _)| {
            config
                .amplitudes_ppm
                .iter()
                .map(|&ppm| {
                    SERIES
                        .iter()
                        .enumerate()
                        .filter_map(|(slot, class)| {
                            let row: &TtdRow = results.ttd.iter().find(|r| {
                                r.group.set == *set
                                    && r.group.amplitude_ppm == ppm
                                    && r.group.source == "sim"
                                    && r.group.class == *class
                            })?;
                            let n = row.sessions.max(1) as f64;
                            let mut points = vec![(0.0, 0.0)];
                            points.extend(
                                row.seconds
                                    .iter()
                                    .enumerate()
                                    .map(|(k, t)| (t / config.horizon_s, (k + 1) as f64 / n)),
                            );
                            points.push((1.0, row.seconds.len() as f64 / n));
                            Some(Series {
                                slot,
                                points,
                                steps: true,
                                labels: vec![format!(
                                    "{class}, {} amplitude: {} of {} flagged within {} s",
                                    amplitude(ppm),
                                    row.seconds.len(),
                                    row.sessions,
                                    config.horizon_s
                                )],
                            })
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    svg::render(&Grid {
        title: format!(
            "Time to detection: share of cheat sessions flagged so far, at a {} false-positive target",
            percent(config.fpr_target)
        ),
        subtitle: vec![
            "Rows: what the thresholds were calibrated on. Columns: drift amplitude.".to_owned(),
            "x: time since the session started. y: share of sessions flagged by then.".to_owned(),
        ],
        rows: rows.into_iter().map(|r| r.1).collect(),
        columns: config
            .amplitudes_ppm
            .iter()
            .map(|a| format!("{} amplitude", amplitude(*a)))
            .collect(),
        x_ticks: (0..=4)
            .map(|i| {
                let f = f64::from(i) / 4.0;
                (f, format!("{} s", config.horizon_s * f))
            })
            .collect(),
        reference: None,
        baseline: None,
        panels,
        footer: format!(
            "Simulated cheats. Numbers: ttd.csv and ttd-curve.csv. Revision {}.",
            short(git)
        ),
        metadata: plot_metadata(git, config, results),
    })
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::CalibrationCheck => "calibration check (not a measurement)",
        Kind::FalsePositive => "false-positive rate (held out)",
        Kind::Control => "scripted player flagged (control)",
        Kind::Detection => "detection rate",
    }
}

fn source_label(row: &RateRow) -> String {
    match (row.group.source, row.kind) {
        ("sim", Kind::CalibrationCheck) => "simulated humans, calibration set".to_owned(),
        ("sim", Kind::FalsePositive) => "simulated humans, held out".to_owned(),
        ("human", Kind::CalibrationCheck) => "people, calibration set".to_owned(),
        ("human", _) => "people, evaluation set".to_owned(),
        ("sim", _) => format!("{} (simulated)", row.group.class),
        (_, Kind::Control) => "scripted player (aim range)".to_owned(),
        _ => format!("{} (aim-range test bot)", row.group.class),
    }
}

fn set_title(set: &str) -> &'static str {
    if set == "sim" {
        "Thresholds calibrated on simulated humans"
    } else {
        "Thresholds calibrated on people"
    }
}

fn markdown(git: &str, config: &EvalConfig, results: &Results) -> String {
    let mut s = String::new();
    let sets: Vec<&str> = ["sim", "human"]
        .into_iter()
        .filter(|set| results.thresholds.iter().any(|t| t.group.set == *set))
        .collect();
    let people = results.data.iter().any(|d| d.source == "human");
    let _ = writeln!(s, "# Detector evaluation (task 1.12)\n");
    let _ = writeln!(
        s,
        "Written by `cargo xtask eval`; rerun it instead of editing this file. Revision `{git}`.\n"
    );
    if !people {
        let _ = writeln!(
            s,
            "> **No human sessions were given.** Every threshold and every false-positive rate below comes from simulated humans, whose model is not fitted to real players. Nothing here is evidence about people.\n"
        );
    }
    if results.test_exports > 0 {
        let _ = writeln!(
            s,
            "> **{} of the exports are not from people** (automated self-test runs or debug builds, allowed with `--allow-test-exports`). Rows labelled \"people\" are not study data in this run.\n",
            results.test_exports
        );
    }
    let _ = writeln!(
        s,
        "Detection rates and false-positive rates are per session of {} s, at thresholds calibrated for a false-positive rate of {}. Intervals are 95% Wilson score intervals. A false-positive rate is only ever measured on sessions that played no part in setting the thresholds.\n",
        config.horizon_s,
        percent(config.fpr_target)
    );

    let _ = writeln!(s, "## Sessions\n");
    let _ = writeln!(
        s,
        "| Source | Used for | Amplitude | Scenario | Class | Sessions | People |\n|---|---|---|---|---|---:|---:|"
    );
    for d in &results.data {
        let source = match d.source {
            "sim" => "simulator",
            "bot" => "aim-range test bots",
            _ => "people (study exports)",
        };
        let people = if d.source == "human" {
            d.participants.to_string()
        } else {
            "-".to_owned()
        };
        let _ = writeln!(
            s,
            "| {source} | {} | {} | {} | {} | {} | {people} |",
            d.role,
            amplitude(d.amplitude_ppm),
            d.scenario,
            d.class,
            d.sessions
        );
    }
    let _ = writeln!(s);
    if !results.excluded.0.is_empty() {
        let _ = writeln!(s, "Recorded sessions left out:\n");
        let _ = writeln!(s, "| Source | Reason | Sessions | |\n|---|---|---:|---|");
        for ((source, reason, by_design), n) in &results.excluded.0 {
            let note = if *by_design {
                "by design"
            } else {
                "**problem**"
            };
            let _ = writeln!(s, "| {source} | {reason} | {n} | {note} |");
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## Thresholds, per scenario\n");
    let _ = writeln!(
        s,
        "A session is flagged when a statistic's session score goes above its threshold. With n calibration sessions no false-positive rate below 1/n can be told from zero: when 1/n is above the target, the threshold is just the highest score seen, and the target is not met by calibration alone. \"off\": no calibration session scored on that statistic, so it has no threshold and never flags.\n"
    );
    let _ = writeln!(
        s,
        "| Calibrated on | Amplitude | Scenario | Sessions | People | Smallest resolvable rate | Thresholds |\n|---|---|---|---:|---:|---:|---|"
    );
    for row in &results.thresholds {
        let t = &row.thresholds;
        let values: Vec<String> = t
            .stats
            .iter()
            .map(|&i| {
                if t.values[i].is_finite() {
                    format!("{} {:.3}", STATISTICS[i], t.values[i])
                } else {
                    format!("{} off", STATISTICS[i])
                }
            })
            .collect();
        let who = if row.group.set == "sim" {
            "simulated humans"
        } else {
            "people"
        };
        let people = if row.group.set == "sim" {
            "-".to_owned()
        } else {
            row.participants.to_string()
        };
        let _ = writeln!(
            s,
            "| {who} | {} | {} | {} | {people} | {} | {} |",
            amplitude(row.group.amplitude_ppm),
            row.group.scenario,
            t.sessions,
            percent(1.0 / t.sessions as f64),
            values.join(", ")
        );
    }
    let _ = writeln!(
        s,
        "\nFull precision, and the matching `rearguard-server` settings (one threshold set per scenario): `thresholds.csv`, `thresholds.json`.\n"
    );

    let _ = writeln!(s, "## False-positive and detection rates\n");
    for set in &sets {
        let _ = writeln!(s, "### {}\n", set_title(set));
        let _ = writeln!(
            s,
            "| Amplitude | Scenario | Who | What | Flagged | Rate [95% Wilson] |\n|---|---|---|---|---:|---|"
        );
        for row in results.rates.iter().filter(|r| r.group.set == *set) {
            let rate = match row.kind {
                Kind::CalibrationCheck => row.count.rate().map_or("-".to_owned(), percent),
                _ => rate_cell(row.count.flagged, row.count.sessions),
            };
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {}/{} | {rate} |",
                amplitude(row.group.amplitude_ppm),
                row.group.scenario,
                source_label(row),
                kind_label(row.kind),
                row.count.flagged,
                row.count.sessions
            );
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## By drift amplitude\n");
    for set in &sets {
        let rows: Vec<&RateRow> = results
            .rates
            .iter()
            .filter(|r| r.group.set == *set && r.kind != Kind::CalibrationCheck)
            .collect();
        let amplitudes: BTreeSet<u32> = rows.iter().map(|r| r.group.amplitude_ppm).collect();
        let mut lines: BTreeMap<(Kind, &str, &str, &str), BTreeMap<u32, &RateRow>> =
            BTreeMap::new();
        for row in &rows {
            lines
                .entry((
                    row.kind,
                    row.group.source,
                    row.group.scenario,
                    &row.group.class,
                ))
                .or_default()
                .insert(row.group.amplitude_ppm, row);
        }
        let _ = writeln!(s, "### {}\n", set_title(set));
        let heads: Vec<String> = amplitudes.iter().map(|a| amplitude(*a)).collect();
        let _ = writeln!(
            s,
            "| What | Who | Scenario | {} |\n|---|---|---|{}",
            heads.join(" | "),
            "---|".repeat(heads.len())
        );
        for ((kind, _, scenario, _), cells) in &lines {
            let first = cells.values().next().expect("a line has a cell");
            let cells: Vec<String> = amplitudes
                .iter()
                .map(|a| {
                    cells.get(a).map_or("-".to_owned(), |r| {
                        rate_cell(r.count.flagged, r.count.sessions)
                    })
                })
                .collect();
            let _ = writeln!(
                s,
                "| {} | {} | {scenario} | {} |",
                kind_label(*kind),
                source_label(first),
                cells.join(" | ")
            );
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## Time to detection\n");
    let _ = writeln!(
        s,
        "Seconds from the start of a session to its first flag, for the cheat sessions flagged within {} s. The plot shows the whole distribution: the share of sessions flagged so far.\n",
        config.horizon_s
    );
    let _ = writeln!(s, "![Time to detection](ttd.svg)\n");
    let _ = writeln!(
        s,
        "| Thresholds from | Amplitude | Cheat | Source | Flagged | 10% | Median | 90% | Median shots | Median engagements |\n|---|---|---|---|---:|---:|---:|---:|---:|---:|"
    );
    let seconds = |v: Option<f64>| v.map_or("-".to_owned(), |x| format!("{x:.1} s"));
    let whole = |v: Option<f64>| v.map_or("-".to_owned(), |x| format!("{x:.0}"));
    for row in &results.ttd {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {}/{} | {} | {} | {} | {} | {} |",
            if row.group.set == "sim" {
                "simulated humans"
            } else {
                "people"
            },
            amplitude(row.group.amplitude_ppm),
            row.group.class,
            if row.group.source == "sim" {
                "simulator"
            } else {
                "aim-range test bot"
            },
            row.seconds.len(),
            row.sessions,
            seconds(quantile(&row.seconds, 0.1)),
            seconds(quantile(&row.seconds, 0.5)),
            seconds(quantile(&row.seconds, 0.9)),
            whole(quantile(&row.shots, 0.5)),
            whole(quantile(&row.engagements, 0.5))
        );
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "## Detection rate against false-positive rate\n");
    let _ = writeln!(
        s,
        "The thresholds are re-calibrated at a range of false-positive targets; each point is the false-positive rate then measured on held-out humans and the detection rate on the cheats. Numbers and intervals: `roc.csv`.\n"
    );
    let _ = writeln!(
        s,
        "![Detection rate against false-positive rate](roc.svg)\n"
    );

    let _ = writeln!(s, "## Method\n");
    let _ = writeln!(
        s,
        "- **Sessions through the detector.** Every session, simulated or recorded, is streamed through `rearguard_core::detect` with the player's drift seed, as a server would. Recorded sessions are used only if the detector's replay reproduces every reported view angle.\n\
         - **Flag rule.** A session is flagged if, at any moment in its {} s, the session score of any statistic its scenario runs is above that statistic's threshold. Flick sessions run `error`, `steps` and `change`; spray sessions run `error`, `steps` and `spray` (those the configuration enables).\n\
         - **Calibration.** Per threshold set, amplitude and scenario: each statistic's threshold is the smallest value that at most (target ÷ number of statistics) of the calibration sessions' highest scores exceed. Together they flag at most the target share of the calibration sessions.\n\
         - **Statistics without a null.** A statistic no calibration session scored on (every highest score zero) cannot be calibrated; it is left off for that scenario and never flags.\n\
         - **Split.** Thresholds come from a calibration set; false-positive rates are measured on other sessions. Simulated humans: separate session indices. People: each participant is in one set only, chosen by a hash of the split's salt and the participant id (`human_split` in the configuration); the code cannot calibrate on an evaluation session, because the two sets are different types. The \"calibration check\" rows show the calibration sessions under their own thresholds; they are at most the target by construction and are not measurements.\n\
         - **Intervals.** 95% Wilson score interval for k flagged of n: with p = k/n and z = 1.959964, centre = (p + z²/2n) / (1 + z²/n), half-width = z·sqrt(p(1−p)/n + z²/4n²) / (1 + z²/n). It treats the sessions as independent and the thresholds as fixed, so it does not include the uncertainty of the thresholds themselves, and several sessions from one person count as independent.\n\
         - **Not evaluated.** Drift-off sessions (no drift to follow, so no score), the study's blind-comparison intervals, the `tracking` scenario, and per-window evidence.\n",
        config.horizon_s
    );
    let _ = writeln!(s, "## Configuration and inputs\n");
    let _ = writeln!(
        s,
        "```json\n{}\n```\n",
        serde_json::to_string_pretty(config).expect("plain data serialises")
    );
    let _ = writeln!(
        s,
        "Inputs (counts and SHA-256 digests; the simulation seed, keys, paths and participant ids are not recorded):\n\n```json\n{}\n```",
        results.inputs_json
    );
    s
}

/// Every result file, by name.
#[must_use]
pub fn render(git: &str, config: &EvalConfig, results: &Results) -> BTreeMap<&'static str, String> {
    let head = header(git, config, results);
    let csv = |body: String| format!("{head}{body}");
    BTreeMap::from([
        ("report.md", markdown(git, config, results)),
        ("rates.csv", csv(rates_csv(config, results))),
        ("thresholds.csv", csv(thresholds_csv(config, results))),
        ("thresholds.json", thresholds_json(git, config, results)),
        ("roc.csv", csv(roc_csv(results))),
        ("roc.svg", roc_svg(git, config, results)),
        ("ttd.csv", csv(ttd_csv(results))),
        ("ttd-curve.csv", csv(ttd_curve_csv(config, results))),
        ("ttd.svg", ttd_svg(git, config, results)),
    ])
}

#[cfg(test)]
mod tests {
    use rearguard_core::detect::DetectorSet;

    use super::*;
    use crate::config::tests::small;
    use crate::eval::tests::{inputs, shared};
    use crate::eval::{Inputs, evaluate};

    const GIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn the_same_inputs_give_byte_identical_files() {
        let config = small();
        let first = render(GIT, &config, shared());
        // Again: other directories for the inputs, another thread count.
        let second = render(
            GIT,
            &config,
            &evaluate(&config, inputs("report-again", 1)).unwrap(),
        );
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            second.keys().collect::<Vec<_>>()
        );
        for (name, body) in &first {
            assert_eq!(body, &second[name], "{name} differs between runs");
        }
        // And the inputs matter: another simulation seed changes the numbers.
        let other = Inputs {
            seed: rearguard_sim::seed::SimSeed::new(8),
            humans: None,
            bots: None,
            threads: 5,
        };
        let third = render(GIT, &config, &evaluate(&config, other).unwrap());
        assert_ne!(first["rates.csv"], third["rates.csv"]);
    }

    #[test]
    fn every_file_records_the_revision_configuration_and_inputs() {
        let config = small();
        let results = shared();
        let files = render(GIT, &config, results);
        assert_eq!(files.len(), 9);
        let sha = &results.inputs_json[results.inputs_json.find("sha256").unwrap()..][9..73];
        for (name, body) in &files {
            assert!(body.contains(GIT), "{name}: revision");
            assert!(
                body.contains("rearguard-eval-1.12"),
                "{name}: configuration (the split's salt)"
            );
            assert!(body.contains(sha), "{name}: input digest");
            assert!(body.ends_with('\n') && !body.contains('\r'), "{name}");
            // Never a participant, a path or a key.
            assert!(
                !body.contains("P-00") && !body.contains("xtask-tests"),
                "{name}"
            );
            assert!(
                !body.contains(&crate::inputs::tests::KEY_HEX[..16]),
                "{name}"
            );
        }
        // CSV: comment lines, a header, then rows with as many fields as the header.
        for (name, body) in files.iter().filter(|(n, _)| n.ends_with(".csv")) {
            let mut lines = body.lines().skip_while(|l| l.starts_with('#'));
            let columns = lines.next().unwrap().split(',').count();
            let rows: Vec<&str> = lines.collect();
            assert!(!rows.is_empty(), "{name}");
            assert!(
                rows.iter().all(|r| r.split(',').count() == columns),
                "{name}"
            );
        }
    }

    #[test]
    fn the_report_shows_rates_together_with_intervals_and_plots() {
        let config = small();
        let results = shared();
        let files = render(GIT, &config, results);
        let report = &files["report.md"];
        for needle in [
            "## False-positive and detection rates",
            "### Thresholds calibrated on simulated humans",
            "### Thresholds calibrated on people",
            "## By drift amplitude",
            "## Time to detection",
            "![Time to detection](ttd.svg)",
            "![Detection rate against false-positive rate](roc.svg)",
            "false-positive rate (held out)",
            "detection rate",
            "flick-aimbot (aim-range test bot)",
            "scripted player (aim range)",
            "95% Wilson score interval",
            "| bot | planned length differs from horizon_s | 1 | **problem** |",
            "by design",
        ] {
            assert!(report.contains(needle), "report lacks: {needle}");
        }
        // Each threshold set's section has false-positive and detection rows.
        for section in report.split("\n### ").skip(1).take(2) {
            assert!(section.contains("false-positive rate") && section.contains("detection rate"));
        }
        assert!(!report.contains("No human sessions were given"));
        // 3 of 3 flagged prints with its Wilson interval.
        assert!(report.contains("3/3 | 100% [43.9%, 100%]"), "{report}");
        // Plots: a row per threshold set, a column per amplitude.
        for plot in ["roc.svg", "ttd.svg"] {
            assert_eq!(files[plot].matches("<g ").count(), 4, "{plot}");
            assert!(files[plot].contains("thresholds: people"));
        }
        // The amplitude breakdown has a column per amplitude.
        assert!(report.contains("| What | Who | Scenario | 1% | 2% |"));
    }

    #[test]
    fn without_people_the_report_says_so() {
        let config = small();
        let mut only_sim = inputs("report-sim", 4);
        only_sim.humans = None;
        only_sim.bots = None;
        let results = evaluate(&config, only_sim).unwrap();
        let files = render(GIT, &config, &results);
        assert!(files["report.md"].contains("No human sessions were given"));
        assert!(!files["report.md"].contains("calibrated on people"));
        assert!(results.thresholds.iter().all(|t| t.group.set == "sim"));
        assert_eq!(files["roc.svg"].matches("<g ").count(), 2);
    }

    #[test]
    fn the_server_settings_parse_as_one_threshold_set_per_scenario() {
        let results = shared();
        let files = render(GIT, &small(), results);
        let json: serde_json::Value = serde_json::from_str(&files["thresholds.json"]).unwrap();
        let entries = json["server_detector"].as_array().unwrap();
        // sim: two amplitudes; people: the one they played.
        assert_eq!(entries.len(), 3);
        for entry in entries {
            let set: DetectorSet = serde_json::from_value(entry["detector"].clone()).unwrap();
            set.validate().unwrap();
            let (flick, spray) = (
                set.for_scenario("flick").unwrap(),
                set.for_scenario("spray").unwrap(),
            );
            assert!(flick.change.is_some() && flick.spray.is_none());
            assert!(spray.spray.is_some() && spray.change.is_none());
            assert_ne!(flick.error_flag_score, spray.error_flag_score);
            // The flag score is the next number above the threshold in thresholds.csv.
            let row = results
                .thresholds
                .iter()
                .find(|t| {
                    t.group.set == entry["threshold_set"]
                        && u64::from(t.group.amplitude_ppm) == entry["amplitude_ppm"]
                        && t.group.scenario == "spray"
                })
                .unwrap();
            let error = row.thresholds.values[0];
            let expected = if error.is_finite() {
                error.next_up()
            } else {
                f64::MAX
            };
            assert_eq!(spray.error_flag_score, expected);
            assert_eq!(
                spray.spray.unwrap().flag_score,
                row.thresholds.values[3].next_up()
            );
        }
    }
}
