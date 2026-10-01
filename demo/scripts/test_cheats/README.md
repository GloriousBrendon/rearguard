# Test cheats (task 1.8)

In-tree bots that inject input into **this aim range only**, to exercise Rearguard's own
detector end to end: aim range → live loop → `rearguard-server` → verdict. They are the
Godot counterparts of the simulator's cheat models (`crates/rearguard-sim/src/aimbot.rs`
and the macro in `crates/rearguard-sim/src/human.rs`), which remain the behavioural
reference.

## Test environment only

**These bots run only inside Rearguard's own test environment.** This is a hard rule of
the project, not a default:

- **In process, through the range's own input path.** A bot produces raw counts and a
  trigger state. The scene injects them with `Input.parse_input_event`, tagged with the
  range's reserved bot device id, and they arrive at `_input` like a mouse's. Nothing
  hooks, injects into, reads or targets any other process or game. The bots read only
  this range's own state (`RangeSession`: the view and the current target).
- **Guarded.** `test_env.gd` refuses to create a bot unless:
  - `REARGUARD_TEST_ENV=1` is set; and
  - `--server` is an IP literal on this machine (`127.0.0.0/8` or `::1`), or appears in
    `REARGUARD_TEST_SERVER_ALLOWLIST` (comma-separated `HOST` or `HOST:PORT`).

  Host names, even `localhost`, are not trusted, because they could resolve anywhere.
  Without a server the bots do not run at all. On refusal the aim range logs why and
  exits with status 3, before any session, recording or input.
- **Not shipped.** Study builds for volunteers (`scripts/export-study.sh`) leave this
  folder out, and their smoke test checks that it is absent.

Do not adapt these bots to target anything else. That is out of scope for Rearguard and
against its rules (`CLAUDE.md`).

## The bots

| `--cheat` | Scenario | Behaviour (after the simulator's model) |
|-----------|----------|------------------------------------------|
| `recoil-macro` | spray | The scripted player (`scripts/scripted_bot.gd`) acquires each target and holds the trigger. During the burst the hand rests (tremor only), and after every shot the macro injects the exact nominal counts that cancel that shot's kick in pattern `v1`, carrying the rounding remainder |
| `flick-aimbot` | flick | 30 ms after a target appears, and once the weapon can fire, one move of the exact counts to the target (from the range's own angles), then a tap |
| `humanised-aimbot` | flick | Log-normal reaction of about 200 ms. Then a minimum-jerk path with a Fitts'-law duration, planned once to a random point on the target (σ 0.1° to 0.2°), with jitter, then a click. Per-run parameters come from the simulator's ranges |
| `adaptive-aimbot` | flick | A computed flick that knows a small drift exists, but not the seed. After each flick it reads the view change its own move caused, keeps a running estimate of the multiplier (weight 0.5 for the newest), and divides the next flick's counts by it. Log-normal reactions of about 150 ms |

None of them knows the probe seed. Each is stepped once per physics tick (1/120 s),
whereas the simulator steps once per millisecond, so timings are rounded to ticks. The
flicking bots re-aim only after a real miss. `--bot-seed` seeds their noise.

## Ground-truth labels

Each run's label (`cheat:<name>`; the scripted bot's is `bot:scripted`) goes to the
server in `Hello` and is stored with the session (`sessions.label`) for the evaluation
harness. `rearguard-server labels --db FILE` lists labelled sessions with their verdicts.
It is also in the local `<recording>.env.json` (`cheat`, `ground_truth`). The label is
**never** in the telemetry, and the detector never reads it:
- `crates/rearguard-server/tests/labels.rs` shows that a labelled session's verdict is
  exactly what a label-free offline detector computes from the same telemetry, and that
  lying labels change nothing;
- `scripts/test_cheats_test.gd` checks that no recording contains the label.

## Running

```sh
cargo build -p rearguard-godot -p rearguard-server
cargo run -p rearguard-server -- run --config server.json      # another terminal
REARGUARD_TEST_ENV=1 godot --headless --path demo --fixed-fps 120 -- \
    --cheat flick-aimbot --scenario flick --server 127.0.0.1:7461 --duration 30
```

Tests: `scripts/test_cheats_test.gd`, in the normal suite (`demo/README.md`).

To record every bot for the detector evaluation (task 1.12), use
`scripts/record-test-bots.sh`, then `cargo xtask eval --bots ...`
(`crates/xtask/README.md`). The evaluation reads each run's label from its
`.env.json`; the detector does not.

## Hardware injectors: equivalence unverified

These bots inject input in software, inside the game process. Hardware injectors, such as
**KMBox** or **Cronus** devices, sit between a physical mouse or controller and the PC
and present their input as a real USB device. On a single session, the game sees the same
kind of relative deltas either way, but **equivalence with physical KMBox or Cronus
hardware is unverified**. The two can differ in report timing and batching, polling
rate, how deltas are split and rounded, and how they mix with the user's own hand
movement. Confirming equivalence would need a physical device, recorded through the
range's real mouse path. Real hardware injectors are out of scope for task 1.8.
