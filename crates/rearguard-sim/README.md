# rearguard-sim

Rearguard's own closed test environment. Synthetic players and test cheats play a
simulated aim range under the input-probe drift. They emit exactly the telemetry a real
client emits (`rearguard_core::telemetry`, version 1). Later tasks use this output to
build and measure the detector, always reporting detection rate and false-positive
rate together.

> **Simulated humans are not real humans.** The human model is a plausible
> closed-loop controller built from textbook ingredients. None of its parameters are
> fitted to real players, and nothing it shows is evidence about how real players
> behave. Validation against real players happens in **task 1.10**. Until then, treat
> every result from this crate as a statement about the models, not about people.

Test cheats run only inside this simulator. Nothing here injects real input, reads
another process, or talks to a server or game.

**No Godot dependency.** This crate must never depend on Godot, gdext or any other
engine crate, directly or transitively. `unsafe` code is forbidden.

## Generating sessions

```sh
cargo run --release -p rearguard-sim -- --out /tmp/sim-batch --seed 42 --sessions 10
```

| Option | Default | Meaning |
|--------|---------|---------|
| `--out DIR` | required | Output directory. Refuses a directory that already has a manifest |
| `--seed N` | required | Top-level seed (`u64`). The same seed and options give byte-identical output on every platform |
| `--sessions N` | 10 | Sessions per class *and scenario* |
| `--duration S` | 60 | Session length, seconds of game time |
| `--amplitude-ppm P` | 5000 | Probe drift amplitude, 0 to 20000 |
| `--classes A,B` | all | Any of `human`, `recoil-macro`, `flick-aimbot`, `humanised-aimbot`, `adaptive-aimbot`, `smoothing-aimbot`, `fast-adaptive-aimbot` |
| `--smoothing F` | 0.2 | `smoothing-aimbot`: fraction of the remaining error moved per frame, 0 < F ≤ 1 |
| `--estimation-window-ms W` | 250 | `fast-adaptive-aimbot`: drift estimation window, 1 to 60000 ms |

Output:

- `DIR/<class>/<scenario>-<index>.jsonl`: one telemetry session per file.
- `DIR/manifest.json`: every file with its class label, scenario, match and player
  identifiers, and shot and hit counts.

The telemetry itself never names the class (`source` is always `"sim"`, and match
identifiers are random). The labels live only in the manifest and the directory
names, so keep those away from anything that should judge sessions blind.

The seed is never written out. It determines the simulated server's probe root seed,
so it is handled like a secret: it is wrapped in `SimSeed`, which redacts itself and
is wiped on drop. It is also test data with 64 bits of entropy, so never reuse a sim
seed for anything real. It does appear on the command line, and so in shell history.

## What is simulated

A Rust mirror of the Godot aim range (`demo/`, task 1.4) with the same conventions:

- 120 Hz ticks, one frame per tick, and 600 rounds per minute.
- Flick targets within ±40° yaw and −10° to 20° pitch, replaced after a hit or a 2.5 s
  timeout. Spray rounds use recoil pattern `v1`.
- The same yaw/pitch signs.

The drift is applied at the two hook points: every raw delta is scaled by the
sensitivity drift, and every recoil kick by the recoil drift. Both come from
`rearguard_core::probe` at the event's probe tick.

Input arrives from a 1000 Hz mouse. It is delivered in per-frame batches stamped a few
microseconds apart, as Godot does. Each session's sensitivity is drawn between 0.011
and 0.044° per count. The demo's `tracking` scenario is not simulated yet.

