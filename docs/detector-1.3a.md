# Input-probe detector: the two weak cases from task 1.3 (task 1.3a)

For: PM. Follows [`detector-1.3.md`](detector-1.3.md), which stays as written. Full
tables: [`results/detector-1.3a/summary.md`](results/detector-1.3a/summary.md) (the
task-1.3 tables, unchanged, then a task-1.3a section) and the CSV files beside it.

> Every number here comes from **simulated** players, as in task 1.3. The human model
> is unfitted and the cheat models are ours. Real-player validation is task 1.10.

## Result

Detection rate at a 0.1% false-positive rate, 500 sessions per cheat against 5,000
simulated humans, with 95% bootstrap intervals. "1.3" is the fire-time statistic from
task 1.3. "New statistic" is the added statistic on its own, calibrated the same way.
"Whole detector" flags when any of the scenario's three statistics passes its own
threshold, each set at a third of the 0.1% budget.

**Recoil macro, 0.5% amplitude**

| | 30 s | 2 min | 5 min | 15 min |
|---|---|---|---|---|
| 1.3 | 0% | 4.4% [1.4, 8.0] | 15.2% [9.0, 24.0] | 69.2% [62.8, 75.2] |
| New statistic (per-shot spray) | 15.8% [12.4, 21.8] | 68.4% [62.4, 74.4] | 93.4% [87.2, 95.6] | 99.8% [99.4, 100] |
| Whole detector | 14.8% [8.6, 18.2] | 60.8% [55.8, 70.6] | 86.8% [78.6, 94.2] | 99.8% [99.2, 100] |

**Adaptive aimbot (task 1.2)**

| | 30 s | 2 min | 5 min | 15 min |
|---|---|---|---|---|
| 1%, 1.3 | 94.4% [90.6, 97.2] | 100% | 100% | 100% |
| 1%, new statistic (drift change) | 100% | 100% | 100% | 100% |
| 1%, whole detector | 100% | 100% | 100% | 100% |
| 2%, 1.3 | 56.2% [45.8, 61.6] | 96.8% [95.0, 98.2] | 100% | 100% |
| 2%, new statistic (drift change) | 100% | 100% | 100% | 100% |
| 2%, whole detector | 100% | 100% | 100% | 100% |

**Fast adaptive aimbot (task 1.2a)**

| | 30 s | 2 min | 5 min | 15 min |
|---|---|---|---|---|
| 1%, 1.3 | 5.4% [2.6, 10.4] | 21.2% [15.2, 30.8] | 59.6% [51.2, 70.6] | 98.8% [97.6, 99.8] |
| 1%, new statistic (drift change) | 100% | 100% | 100% | 100% |
| 1%, whole detector | 100% | 100% | 100% | 100% |
| 2%, 1.3 | 0.2% | 0% | 0% | 0% |
| 2%, new statistic (drift change) | 100% | 100% | 100% | 100% |
| 2%, whole detector | 100% | 100% | 100% | 100% |

Every 100% in the adaptive tables is 500 of 500, with interval [100, 100].

Monitoring continuously over 15 minutes (sequential thresholds, 0.1% shared across the
whole horizon and the three statistics):

| Cheat, amplitude | 1.3: flagged within 15 min, median engagements | 1.3a |
|---|---|---|
| Recoil macro, 0.5% | 33%, 145 bursts | 84.8%, 76 bursts |
| Recoil macro, 1% | 86%, 80 bursts | 100%, 25 bursts |
| Adaptive aimbot, 2% | 99.8%, 73 | 100%, 11 |
| Fast adaptive aimbot, 0.5% | 100%, 903 | 100%, 11 |
| Fast adaptive aimbot, 1% | 25.4%, 32 | 100%, 11 |
| Fast adaptive aimbot, 2% | 12%, 12 | 100%, 11 |

**False positives for simulated humans are not worse than in task 1.3.**

- Fixed observation time: the whole detector flags 0.06% of flick humans and 0.02–0.04%
  of spray humans, in every amplitude and observation time, against a 0.1% target.
  Task 1.3 calibrated each statistic at exactly 0.1%.
- Sequential, 15 minutes: 0.06% flick and 0.04% spray. Task 1.3 measured 0.08% and 0.04%.
- These thresholds are calibrated on the same humans they are measured on, as in task
  1.3. No held-out false-positive measurement was made (see "Not done").

The task-1.3 statistics are untouched: with the new configuration, `roc.csv`,
`detection.csv`, `ttd.csv`, `separation.csv`, `steps.csv` and both plots are
byte-identical to `results/detector-1.3/`.

## What changed

Two statistics were added beside the two from task 1.3. Both use the same test as the
fire-time statistic: a bound on κ (drift gain per degree of unexplained error), a signed
score and a confidence, per window and per session. Each has its own κ bound and flag
score in `DetectorConfig`. They are optional there: a configuration without them, such
as `detector-1.3.json` or the server's example, runs exactly as before.

