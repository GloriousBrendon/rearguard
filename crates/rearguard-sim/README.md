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
| `--classes A,B` | all | Any of `human`, `recoil-macro`, `flick-aimbot`, `humanised-aimbot`, `adaptive-aimbot` |

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

Every aimbot waits for the weapon to be ready and re-aims only after a real miss, so
no flick is corrected before its shot. Per-session parameters for humans and
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

## Tests

`cargo test -p rearguard-sim` runs in a few seconds, and covers:

- the drift-correlation checks (acceptance criterion 2);
- reproducibility, and golden hashes of one short session per class, which pin the
  output across Linux and Windows;
- an exact replay of every telemetry file from raw deltas plus the probe seed, which
  checks the stream carries everything decision D2 needs;
- batch generation.

The two report tests above are `#[ignore]`d. Run them with `--release`.

Determinism across platforms comes from three things:
- every random draw is a ChaCha20 stream keyed by the seed and numbered by purpose;
- every transcendental function comes from `libm` (pure Rust), not the platform
  library;
- the rest is IEEE 754 basic arithmetic.
