# One-command detector evaluation (task 1.12)

For: PM. Full results: [`results/eval-1.12/report.md`](results/eval-1.12/report.md) and
the CSV files and plots beside it. How to run it and how it works:
[`crates/xtask/README.md`](../crates/xtask/README.md).

> **No human sessions exist yet**, and collecting them was out of scope. The committed
> results therefore come from simulated players only, as in tasks 1.3 and 1.3a. The
> human path is built and tested, on inputs made with the simulator in the study's
> export format and, in CI, on a real export from the study build. It has never seen a
> person's data.

## What there is now

```sh
cargo xtask eval --seed N \
  [--humans EXPORTS --study-key KEY] \
  [--bots RECORDINGS --bots-master-secret SECRET]
```

It runs every session through the detector the way a server would, and writes a
Markdown report, CSV tables, a ROC plot, a time-to-detection plot and the calibrated
thresholds. About 70 seconds for the default configuration on 16 threads.

| Acceptance criterion | Where |
|---|---|
| 1. Markdown and CSV, ROC plot, time to detection, breakdown by amplitude | `report.md`, `rates.csv`, `roc.csv` and `roc.svg`, `ttd.csv`, `ttd-curve.csv` and `ttd.svg`; "By drift amplitude" in the report |
| 2. Interval computation tested against reference values | `crates/xtask/src/wilson.rs`: Newcombe (1998), Table I |
| 3. Same inputs, identical output | Tested across thread counts and input locations; the committed results were produced twice and compared byte for byte |
| 4. Human sessions never tune and evaluate in the same run | Split by participant, enforced by types (`crates/xtask/src/split.rs`), checked again at run time, and tested through the whole evaluation |
| 5. Thresholds per scenario, calibrated on real baseline sessions; server supports a set per scenario | The `human` threshold set (needs exports); `rearguard-server`'s `detector` now accepts `{"scenarios": {...}}`, and the evaluation writes that form |
| 6. False-positive rate on held-out simulated humans | 5,000 simulated humans calibrate, another 5,000 measure |

## Result (simulated players, 60-second sessions)

Thresholds calibrated for a 0.1% false-positive rate per 60-second session, per
scenario and amplitude, on 5,000 simulated humans. Rates with 95% Wilson intervals;
500 sessions per cheat. The false-positive rows are the 5,000 held-out simulated humans.

| | 0.25% | 0.5% | 1% | 2% |
|---|---|---|---|---|
| **False positives, flick** | 0.08% [0.03, 0.21] | 0.12% [0.06, 0.26] | 0.02% [0.00, 0.11] | 0.08% [0.03, 0.21] |
| **False positives, spray** | 0.06% [0.02, 0.18] | 0.04% [0.01, 0.15] | 0.02% [0.00, 0.11] | 0.04% [0.01, 0.15] |
| Flick aimbot | 100% | 100% | 100% | 100% |
| Adaptive aimbot | 100% | 100% | 100% | 100% |
| Fast adaptive aimbot | 100% | 100% | 100% | 100% |
| Smoothing aimbot | 33.0% [29.0, 37.2] | 74.4% [70.4, 78.0] | 99.0% [97.7, 99.6] | 100% |
| Humanised aimbot | 1.2% [0.6, 2.6] | 11.2% [8.7, 14.3] | 81.2% [77.5, 84.4] | 99.8% [98.9, 100] |
| Recoil macro (spray) | 13.4% [10.7, 16.7] | 15.4% [12.5, 18.8] | 37.6% [33.5, 41.9] | 83.4% [79.9, 86.4] |

Every 100% is 500 of 500, interval [99.2, 100].

- **The held-out false-positive rates are consistent with the 0.1% target.** All eight
  intervals contain it. This is the measurement task 1.3a could not make.
- **The three computed-flick aimbots are flagged within about two seconds** at every
  amplitude (median 1.0 to 1.9 s, 10 to 13 shots).
- **One minute is short for the other three.** At the default 0.5% amplitude the
  humanised aimbot is flagged 11% of the time and the recoil macro 15%. Tasks 1.3 and
  1.3a reported these classes over 2 to 15 minutes; this evaluation uses 60 seconds
  because that is the length of the study's baseline rounds, which is what real
  thresholds will be calibrated on.

## What the plan should account for

1. **Thresholds at 0.1% rest on the two highest scores of 5,000.** The budget is split
   three ways, so each statistic's threshold sits where one or two calibration sessions
   exceed it. Another simulation seed moved the flick `error` threshold at 0.5% from
   11.9 to 8.3 and the humanised aimbot's detection rate from 11% to 26%. The intervals
   in the table do not include this: they take the thresholds as given. More simulated
   humans (`sim.calibration_humans`) would steady it, at proportional run time.
2. **A study of tens of people cannot establish a 0.1% false-positive rate.** With n
   calibration sessions the threshold can only be "the highest score seen", which
   resolves 1/n. Twenty participants, half of them calibrating, give 20 flick and 10
   spray sessions at 0.5%: a resolution of 5% and 10%. The report prints this beside
   each threshold, and the held-out interval shows what is known (0 flagged of 20 is
   [0%, 16%]). Real thresholds at 0.1% need on the order of thousands of sessions, or a
   model of the score's tail, which is detector work.
3. **The study plays 0.5% only.** People calibrate one amplitude; the other three stay
   simulation-only.
4. **Several sessions from one person are counted as independent** in the intervals. The
   split is by person, but the interval is by session.
5. **The scenario is the client's claim.** With a threshold set per scenario, the server
   picks the set from the telemetry header. A real game's server knows the mode and must
   not take it from the client.
6. **The `steps` statistic is off in spray.** No simulated spray human ever scores on
   it, so there is nothing to calibrate against. The evaluation leaves such a statistic
   off instead of flagging at the first score above zero.

## Changes outside the new crate

- **`rearguard-core`**: new `detect::DetectorSet` (one configuration, or one per
  scenario). Detector logic is unchanged.
- **`rearguard-server`** (public interface): `Config::detector` is now a `DetectorSet`.
  Existing config files still work. With a set per scenario the detector starts when
  the telemetry header arrives; a scenario without a set is refused. Detector
  configurations are now checked when the config file is read.
- **CI**: the Linux Godot job records the test bots and evaluates them; the study-build
  job evaluates the export its smoke test wrote. `scripts/smoke-study-build.sh` gained
  `--keep-export`.
- **New dependency**: `miniz_oxide` 0.9 (MIT OR Zlib OR Apache-2.0), with `adler2` (0BSD
  OR MIT OR Apache-2.0), to inflate the study export zips. `cargo deny` passes.

## Not done

- No human data was evaluated: there is none.
- No test-bot results are committed. Recording them needs Godot, which was not on the
  development machine; CI records one 10-second run per bot and checks that each
  replays, which is a format check, not a measurement.
- `rearguard-eval` (tasks 1.3 and 1.3a) is unchanged and still reproduces those results.
  Its bootstrap intervals and this evaluation's Wilson intervals answer different
  questions and are not comparable.

## How to reproduce

```sh
cargo xtask eval --seed 20260930 --out docs/results/eval-1.12
```

At the revision recorded in the result files, this rewrites them byte for byte.