1. **Drift change (`change`, flick).** A cheat that measures the multiplier its last
   flick produced, and divides the next flick by it, leaves only the drift's *change*
   since that measurement in its aim error. So for each shot the detector pairs the error
   with the drift's effect since the previous shot, minus what a learner that measured
   the movement before that shot would have removed.
   - The adaptive aimbots' κ on this is 29–185/°, against a human median of 0.36/° and a
     bound of 2/°. The task-1.3 statistic failed at high amplitude because their κ there
     falls as 1/amplitude, to about 1/° at 2%.
   - The fitted slope is 1.4, not 1: their estimates lag a smooth drift, so they also
     miss part of the previous change. This does not matter to the test.
2. **Per-shot spray (`spray`).** Within a burst, each shot's change in error is paired
   with what a fixed-pattern macro would get wrong on that kick: the nominal kick times
   (sensitivity multiplier − recoil multiplier). That is about 29 pairs per axis per
   burst, where task 1.3 had one.
   - The macro's slope on this is 1.00 with 0.014° of residual (hand tremor), so κ ≈ 71/°.
     The simulated human's median is 3.3/° (slope 0.29, residual 0.088°). The bound is
     15/°.
   - Two things had to be removed first, both found on the tuning set. A player's
     habitual error on a given shot repeats every burst, so the fit removes a separate
     mean per burst shot and axis. And the regressor is built from the recoil records
     and the probe only, never from the player's own moves, because the drift's effect
     on a noisy pull puts the same noise on both sides of the fit. Before these, a tail
     of simulated humans had κ above 60/°.

The evaluation gained a section for these (new files `detection-1.3a.csv`,
`ttd-1.3a.csv`, `evidence-1.3a.csv`), written only when the configuration enables them.
The server counts the new statistics in a verdict's score and stores their evidence when
they are enabled; its example configuration does not enable them.

## What did not improve, and what it cost

- **The macro at 30 s and at 0.25%.** At 0.5% it is caught 15% of the time in 30 s and
  61–68% in 2 minutes. At 0.25% the whole detector reaches 86% only after 15 minutes of
  continuous spraying, and flags 41% within 15 minutes when monitoring sequentially.
- **Splitting the budget three ways slows the classes the old statistics already
  caught, slightly.** Humanised aimbot at 0.5%: 20.0% at 30 s and 74.0% at 2 min, against
  26.8% and 85.6% for the fire-time statistic alone in task 1.3. Its sequential median
  rises from 221 to 232 engagements, and the smoothing aimbot's from 155 to 167. At 0.25%
  the humanised aimbot is flagged within 15 minutes 75.6% of the time, against 76%.
- **The drift-change statistic does nothing for the humanised aimbot** (its κ there is
  about 3/°, close to the bound) or the smoothing aimbot (0%). The fire-time and step
  statistics still carry those.
- **The amplitude finding of task 1.3 is not revisited.** The adaptive aimbots are no
  longer a reason to prefer a low amplitude, and the macro still favours a high one.
  Amplitude defaults were out of scope and are unchanged.

## How the sessions were split

- **Tuning set:** simulation seed 1, 1,000 humans per scenario and 100 sessions per
  cheat class, plus single-class probes of up to 1,000 sessions. Every design choice was made on it: the form of both
  regressors, the per-shot means, and the spray κ bound of 15/°. The drift-change bound
  of 2/° is the task-1.3 value and was not tuned.
- **Report set:** seed 20260930 at full size, the same sessions as task 1.3. It was run
  after the design was frozen (twice, to check the output is byte-identical). Nothing
  was changed after seeing it.
- The simulator's models were not changed.

## Assumptions and risks

Those of `detector-1.3.md` still hold. In addition:

- **The spray bound rests on the simulated human's per-shot noise** (10–20% of each
  kick). A real player with very consistent recoil control has a higher κ. The bound of
  15/° must be re-set from real players in task 1.10, like the bound of 2/°.
- **Both statistics target the modelled cheat.** A macro that adds its own noise per
  kick, or pulls less than the full pattern, lowers its κ towards the human's. An
  adaptive aimbot that predicts the drift, or measures it without flicking, was not
  modelled.
- **Within a burst the pairs are not independent** (the drift is smooth over several
  shots), so the score overstates the evidence somewhat. The thresholds are calibrated on
  simulated humans, which absorbs this, but the stated confidence should not be read as
  a probability.
- **The statistics are meaningful only when the replay reproduces the view**, that is,
  when `angle_mismatches` is zero. This was already true in task 1.3.

## Not done

- A held-out false-positive measurement (thresholds from one set of humans, applied to
  another). The evaluation calibrates and measures on the same 5,000 humans.
- ROC plots for the new statistics.
- Enabling the new statistics in the server's example configuration, or carrying their
  evidence in the wire protocol's verdict.

## How to reproduce

The evaluation command is the one from task 1.3, with the new configuration and its own
output directory:

```sh
cargo run --release -p rearguard-sim --bin rearguard-eval -- \
  --config crates/rearguard-sim/eval/detector-1.3a.json \
  --out docs/results/detector-1.3a --seed 20260930
```

About 10.5 minutes on 16 threads. The task-1.3 files it writes are byte-identical to
those in `results/detector-1.3/`; the run with `detector-1.3.json` itself was not
repeated.
