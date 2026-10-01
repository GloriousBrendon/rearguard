# xtask

Repository tasks, run with `cargo xtask` (an alias in `.cargo/config.toml`). One task so
far: `cargo xtask eval`, the detector evaluation (task 1.12).

**No Godot dependency.** This crate must never depend on Godot, gdext or any other
engine crate, directly or transitively. `unsafe` code is forbidden.

## `cargo xtask eval`

Runs simulated, test-bot and human sessions through the input-probe detector
(`rearguard_core::detect`) and reports detection rate and false-positive rate together.

```sh
# Simulated players only.
cargo xtask eval --seed 20260930

# With people's study exports and test-bot recordings.
cargo xtask eval --seed 20260930 \
  --humans exports/ --study-key study-key.hex \
  --bots target/bots/recordings --bots-master-secret target/bots/master.hex
```

| Option | Default | Meaning |
|--------|---------|---------|
| `--seed N` | required | Simulation seed. Never written out |
| `--config FILE` | `crates/xtask/eval/eval.json` | The evaluation's configuration (below) |
| `--out DIR` | `target/eval` | Output directory; files are overwritten |
| `--humans PATH` | none | A study export (task 1.9): the zip, an extracted export, or a folder of either. May be repeated |
| `--study-key FILE` | none | The study key of the release the exports came from. May be repeated, one per release; the key that reproduces a session is found by replaying it |
| `--bots DIR` | none | Test-bot recordings from the aim range (task 1.8): `NAME.jsonl` with `NAME.jsonl.env.json`. `scripts/record-test-bots.sh` makes them |
| `--bots-master-secret FILE` | none | The master secret of the server the bots played against |
| `--allow-test-exports` | off | Accept exports that are not from people (automated self-test runs, debug builds). The report says so in its first lines |
| `--strict` | off | Exit with an error if a recorded session had to be left out because something is wrong with it |
| `--threads N` | all cores | Worker threads. The results do not depend on it |

With the default configuration the simulation takes about a minute on 16 threads.

### What it writes

| File | Holds |
|------|-------|
| `report.md` | The report: sessions used and left out, thresholds per scenario, false-positive and detection rates with intervals, the breakdown by drift amplitude, times to detection, both plots, the method, the configuration |
| `rates.csv` | Every rate: threshold set, amplitude, scenario, source, class, kind, sessions, flagged, rate, 95% Wilson interval, and how many each statistic flagged |
| `thresholds.csv` | Every threshold, with the detector flag score it corresponds to and the calibration set's size |
| `thresholds.json` | The same, plus ready-made `rearguard-server` `detector` settings: one threshold set per scenario, for each amplitude |
| `roc.csv`, `roc.svg` | Detection rate against false-positive rate, as the calibration target is varied |
| `ttd.csv`, `ttd-curve.csv`, `ttd.svg` | Time to detection: quantiles, and the share of sessions flagged by each moment |

Every file records the git revision (with `-dirty` if the working tree differs from it,
not counting the output directory), the configuration and the inputs. CSV files carry
them in leading `#` lines (`pandas.read_csv(..., comment="#")`), plots in `<metadata>`.
Inputs are recorded as counts and SHA-256 digests. The simulation seed, keys, paths and
participant ids are never written.

**The same inputs give byte-identical files**: the same revision, configuration, seed
and recorded sessions, on any machine and with any `--threads`. Nothing in a file comes
from a clock, a path or the order files were listed in.

### Configuration

JSON; every field is required and unknown fields are refused.

| Field | Meaning |
|-------|---------|
| `detector` | The detector's parameters that are not thresholds: `kappa_bound`, `min_pairs`, `window_ms`, `min_step_counts`, and `change_kappa_bound` and `spray_kappa_bound` (`null` leaves that statistic out). The flag scores are what the evaluation calibrates |
| `fpr_target` | False-positive rate the thresholds aim for, per session, shared between a scenario's statistics |
| `horizon_s` | Session length evaluated. Simulated sessions run for exactly this long; recorded sessions planned for any other length are left out and counted |
| `amplitudes_ppm` | Drift amplitudes simulated |
| `sim.calibration_humans`, `sim.held_out_humans`, `sim.cheats` | Simulated sessions per scenario (humans) or per cheat class, at every amplitude |
| `human_split.calibration_fraction`, `human_split.salt` | How participants are divided (below) |