| Class | Scenarios | Model |
|-------|-----------|-------|
| `human` | flick, spray | Closed loop. The player sees the frame shown 100 to 140 ms ago, with visual noise, and subtracts its own recent commands from it (an efference copy, or Smith predictor). After a reaction time it plans a minimum-jerk primary movement with a Fitts'-law duration and noise in amplitude and direction. A continuous feedback term corrects what the plan will miss. Motor noise grows with speed, and there is tremor. The player taps once the estimated error is within tolerance. In spray it pulls against the learned pattern (gain and per-shot noise) while still correcting by sight |
| `recoil-macro` | spray | The same player acquires the target. A macro then injects the exact nominal counts for every shot of pattern `v1`, and the player holds still (with tremor) during the burst |
| `flick-aimbot` | flick | 30 ms after a target appears, one move of the exact counts to the target (from the game's own angles), then a tap |
| `humanised-aimbot` | flick | A log-normal reaction of about 200 ms, then a minimum-jerk path planned once to a random point on the target (σ 0.1° to 0.2°), with per-millisecond jitter, then a click |
| `adaptive-aimbot` | flick | Knows a small drift exists, but not the seed. After each flick it reads the view change its own move caused, keeps a running estimate of the multiplier (weight 0.5 for the newest measurement), and divides the next flick's counts by it. Human-like 150 ms reactions |
| `smoothing-aimbot` | flick | Closed loop (task 1.2a), the common "smooth aim" design. 30 ms after a target appears, on every frame it reads the actual view and moves `--smoothing` of the remaining error toward the target, carrying the rounding remainder. It taps once the actual error is below a quarter of the target radius and the weapon is ready |
| `fast-adaptive-aimbot` | flick | Open-loop flicks with fast drift estimation (task 1.2a). Reacts in 30 ms, so it flicks and measures about every 110 ms. Before each flick it estimates the multiplier by least squares over its own moves observed in the last `--estimation-window-ms` (the newest always counts, so a short window means "the last flick only"), divides the flick's counts by it, and fires at once |

Every aimbot waits for the weapon to be ready. The flicking aimbots re-aim only after
a real miss, so no flick is corrected before its shot. The smoothing aimbot, by
design, corrects every frame, including while it waits for the weapon. Per-session parameters for humans and
humanised aimbots are drawn from the ranges in `human.rs` and `aimbot.rs`.

## Does the error follow the drift?

`validation` compares each shot's aim error with the drift's exact effect on it. That
effect is known from the simulator's drift-free "nominal" view. The comparison is taken
from the moment the target appeared (flick) or from the first shot of the burst (spray).
The null distribution comes from a permutation test that re-pairs whole targets or
bursts. This is ground truth for checking the models, **not a detector**.

Measured with `cargo test --release -p rearguard-sim print_model_report -- --ignored
--nocapture` (seed 1, 5000 ppm), at 30 one-minute sessions per class and scenario:

| Class | Scenario | Shots | r | z | Slope |
|-------|----------|------:|--:|--:|------:|
| human | flick | 2,571 | +0.005 | +0.4 | +0.04 |
| human | spray | 14,678 | −0.011 | −0.5 | −0.22 |
| recoil-macro | spray | 14,675 | +0.112 | +4.2 | +0.69 |
| flick-aimbot | flick | 16,380 | +0.990 | +135.4 | +1.00 |
| humanised-aimbot | flick | 2,552 | +0.319 | +21.2 | +1.01 |
| adaptive-aimbot | flick | 11,563 | +0.438 | +54.2 | +0.26 |
| smoothing-aimbot | flick | 9,520 | −0.015 | −1.7 | −0.09 |
| fast-adaptive-aimbot | flick | 16,380 | +0.173 | +25.9 | +0.07 |

The last two rows use the defaults (smoothing 0.2, window 250 ms). The existing rows
are unchanged from task 1.2.
Slope 1 means the error contains all of the drift's effect, and 0 means none of it.
Human slopes at this volume are very noisy, because the drift's effect on a human is
tiny compared with its other errors.

What the models show (about the models, see the note at the top):

- **The human model leaks a little in flicks.** It taps once the error looks small
  enough, not after correcting fully, so part of the most recent drift survives. Over
  300 sessions (25,692 flick shots, `print_human_leak`) the slope is 0.26 ± 0.03, with
  r = 0.037 and z = 8.4. That is significant from roughly 3,000 flick shots. Spraying
  humans show no leak, because continuous fire removes the tolerance effect. So a
  detector has to judge how strongly the error follows the drift, not just whether it
  does.
- **The adaptive aimbot's slope (0.26) looks like the leaky human's.** Its r (0.43) is
  ten times higher, because its error has almost no other noise. Neither slope nor r
  alone separates them.
- **A macro used with a resting hand is visible only slowly.** The hand drifts more
  over a 3 s burst than a 0.5% drift moves the aim, so r is only about 0.1 to 0.18,
  and it takes about 30 minutes of spraying to reach z ≈ 4 to 6. Over 300 sessions its
  slope is 1.02 ± 0.05.
- **The humanised aimbot passes all of the drift through (slope ≈ 1).** Its random aim
  point dilutes r to about 0.3.

### Task 1.2a: closed-loop smoothing and faster adaptation

From `cargo test --release -p rearguard-sim print_closed_loop_report -- --ignored
--nocapture`. "Shots for z = 4" extrapolates the measured z by the square root of the
shot count. The aim range is dense (this bot fires about 300 to 550 shots a minute);
real matches have far fewer engagements per minute.

| Model, 30 sessions × 60 s unless stated | Shots | r | z | Slope | Shots for z = 4 |
|---|---:|---:|---:|---:|---:|
| smoothing 0.05 | 2,573 | −0.009 | −0.6 | −0.06 | n/a |
| smoothing 0.1 | 5,026 | −0.005 | −0.4 | −0.03 | n/a |
| smoothing 0.2 (default) | 9,520 | −0.016 | −1.7 | −0.09 | n/a |
| smoothing 0.4 | 15,397 | +0.010 | +1.3 | +0.03 | n/a |
| smoothing 0.7 | 15,420 | +0.001 | +0.1 | +0.00 | n/a |
| smoothing 1.0 | 15,420 | +0.002 | +0.4 | +0.00 | n/a |
| smoothing 0.2, 300 sessions (seed 2) | 95,245 | −0.013 | −4.9 | −0.07 | ~64,000 |
| window ≤ 125 ms (last flick only) | 16,380 | +0.089 | +13.0 | +0.03 | ~1,500 |
| window 250 ms (default) | 16,380 | +0.173 | +25.9 | +0.07 | ~390 |
| window 1 s | 16,380 | +0.573 | +91.6 | +0.52 | ~30 |
| window 5 s | 16,380 | +0.867 | +141.8 | +0.87 | ~13 |
| window 20 s | 16,380 | +0.937 | +150.9 | +0.94 | ~12 |
| window 250 ms, 300 sessions (seed 2) | 163,800 | +0.196 | +84.5 | +0.08 | ~370 |
| human flick, 300 sessions (seed 2), for comparison | 25,692 | +0.038 | +8.4 | +0.26 | ~5,800 |

Conclusions (about these models):

- **The closed-loop smoothing aimbot is effectively invisible to this drift
  correlation.** At 30 minutes no smoothing factor from 0.05 to 1.0 comes near the
  null threshold (|z| ≤ 1.7). Every step starts from the view the game really shows,
  so later steps remove what the drift did to earlier ones. Even at smoothing 1.0 the
  bot must wait for the weapon (100 ms between shots) and keeps correcting while it
  waits. Over 5 hours (95,245 shots) a weak *negative* correlation appears (slope
  −0.07, z −4.9), needing about 64,000 shots for |z| = 4. That is ten times the
  volume the human model's own leak needs, and of the opposite sign.

  The fire-time error correlation cannot catch this design. Catching it would need a
  different signature: the drift's effect on the per-frame step sizes, or the
  trajectory shape. That is detector work, out of scope here.
- **The faster adaptive aimbot is visible, and becomes more visible the slower it
  adapts.** With the default 250 ms window it cancels about 93% of the drift (slope
  0.07). Its error has almost no other noise, though, so r stays at 0.17, and about
  400 shots reach z = 4. Using only its last flick (about 110 ms old) it cancels 97%
  (slope 0.03), and it still needs only about 1,500 shots. Windows of a second or
  more average stale measurements and approach the plain flick aimbot.

  By significance alone the fastest variant (about 1,500 shots) is easier to catch
  than the leaky human (about 5,800). By slope it looks *less* like a cheat than the
  human does (0.03 against 0.26). A detector has to weigh slope against the residual
  noise, not either alone.

Limits of the two models:

- **Smoothing aimbot:** no noise, no random aim point, and a fixed 30 ms reaction.
  Real smoothing aimbots add noise and randomise the smoothing, which would only make
  the drift harder to see. It reads the exact view and target, as a memory-reading
  cheat would.
- **Fast adaptive aimbot:** it can measure only through its own flicks, about one
  every 110 ms. A cheat that also injected small probing moves between flicks, or
  read the multiplier straight from the game's memory, could cancel the drift almost
  completely. That is not modelled. Nor is a variant that flicks, re-reads the view
  and corrects before firing; that is closed-loop at fire time and behaves like the
  smoothing aimbot.
- Both play only the flick scenario, and neither is fitted to any real cheat.

## Evaluating the detector (task 1.3)

`rearguard-eval` streams simulated sessions through `rearguard_core::detect`, exactly as a
server would (telemetry plus the player's epoch seed), and writes:
- ROC data and plots;
- detection rates at a fixed false-positive rate, with bootstrap intervals;
- times to detection;
- the slope-versus-significance comparison;
- the step-response results against the smoothing aimbot;
- calibrated thresholds and a summary.

```sh
cargo run --release -p rearguard-sim --bin rearguard-eval -- \
  --config crates/rearguard-sim/eval/detector-1.3.json --out docs/results/detector-1.3 --seed 20260930
```

| Option | Default | Meaning |
|--------|---------|---------|
| `--config FILE` | required | Detector configuration (JSON). Thresholds live only here, never in code |
| `--out DIR` | required | Output directory (files are overwritten) |
| `--seed N` | required | Top-level simulation seed (never written out) |
| `--humans N` | 5000 | Human sessions per scenario and amplitude (the false-positive side) |
| `--cheats N` | 500 | Sessions per cheat class and amplitude |
| `--bootstrap N` | 1000 | Bootstrap rounds |
| `--fpr F` | 0.001 | Target false-positive rate |
| `--engagements-per-match E` | 50 | For converting engagements into matches (an assumption) |
| `--threads N` | all cores | Worker threads; the output does not depend on it |

Each session is 15 minutes, scored at 30 s, 2 min, 5 min and 15 min, at drift
amplitudes of 0.25, 0.5, 1 and 2%. The committed results and the assumptions note are in
`docs/results/detector-1.3/` and `docs/detector-1.3.md`.

## Tests

`cargo test -p rearguard-sim` runs in a few seconds, and covers:

- the drift-correlation checks (acceptance criterion 2);
- reproducibility, and golden hashes of one short session per class, which pin the
  output across Linux and Windows;
- an exact replay of every telemetry file from raw deltas plus the probe seed, which
  checks the stream carries everything decision D2 needs;
- batch generation.

The three report tests (`print_model_report`, `print_human_leak`,
`print_closed_loop_report`) are `#[ignore]`d. Run them with `--release`.

Determinism across platforms comes from three things:
- every random draw is a ChaCha20 stream keyed by the seed and numbered by purpose;
- every transcendental function comes from `libm` (pure Rust), not the platform
  library;
- the rest is IEEE 754 basic arithmetic.
