# Input-probe detector: offline results and assumptions (task 1.3)

For: PM, to ratify the provisional Phase 1 targets. Full tables and plots:
[`results/detector-1.3/summary.md`](results/detector-1.3/summary.md), `roc.svg`,
`roc-steps.svg`, and the CSV files beside them. Code: `rearguard_core::detect` (the
detector) and `rearguard-sim`'s `rearguard-eval` (the evaluation).

> Every number here comes from **simulated** players. The human model is unfitted, and
> the cheat models are ours. Treat them as a statement about the detector against these
> models, not a forecast for real players. Real-player validation is task 1.10.

## What the detector does

The server replays each player's untrusted telemetry with and without the secret drift,
and compares the two. It computes two statistics as a stream:

1. **Fire-time drift correlation.** For each shot it pairs the aim error with the drift's
   exact effect on the aim. The null hypothesis is a strength bound, not zero
   correlation: a closed-loop player's drift gain per degree of unexplained error
   (κ = slope ÷ residual sd) is at most 2/°. The simulated human's median is 0.68/°.
   - Plain significance is not enough. The simulated human passes about a fifth of
     the recent drift through (slope 0.2), so its plain z keeps growing with time:
     +1.7 at 15 min at 0.5% amplitude, and +6.4 at 2%.
   - The κ test's human z instead falls without bound (−3.2 and −12.6).
2. **Step response.** For consecutive per-frame raw steps it pairs the drift with the
   log ratio of the step lengths. Only steps of at least 100 counts are used, because
   smaller steps are dominated by rounding.
   - A controller that re-reads the view every frame (a smoothing aimbot) responds to
     the drift within one frame, and no human can.
   - The null is zero correlation.

Each statistic gives a signed log-likelihood-ratio score (z²/2 with the sign of z) and a
confidence Φ(z), per window and per session. Thresholds come only from configuration
(`crates/rearguard-sim/eval/detector-1.3.json`, and the calibrated values in
`thresholds.json`). The configuration has no defaults and is never sent to a client.

## Headline (0.5% drift amplitude, 0.1% false-positive rate)

Detection rate at a fixed observation time, with 95% bootstrap intervals, against 5,000
simulated humans per scenario:

| Cheat | 30 s | 2 min | 5 min | 15 min |
|---|---|---|---|---|
| Computed-flick aimbot | 100% | 100% | 100% | 100% |
| Adaptive aimbot (task 1.2) | 98.8% [97.6, 99.6] | 100% | 100% | 100% |
| Humanised aimbot | 26.8% [18.4, 31.8] | 85.6% [78.0, 92.4] | 99.8% [99.4, 100] | 100% |
| Fast adaptive aimbot (task 1.2a) | 35.8% [26.6, 41.0] | 91.8% [84.8, 95.2] | 100% | 100% |
| Smoothing aimbot, step statistic (task 1.2a) | 94.6% [92.6, 96.8] | 100% | 100% | 100% |
| Smoothing aimbot, fire-time statistic | 4.6% | 2.2% | 0.2% | 0% |
| Recoil macro (resting hand) | 0% | 4.4% [1.4, 8.0] | 15.2% [9.0, 24.0] | 69.2% [62.8, 75.2] |

Time to detection when both statistics monitor continuously, with the 0.1% budget
shared across the whole 15 minutes (measured human false-positive rate 0.08% flick,
0.04% spray), in median engagements:

| Cheat | Median engagements (90th percentile) | Flagged within 15 min |
|---|---|---|
| Computed-flick aimbot | 10 (10) | 100% |
| Adaptive aimbot | 43 (134) | 100% |
| Smoothing aimbot (step statistic) | 155 (502) | 100% |
| Humanised aimbot | 221 (503) | 100% |
| Fast adaptive aimbot | 903 (2,048) | 100% |
| Recoil macro | 145 (216) bursts, where flagged | 33% |

At the assumed 50 aimed engagements per match (see below), those medians are 0.2, 0.9,
3.1, 4.4 and 18 matches. The macro is flagged in only a third of cases within 15
minutes of continuous spraying.

## Findings the targets should account for

