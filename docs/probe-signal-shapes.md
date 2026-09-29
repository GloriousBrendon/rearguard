# Input probe: choosing the drift signal shape

Task 1.1 · status: recommendation · code: `crates/rearguard-core/src/probe/`

## Question

The input probe multiplies mouse sensitivity and recoil scale by `1 + d(t)`. Here
`d(t)` is a secret, server-seeded drift with a hard peak cap (default 0.5%, maximum
2%). A detector later correlates each player's aim error with their own `d(t)`. Which
shape should `d(t)` have?

Both candidates are implemented behind `SignalShape`, with comparable bandwidth:

- **Band-limited noise** (`DEFAULT_NOISE`): independent uniform random knots every
  500 ms, joined by a uniform cubic B-spline. It is C2-smooth, and its energy is
  mostly below about 0.6 Hz.
- **Sum of sinusoids** (`DEFAULT_SINUSOIDS`): 4 sinusoids, each weighted 1/4, with
  secret 64-bit frequencies drawn uniformly from 0.1 to 0.625 Hz and secret phases.

Both stay within the cap by construction: B-spline weights are a convex combination
of the knots, and sinusoid weights sum to 1. A final clamp absorbs rounding.

## Measurements

From `cargo run --release -p rearguard-core --example compare_shapes`: 1,000 pairs of
players, a 10-minute match, sampled every 50 ms, in units of the peak cap.

| Measure | Band-limited noise | Sum of 4 sinusoids |
|---|---|---|
| RMS (power available to a correlation detector) | **0.400** | 0.354 |
| Chance correlation between two players, \|r\| median | 0.022 | **0.005** |
| \|r\| 95th percentile | **0.070** | 0.076 |
| \|r\| 99th percentile | **0.093** | 0.211 |
| \|r\| maximum | **0.161** | 0.402 |
| Autocorrelation at a 5 s lag | **−0.002** | 0.031 |
| Fastest change (cap per second) | **2.95** | 3.57 |

## Recommendation: band-limited noise (now the default)

1. **More signal for the same cap.** Detection strength scales with signal power.
   The cap limits the peak, and noise carries 28% more power under the same peak
   (RMS 0.400 vs 0.354). Adding sinusoids does not help: with more of them, RMS falls
   towards cap/√(2n).
2. **A well-behaved false-positive floor.** For noise, chance correlation between
   unrelated signals is close to Gaussian, with σ ≈ 1/√(number of knots), and it
   shrinks predictably as a match gets longer. That lets the detector set a
   threshold with a known false-positive rate. The sinusoid sum has a better median
   but a heavy tail (99th percentile 2.3× higher, maximum 0.40). The tail comes from
   pairs of players whose secret frequencies happen to nearly coincide. A detector
   would have to budget for that tail, and it would be worse than it looks here for
   short windows or in-game signals that are themselves periodic.
3. **No long-range memory.** Each noise knot is fresh CSPRNG output, so observing
   part of the signal says nothing about the signal 2 s later. The sinusoid sum is
   fixed by 12 numbers: anyone who fits those from a few seconds of observation can
   extrapolate the rest of the epoch. That makes it cheaper to learn and cancel.
4. **Perceptibility is a wash.** Both are confined to slow drift below about 0.6 Hz.
   The noise's fastest change is lower: about 1.5%/s of sensitivity at the default
   0.5% cap, against 1.8%/s for the sinusoids. The sinusoid sum is quasi-periodic,
   which a player might conceivably learn to feel; noise has no pattern to learn.
   We have **no measured perception threshold**; a perceptual test is needed before
   the cap or bandwidth is raised.

Where the sinusoids would have been better: they need no keystream access per sample,
and a lock-in detector at known frequencies is simple. The server knows the exact noise
waveform too, so a matched filter gives the noise shape the same advantage.

## How determinism is guaranteed

- **Keys:** HKDF-SHA256 (RFC 5869) derives root → match → player → epoch → stream.
  Each step uses a distinct, length-prefixed label and context.
- **Randomness:** a ChaCha20 keystream. Knot *j* is word pair *j*, so any tick is
  computed in O(1) without replaying history.
- **Signal maths:** integer Q30 only. Sine is a 13th-order polynomial with error
  below 5e-9. The B-spline is evaluated in `i128`. No floating point until the
  optional `Drift::multiplier()`, which is two correctly rounded IEEE operations.
- **Golden vectors** for both shapes, both streams and several amplitudes, at ticks
  up to `u64::MAX`, are committed. CI checks them on Linux and Windows. They were also
  reproduced by an independent pure-Python implementation (hashlib HMAC plus a
  hand-written ChaCha20 block function).

## Open points for later tasks

- **Epoch boundaries:** a new epoch seed starts a fresh signal, so the drift can jump
  (by up to twice the cap) at the boundary. v0 uses one epoch per match, so this does
  not arise yet. Multi-epoch schedules need a cross-fade, or knots carried across the
  boundary.
- **Knot interval against flick duration:** a 500 ms knot interval makes the drift
  nearly constant within a 100–300 ms flick but different between flicks, which is
  what an open-loop detector needs. The detector task should tune this with the
  simulator.
- **The client can see the drift:** the client computes `d(t)` and applies it, so a
  cheat that reads memory, or compares raw input with view angle, can measure it and
  compensate. The probe raises the cost of cheating; it cannot stop a cheat built
  specifically to defeat it.