`eval/ci-bots.json` and `eval/ci-study.json` are small configurations for the CI checks
that read real aim-range recordings and a real study export.

### Method

- **One path for every session.** A session is telemetry plus the player's drift seed,
  streamed through the detector as a server would. Simulated sessions get their seed
  from the simulated server's key hierarchy; test-bot recordings from the test server's
  master secret; study sessions from the study key. A recorded session is used only if
  the detector's replay reproduces every reported view angle, which shows that key,
  amplitude and telemetry belong together.
- **Flag rule.** A session is flagged if, at any moment up to the horizon, the session
  score of any statistic its scenario runs is above that statistic's threshold. This is
  what a server polling a live verdict sees. The verdict at the end of a session is the
  same test at the last record, so it flags no more often. Per-window evidence is not
  evaluated.
- **Thresholds are per scenario.** Flick sessions run `error`, `steps` and `change`;
  spray sessions run `error`, `steps` and `spray`. Each statistic's threshold is the
  smallest value that at most `fpr_target ÷ number of statistics` of the calibration
  sessions' highest scores exceed (a union bound). A statistic no calibration session
  scored on cannot be calibrated and is left off.
- **Two threshold sets.** `sim`: calibrated on simulated humans. `human`: calibrated on
  people's baseline sessions with the drift on, when exports are given. Each is
  calibrated per amplitude, and measured against every kind of session there is.
- **Calibration and evaluation never share a session.** Simulated humans: the first
  `calibration_humans` session indices calibrate, the next `held_out_humans` measure.
  People: a participant is in the calibration set when a hash of the salt and their id
  falls below `calibration_fraction`, and in the evaluation set otherwise, with all
  their sessions. It depends on nothing else, so adding participants moves nobody; keep
  the salt for the life of a study. In the code the two sets are different types
  (`split::Calibration`, `split::Evaluation`) that only the split can make:
  `analysis::calibrate` accepts only the first and `analysis::false_positives` only the
  second.
- **Intervals.** 95% Wilson score intervals (`wilson.rs`, with the formula and the
  reference values it is tested against). They treat sessions as independent and the
  thresholds as fixed. So they do not include the uncertainty of the thresholds, and
  several sessions from one person count as independent.
- **Small calibration sets.** With `n` calibration sessions no false-positive rate below
  `1/n` can be told from zero. When `1/n` is above the target, the threshold is the
  highest score seen, and the report shows `1/n` beside it. A handful of people cannot
  establish a 0.1% false-positive rate; the held-out interval says how little is known.

### What is left out

By design: sessions with the drift off (there is no drift to follow, so the detector
gives no score), the study's blind-comparison intervals, and the `tracking` scenario.
As problems: incomplete sessions, sessions planned for another length than `horizon_s`,
telemetry that does not parse or disagrees with its manifest, and sessions that do not
replay. The report counts all of them.

Exports that are not from people (automated self-test runs, debug builds) are refused
unless `--allow-test-exports` is given.

### Recording the test bots

```sh
godot --headless --path demo --import          # once per fresh checkout, BEFORE the build
cargo build -p rearguard-godot -p rearguard-server
scripts/record-test-bots.sh godot target/bots 60 5     # 60 s sessions, 5 runs per bot
```

The bots run only in Rearguard's own test environment
(`demo/scripts/test_cheats/README.md`). Their ground-truth labels are read from the
`.env.json` beside each recording, by the evaluation only: the detector sees telemetry
and a seed.

## Tests

`cargo test -p xtask` (about 20 s in a debug build) covers:

- the Wilson interval against Newcombe (1998), Table I, and independently computed values;
- the split: one set per participant, stable under added participants, pinned;
- that human sessions calibrate or evaluate, never both, through the whole evaluation;
- that simulated false-positive rates are measured on held-out humans only;
- byte-identical output for the same inputs, across thread counts and input locations;
- that every file records the revision, configuration and input digests, and no
  participant id, path or key;
- study exports as zips and as extracted folders (the same sessions and digest), wrong
  keys, damaged archives, automated exports, bot recordings and their labels;
- that the flag scores written for the server make the detector itself flag at the
  record the evaluation says, and parse as a server `detector` setting.

The test inputs are made with the simulator in the formats the aim range writes. CI also
runs the evaluation on real aim-range recordings and a real study export (the
`godot tests (linux)` and `study build (export, linux smoke)` jobs).