1. **The best amplitude depends on the cheat.**
   - Open-loop cheats with noisy aim (the humanised aimbot, the macro) get easier to
     catch as the amplitude rises. The humanised aimbot is caught 99.8% of the time
     within 30 s at 2%, and the macro 90% within 2 min at 2%.
   - Adaptive cheats get *harder* to catch. Their leftover error is their own
     drift-estimation lag, which grows with the drift, so their κ falls as
     1/amplitude. For the fast adaptive aimbot's 15-minute medians: 6.9 at 0.25%,
     4.0 at 0.5%, 2.1 at 1%, 1.1 at 2%.
   - As a result, the fast adaptive aimbot is caught 100% at 0.25–0.5% but **0% at 2%**.
     The task-1.2 adaptive aimbot also slows at 2% (56% at 30 s).
   - 0.5% is the best single setting in this grid: nothing but the macro drops below
     90% by 5 minutes.
2. **Weighing slope against significance is needed, and it has a cost.**
   - A slope-only test misses both adaptive aimbots entirely: their slopes (0.08 and
     0.25) are at or below the leaky human's (0.2).
   - Plain significance, calibrated per condition, catches the fast adaptive aimbot
     everywhere. But its threshold would have to rise without limit as evidence
     accumulates across matches, because the human's plain z keeps growing.
   - The κ test stays safe as evidence accumulates, and it is what fails at high
     amplitude.
   - A better statistic for adaptive cheats is plausible: regress the error on the
     drift's change since the previous engagement, not only on its total. That
     belongs to a follow-up task and is not built here.
3. **The smoothing aimbot, invisible to the fire-time statistic, is caught by the step
   statistic.** It is caught at every amplitude, with no measurable false-positive cost:
   0.00% of spray humans cross the flick-calibrated threshold. This depends on the
   aimbot having no noise. A smoothing bot that randomises its gain per frame, or
   reacts slower than a frame, was not modelled and would weaken it.
4. **Recoil macros are the slowest class.** One pair per burst keeps the statistic
   honest, but it is data-hungry. The resting hand's drift over a 3 s burst is larger
   than a 0.5% drift's effect on the aim.

Monotonicity (criterion 4): the planted-signal property tests in `rearguard-core` pass
for players beyond the null. In the simulation grid the rates rise with observation
time and amplitude, with the violations listed in the summary. Those are the
adaptive-aimbot amplitude inversions (finding 1), and noise on the fire-time rows of
the smoothing aimbot, a statistic that isn't meant to catch it.

## Assumptions

- **Simulated humans are the false-positive population.** There are 5,000 per
  scenario, amplitude and observation time. At 0.1% that is about five exceedances, so
  the thresholds themselves are uncertain; the bootstrap intervals include that.
- **Human heterogeneity.** The simulated humans' κ clusters around 0.68/° (median at every amplitude). Real
  players who are more precise (smaller residual error) with the same leak would have
  a higher κ. The bound of 2/° must be re-set from real players in task 1.10, or the
  false-positive rate will not hold.
- **Engagements.** One flick target, or one spray burst, counts as one aimed engagement.
  A real match is assumed to contain 50 of them (`--engagements-per-match`). The aim
  range is dense: 86 flicks a minute for the simulated human, and 390–545 for the fast
  aimbots. So minutes of aim-range time are not minutes of play; use engagements.
- **Server knowledge.**
  - The detector takes target angles from the telemetry's fire records. A real server
    knows them from the game state.
  - It takes `deg_per_count` from the header. A real server should hold the player's
    sensitivity itself.
  - It replays views from raw deltas and counts disagreements with the reported
    angles (none in simulation).
- **Probe.** The default band-limited noise shape (500 ms knots), one epoch per
  session, and the same amplitude for sensitivity and recoil.
- **Horizon.** Sequential thresholds were calibrated for 15 minutes. Longer
  accumulation needs its own calibration. The κ test is built to stay safe there;
  plain significance is not.
- **Cheat models.** The models from tasks 1.2 and 1.2a at their defaults:
  - smoothing 0.2;
  - estimation window 250 ms;
  - no noise on the smoothing aimbot;
  - no cheat that reads the multiplier from memory or probes it between flicks.

## How to reproduce

```sh
cargo run --release -p rearguard-sim --bin rearguard-eval -- \
  --config crates/rearguard-sim/eval/detector-1.3.json \
  --out docs/results/detector-1.3 --seed 20260930
```

The run takes about 9.5 minutes on 16 threads. The output does not depend on the number
of threads.
