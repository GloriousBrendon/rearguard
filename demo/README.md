# Rearguard aim range (Godot 4 demo)

A single-player aim range for the Phase 1 input-probe measurements. It captures
raw relative mouse deltas, applies them to the view in one place, and records
each input event with a monotonic timestamp and the resulting view angle, plus
fire events. That is the telemetry decision D2 calls for. No networking and no
drift logic yet.

Everything is GDScript, with no addons and no third-party assets. The world is
built from Godot primitives (a plane, a sphere, labels).

## Godot version

**Godot 4.7.2-stable** (`4.7.2.stable.official.ed1daf0bf`), standard build (not
.NET).

Confirmed as the latest stable on 2026-09-29. The GitHub releases API for
`godotengine/godot` and `godotengine/godot-builds` lists `4.7.2-stable`
(2026-08-18) as the newest non-prerelease 4.x; everything newer is `4.8-dev*`. A
web search agreed. The editor binary was downloaded from the official
`godot-builds` release and checked against its `SHA512-SUMS.txt`.

The tests pin values that depend on Godot's random number generator, so use
exactly this version. `project.godot` sets `config/features` to `4.7`.

## Running

All commands are run from the repository root; `godot` is the 4.7.2 editor binary.

The input probe, the telemetry file and the server link all come from the Rust
extension (`crates/rearguard-godot`), so build it first. Without it the range still
runs, but with no probe (identity hooks), no recording and no server, and Godot logs
one "Can't open dynamic library" error.

```sh
# Fresh checkout only: import BEFORE the first build (Godot 4.7.2 crashes on shutdown
# after a first import that loads a GDExtension; see crates/rearguard-godot/README.md).
godot --headless --path demo --import

# Build the GDExtension library (target/debug); demo/rearguard.gdextension loads it.
cargo build -p rearguard-godot

# Play (windowed). Click to capture the mouse and start; Esc releases and pauses.
godot --path demo -- --scenario flick --seed 7

# Scripted bot, headless, faster than real time, then quit.
godot --headless --path demo --fixed-fps 120 -- --bot --scenario spray --seed 7 --duration 20 --out /tmp/spray.jsonl

# Live loop: a Rearguard server issues the seed and returns a verdict (see "Live loop").
cargo run -p rearguard-server -- run --config server.json    # in another terminal
godot --path demo -- --scenario flick --server 127.0.0.1:7461

# Tests (exit code 0 on success, 1 on any failure). The extension tests are skipped
# if the library is not built, unless REARGUARD_REQUIRE_EXTENSION=1 (as in CI).
REARGUARD_REQUIRE_EXTENSION=1 godot --headless --path demo --fixed-fps 120 --script res://tests/run_tests.gd

# Human study (task 1.9): consent, baseline rounds, blind A/B comparisons, one export
# file. See "Human study" and docs/study/facilitator-instructions.md.
godot --path demo res://scenes/study.tscn

# Input statistics for one or more recordings.
godot --headless --path demo --script res://tools/summarize_recording.gd -- /tmp/spray.jsonl
```

On a fresh checkout, run `godot --headless --path demo --import` once first, before
building the extension, so Godot builds its `.godot/` cache (git-ignored). CI wraps each
call in `timeout` (see "CI").

## CI

The `godot tests (linux)` and `godot tests (windows)` jobs in
`.github/workflows/ci.yml` run this test suite on every branch push, on
`ubuntu-latest` and `windows-latest`. They are independent of the Rust jobs and show
as their own checks. The Windows job runs the same steps in Git Bash with the
official `Godot_v<version>_win64.exe.zip` (its console wrapper, so output reaches the
log), and then builds the release extension DLL for the study export (see "Study
builds").

1. It restores `~/godot-dl/godot.zip` from the Actions cache. On a miss, it
   downloads `Godot_v<version>_linux.x86_64.zip` from the official
   `godotengine/godot-builds` release. The cache key includes the version and the
   checksum.
2. It checks the zip against the pinned SHA-512 with `sha512sum --check` on
   every run, cached or not. A mismatch fails the job.
3. It unpacks the zip, prints `godot --version`, runs the one-time `--import`
   (`timeout 300`), then the test command above (`timeout 600`). The job fails
   if any test fails, if a test file does not compile, or if no tests run.

The job also installs the pinned Rust toolchain. Between the import and the tests it
builds the extension and the server (`cargo build --locked -p rearguard-godot -p
rearguard-server`); the live-loop tests start that server binary. It builds after the
import because of the first-import crash described in `crates/rearguard-godot/README.md`,
and removes any cached library before importing for the same reason. It runs the tests
with `REARGUARD_REQUIRE_EXTENSION=1`, so the extension tests fail if the library does
not load.

To bump Godot, change these values together in one pull request:

1. `GODOT_VERSION` in every job that downloads Godot (`godot`, `godot-windows`,
   `study-build`, `study-smoke-windows`), for example `4.7.3-stable`.
2. The checksums, from that release's `SHA512-SUMS.txt`
   (`https://github.com/godotengine/godot-builds/releases/download/<version>/SHA512-SUMS.txt`),
   standard builds, not `mono`:
   - `GODOT_ZIP_SHA512`: `Godot_v<version>_linux.x86_64.zip` (`godot`, `study-build`);
   - `GODOT_WIN_ZIP_SHA512`: `Godot_v<version>_win64.exe.zip` (`godot-windows`,
     `study-smoke-windows`);
   - `GODOT_TEMPLATES_SHA512`: `Godot_v<version>_export_templates.tpz` (`study-build`).
3. This README's "Godot version" section, and `config/features` in
   `project.godot` if the minor version changes.
4. Any pinned test values that depend on Godot's random number generator (for
   example `PINNED` in `scripts/scenario_test.gd`), if the new version changes
   them. Change these only after confirming the RNG itself changed.

The new version and checksum give a new cache key, so the first run after a bump
downloads the new zip, and later runs use the cache.

Options (after `--`):

| Option | Default | Meaning |
|--------|---------|---------|
| `--scenario NAME` | `flick` | `tracking`, `flick` or `spray` |
| `--seed N` | `1` | Scenario seed; fixes every target position |
| `--duration S` | `60` | Scenario length in seconds of scenario time |
| `--sens DEG` | `0.022` | Degrees of view per raw count |
| `--bot` | off | Scripted bot plays instead of the mouse; implies `--quit` |
| `--bot-seed N` | `1` | Seed for the bot's noise |
| `--cheat NAME` | none | A test cheat plays instead of the mouse (`recoil-macro`, `flick-aimbot`, `humanised-aimbot`, `adaptive-aimbot`); implies `--quit`. **Test environment only:** needs `REARGUARD_TEST_ENV=1` and a loopback or allowlisted `--server`, else exits with status 3. See "Test cheats" |
| `--out PATH` | `user://recordings/<scenario>-seed<N>-<time>.jsonl` | Recording file |
| `--quit` / `--no-quit` | off | Quit when the scenario ends |
| `--no-probe` | probe on | Run without the input-probe drift (identity hooks) |
| `--server HOST:PORT` | none | Stream the session to a Rearguard server (loopback) and take the drift's seed from it; see "Live loop" |
| `--probe-seed-file PATH` | none | Offline only: probe seed file (64 hex digits), created with a fresh OS-random seed if missing. Without it (and without a server), a random seed is kept in memory only, so the run can never be analysed |
| `--probe-amplitude-ppm N` | `5000` | Offline only: drift amplitude, 0 to 20000 (0.5% default, 2% maximum). With a server, the server sets it |
| `--no-record` | recording on | Do not write the telemetry file |
| `--frame-stats PATH` | none | Write frame-time statistics (p50, p99, mean) for the run, and uplink counters in debug builds, as JSON |

Offline, the seed file stands in for the server's secret. Keep it apart from the
recordings; it is never copied into them.

`user://` is `~/.local/share/godot/app_userdata/Rearguard Aim Range/` on Linux and
`%APPDATA%\Godot\app_userdata\Rearguard Aim Range\` on Windows. The path is
printed when a run ends and shown on screen.

`--fixed-fps 120` makes every frame exactly one physics tick, so a headless bot
run is deterministic and not tied to wall-clock time. With `--display-driver`
you can choose `x11` or `wayland` on Linux (see below).

## Scenarios

The target is a 0.3 m sphere 10 m away, about 1.72° in angular radius. Positions
are view angles and depend only on the scenario and its seed (tracking also uses
the tick number). Scenario time advances one physics tick (1/120 s) at a time,
and only while the mouse is captured or the bot is playing.

| Scenario | What happens | Seed decides |
|----------|--------------|--------------|
| `tracking` | One target strafes left and right in straight segments of 0.3 to 1.2 s at 15 to 50°/s, within ±30° yaw | Segment lengths, speeds, directions, pitch |
| `flick` | One static target at a time within ±40° yaw and -10 to 20° pitch. A hit or 2.5 s timeout brings the next | The target sequence |
| `spray` | A static target near the centre. Holding fire sprays up to 30 shots with the fixed recoil pattern `v1`. Releasing ends the round and brings the next target | Each round's target position |

The weapon fires at 600 rounds per minute while the button is held. The first
shot fires on the button press itself. Recoil exists only in `spray`, and the
pattern is a constant in `scripts/scenario.gd` that never depends on the seed.
There is no automatic recoil recovery, so the player pulls down to compensate.

## Hook points for the drift

`scripts/aim_model.gd` is the only code that changes the view angle:

- `apply_sensitivity(raw_counts)` is the one place sensitivity is applied. It
  converts one raw delta to a view change (`-counts × deg_per_count`) and passes
  it through `look_hook`.
- `apply_recoil(kick_deg)` is the one place recoil is applied. It passes each
  shot's kick through `recoil_hook`.

Both hooks are `Callable`s taking and returning `PackedFloat64Array([d_yaw,
d_pitch])` in degrees. Both default to `identity`. Angles are kept as 64-bit
floats throughout, because Godot's `Vector2` is 32-bit.

With the probe on, `scripts/probe_hooks.gd` installs the drift on both hooks. Each
hook multiplies the angle change by the extension's multiplier:
`RearguardProbe.sensitivity_multiplier(ts)` for looks, and `recoil_scale(ts)` for
recoil. `ts` is the event's timestamp, which `RangeSession` puts in `AimModel.event_us`
before each `apply_*` call. The hooks read it from the model's small `EventClock` object,
never from the model itself: a hook that captured the model it is stored on would be a
reference cycle, and the model would leak at exit. The probe's tick 0 is the session's start (`probe_start_us`
in the header), and an event's probe tick is `(ts_us - probe_start_us) / 1000`. All of
the drift maths is in `rearguard-core`.

Angle convention: yaw is Godot `rotation.y`, where positive turns left, and it is
never wrapped. Pitch is `rotation.x`, where positive looks up, clamped to ±89°. At
yaw 0 and pitch 0 the camera looks along -Z. Mouse right (+dx) turns right, and
mouse down (+dy) looks down.

## Recording format

A recording is client telemetry in the **`rearguard.telemetry` version 1** schema
(`rearguard_core::telemetry`). This is exactly what a real client streams to the server,
so a recorded session goes straight into the detector. It is JSON Lines: one object per
line, UTF-8, LF line endings. The Rust extension writes it (`RearguardRecorder`, via
`scripts/telemetry_out.gd`), so its floats are exact. It replaces the task-1.4
`rearguard.aimrange.recording` format.

Every record has `type`, `ts_us`, `frame` and `tick`:

- `ts_us` is `Time.get_ticks_usec()`: microseconds since the engine started, from a
  monotonic clock (`CLOCK_MONOTONIC_RAW` on Linux, `QueryPerformanceCounter` on
  Windows). It is when Godot *handled* the event, not when the mouse reported it.
  See "Batching and timestamps".
- `frame` is `Engine.get_process_frames()`. Events with the same `frame` were
  delivered in the same batch.
- `tick` is the scenario tick (1/120 s) at the time of the record.

| `type` | Written when | Extra fields |
|--------|--------------|--------------|
| `header` | First line | `format` (`"rearguard.telemetry"`), `version` (1), `match_id` (from the server, or `"aimrange-local"`), `player_id`, `source` (`"client"`), `probe_start_us` (the timestamp of probe tick 0: the session start), `deg_per_count`, `physics_hz`, `scenario`, `scenario_seed`, `duration_s`, `recoil_pattern`, `target_distance_m`, `target_radius_m` |
| `move` | Every raw mouse motion event | `dx`, `dy`: raw counts before sensitivity. `yaw`, `pitch`: resulting view in degrees |
| `button` | Fire button press or release | `pressed` |
| `fire` | Every shot | `shot` (run-wide index), `burst_shot`, `yaw`, `pitch` (view when fired, before recoil), `target`, `target_yaw`, `target_pitch`, `hit` |
| `recoil` | After each shot with a kick | `shot`, `burst_shot`, `kick_yaw`, `kick_pitch` (nominal pattern kick), `yaw`, `pitch` (view after recoil) |
| `target` | Flick or spray target appears | `target`, `target_yaw`, `target_pitch` |
| `end` | Last line | `shots`, `hits`, `complete` (false if the window was closed early) |

The runtime details the schema does not carry are written next to the recording as
`<recording>.env.json`:
- `source` (`human` or `bot`), `bot_seed`, `cheat` and `ground_truth` (a test cheat's name
  and label, see "Test cheats"), `godot_version`, `os`, `display_driver`,
  `accumulated_input`, `delta_source`, `clock`, `debug_build`;
- the probe: `probe_enabled`, `probe_amplitude_ppm`, `probe_start_us`, and
  `probe_seed`: where the seed came from (`"server"`, `"file"`, `"random"` or
  `"none"`), never the seed;
- `server` and `server_status`.

Replaying a recording gives back every view angle exactly:
- start at yaw 0 and pitch 0;
- for each `move`, add `-dx × deg_per_count × m` and `-dy × deg_per_count × m`;
- for each `recoil`, add the kick × `r`.

Here `m` and `r` are the probe's sensitivity multiplier and recoil scale at that
record's `ts_us`, and are 1 with the probe off. Computing them needs the seed, so only
its holder (the server, or the holder of the seed file) can replay a probed recording.
The server does exactly this, and counts every mismatch. The tests check the replay, and
`crates/rearguard-core/tests/aimrange_fixture.rs` feeds a recorded session to the
detector with zero mismatches.

Precision: the extension writes floats at full round-trip precision, and a correctly
rounded parser reads them back bit for bit. (In Rust, `serde_json` needs its
`float_roundtrip` feature for this, because its default float parsing can be off in the
last bit.) **Godot's own `JSON.parse_string` and `String.to_float` are not correctly
rounded**, and about 14% of doubles come back one ulp off. So the GDScript tests compare
in-memory records for exactness, and use parsed text only for structure or with a 1e-9
tolerance.

The file is flushed once per second of scenario time. If the process is killed,
everything but a possibly truncated final line is valid, so discard a trailing
line that does not parse.

## Live loop

With `--server HOST:PORT`, the aim range streams its telemetry to a Rearguard server
(`crates/rearguard-server`, loopback only until task 3.3) as it plays:

1. **At the start of the run,** it connects and opens a session. The server's
   `Welcome` carries the session's epoch seed and amplitude. The seed goes straight
   from the extension's `RearguardClient` to its `RearguardProbe`, inside Rust, and
   never reaches GDScript, the screen, the recording or a log. The header takes the
   server's `match_id` and `player_id`.
2. **While playing,** every record goes both to the file and to the uplink
   (`rearguard_core::uplink`). The uplink only queues it; a worker thread batches
   records into numbered chunks and sends them, so the game never waits on the
   network.
3. **At the end,** the aim range ends the server session and waits for the verdict,
   up to 10 s, before completing. It then prints one line: the session, its status,
   and in debug builds the verdict's score and whether it was flagged. The server
   stores the evidence (`rearguard-server verdict --db ... --session ID`).

**When the server is unreachable or goes away** (also tested in
`scripts/live_loop_test.gd` and `crates/rearguard-server/tests/uplink.rs`):

- **Unreachable at the start:** the run goes offline. The drift is applied from a
  local random seed, so play is unchanged. The recording is complete and there is no
  verdict: `aim_range: no server session (unreachable)`.
- **The connection drops mid-session:** the game carries on and is never blocked.
  The uplink keeps every unacknowledged chunk (up to 16 MiB) and tries to resume
  every 0.5 s. If the server resumes the session, the uplink resends what the server
  lacks, and nothing is lost.
- **The session is lost:** this happens if the server no longer knows the session
  (it was closed as abandoned, or the server restarted), if 30 s pass without a
  connection, or if the buffer fills. Records are then dropped from the uplink only;
  the local recording stays complete. The run ends with
  `aim_range: server session N: lost: ... , no final verdict` (or `reconnecting` if it
  ends first).

**Developer overlay:**
- It appears only in debug builds (`OS.is_debug_build()`: the editor, `godot` runs
  and debug exports). Its data accessors (`RearguardClient.verdict()`, `stats()` and
  `request_verdict()`) exist only in debug builds of the extension, so release builds
  have neither.
- It shows the server status, session and match ids, the probe's amplitude and seed
  *source*, uplink traffic and bytes per minute, and the live verdict (requested
  every 5 s) with both statistics.
- It also shows recent frame times.
- It never shows seed material.

**Cost:** frame-time cost (p50 and p99, probe and recorder on versus off) and
bandwidth per minute of play are in `docs/results/live-loop-1.7.md`, measured with
`scripts/measure-live-loop.sh`.

## Test cheats

`scripts/test_cheats/` holds the test cheats of task 1.8: a recoil macro, a
computed-flick aimbot, a humanised aimbot and an adaptive aimbot. They mirror the
simulator's cheat models. Details are in
[scripts/test_cheats/README.md](scripts/test_cheats/README.md).

**They run only inside Rearguard's own test environment.** They inject input only into
this range, in process, through its own input path. Each one refuses to start unless
`REARGUARD_TEST_ENV=1` is set and `--server` is a loopback address or on
`REARGUARD_TEST_SERVER_ALLOWLIST`. Nothing in them hooks, reads or targets another
process or game, and study builds leave them out.

Each run's ground-truth label goes into the server session's metadata, for the
evaluation harness only (`rearguard-server labels`). It is never in the telemetry, and
the detector never reads it.

Equivalence with physical KMBox or Cronus hardware injectors is unverified; it would
need a physical device to confirm.

## Human study

`scenes/study.tscn` (`scripts/study.gd`) is the build for human studies (task 1.9).
The facilitator's steps are in `docs/study/facilitator-instructions.md`.

**Flow:**

1. **Consent** (`study/consent_v1.txt`). Nothing is recorded before the participant
   agrees; declining quits with nothing written.
2. **Participant ID and plan.** A random pseudonymous ID (`P-` plus 10 characters) is
   drawn from the OS's CSPRNG, along with a random 62-bit randomisation seed. The ID
   is not linked to anyone and no mapping is stored.
   `rearguard_core::study::plan` (via `RearguardStudy.plan`) turns the protocol
   (`study/protocol.json`) and the seed into the plan. The plan is deterministic, so
   the analysis can reproduce it from the seed in the export.
3. **Baseline rounds:** one aim-range run per planned session, drift on or off in
   shuffled order.
4. **Blind comparisons:** two intervals, A then B, with the same scenario seed. One is
   drifted at the trial's amplitude and the other is not (amplitude 0 is a catch trial,
   where neither is). The drifted side is balanced within each amplitude and the trial
   order is shuffled. Each trial ends in a forced choice, A or B; the answer and
   response time are logged.
5. **Export:** one zip,
   `rearguard-study-<study>-<participant>.zip`, in `user://exports` (or
   `--export-dir`). It holds `manifest.json` (protocol, plan, seed, sessions, trials,
   Godot and input settings) and `sessions/<label>.jsonl`, each round's telemetry.
   The temporary session files are then deleted.

**Drift seeds.** Each round's probe seed is derived inside the extension from the
facilitator's study key (`--study-key`; default: the key packed into a study build, else `user://study/study-key.hex`). The
derivation is root → match → player 0 → epoch 0, with the match id
`study:<study>:<participant>:<label>`, which is also the recording header's
`match_id`. The export never holds the key or a seed; the analysis re-derives them
from the key, as `scripts/study_test.gd` does.

**Blindness.** During the study, the aim range runs without the developer overlay and
without the environment sidecar file. Its HUD shows only the round's neutral title,
the time left and hits. Nothing on screen shows a round's condition or amplitude.

**Build checks** (options after `--`, for exported builds; see "Study builds"):
- `--smoke-decline` shows the consent screen, prints the SHA-256 of the text on screen
  and what the build packs, then presses "I do not agree" and "Quit". It can only
  decline.
- `--self-test` runs the study with no participant: the scripted bot plays, answers
  come from a seeded RNG, and the export is marked `"automated": true`, so it can never
  pass for a person's data. The build quits once the export is written.

**Tests** (`scripts/study_test.gd`) run the whole flow with automated consent, the
scripted bot and seeded answers. They check:
- the labelled baseline sessions, drift on and off, each replayed with the probe
  re-derived from the key;
- the logged trials;
- that the plan re-derives from the logged seed;
- that the export holds only allowlisted keys and none of the machine's user name,
  host name, home or user-data path, unique id, today's date or the study key;
- that declining writes nothing;
- that the extension reads a `res://` key (as packed into study builds) exactly like a
  file-system one (`tests/public-test-key.hex`, public test data).

The export checks live in `tests/study_export_check.gd`, shared with the check of
exported builds, so both apply exactly the same rules.

## Study builds

Volunteers run exported builds, not the repository (task 1.9a). The facilitator's side
is in `docs/study/facilitator-instructions.md`.

`scripts/export-study.sh` exports Linux and Windows x86_64 builds with the official
Godot 4.7.2 **release** templates (never debug ones). It works on a staging copy of this
folder in a temporary directory, deleted afterwards, which:
- packs the release's study key as `res://study/study-key.hex` and its label as
  `res://study/release.txt`. Neither is ever in the repository (`.gitignore` blocks
  them). The study uses the packed key by default, and the extension reads it from the
  pack itself, so it never passes through GDScript;
- leaves out the developer overlay (`scripts/dev_overlay.gd`), the tests (`*_test.gd`,
  `tests/`), `tools/` and the test cheats (`scripts/test_cheats/`);
- makes `scenes/study.tscn` the main scene;
- turns off Godot's log file and shader cache, so a build that is declined writes
  nothing at all.

Export presets: `scripts/study-export/export_presets.cfg` (pack embedded in the program,
no console wrapper, Windows resources unmodified, so no `rcedit` is needed). Each zip
holds the program, the extension library, a participant `README.txt`
(`scripts/study-export/README.txt`), `THIRD-PARTY-NOTICES.txt`, and Rearguard's licence
texts `LICENSE-MIT` and `LICENSE-APACHE`. The notices come
from the Godot binary (`tools/godot_notices.gd`) and from the licence files of every
crate compiled into the extension (`scripts/study-export/rust_notices.py`).

The manifest's `release` field is the label (`dev` from the repository), and
`input.debug_build` is `false` in a study build.

Official export templates cannot run `--script`, `--scene` or `--main-pack` (they are
built without `OVERRIDE_PATH_ENABLED`), so a build can only run the study. The build
checks above are therefore study options, not scripts.

`scripts/smoke-study-build.sh` tests an exported build headless, on Linux or Windows
(Git Bash), with fresh, empty user folders:
1. `--smoke-decline`: the build starts and exits 0. The consent text on screen matches
   `study/consent_v1.txt` byte for byte, the key is packed, and the overlay, tests,
   tools and test cheats are not. No file is written in the user folders, the working
   directory or the build folder. The build holds `LICENSE-MIT` and `LICENSE-APACHE`,
   identical to the repository's.
2. `--self-test` on a short protocol, then `tools/check_study_export.gd` (run with the
   editor) checks the export with the task 1.9 rules. It also replays every session
   with the drift re-derived from the facilitator's copy of the key, which proves that
   copy is the key in the build.

In CI, `study build (export, linux smoke)` exports both builds from a fresh key per
run, smoke-tests the Linux build, and uploads the builds, their sizes and the key as
artifacts. `study build (windows smoke)` runs the smoke test on the Windows build. The
Windows DLL comes from `godot tests (windows)`.

Sizes (CI run 36765443315, release `r44-d93e066`, 2026-09-30; every run prints its sizes
in the job summary and the `study-build-sizes` artifact):

| File | Linux (bytes) | Windows (bytes) |
|------|---------------|-----------------|
| Program: release template with the pack embedded | 73,569,888 | 109,318,960 |
| Extension library (release) | 2,485,432 | 1,865,216 |
| `THIRD-PARTY-NOTICES.txt` | 545,867 | 545,867 |
| `LICENSE-APACHE` | 11,358 | 11,358 |
| `LICENSE-MIT` | 1,071 | 1,071 |
| `README.txt` | 1,592 | 1,592 |
| **Zip sent to a volunteer** | **29,209,943** (27.9 MiB) | **38,591,915** (36.8 MiB) |

Unzipped, a build takes about 77 MB (Linux) or 112 MB (Windows).

## Raw input path

This is how a mouse movement reaches `move.dx` / `move.dy`. It was checked
against the 4.7.2-stable source; file references are to that tag.

1. **Capture.** The scene sets `Input.mouse_mode = MOUSE_MODE_CAPTURED` on the
   first click. Only in captured mode do the platform backends switch to their
   raw paths, so motion while uncaptured (the desktop cursor) is ignored and never
   recorded.
2. **No accumulation.** `_ready()` sets `Input.use_accumulated_input = false`.
   With that and `input_devices/buffering/agile_event_flushing` off (the
   default), `Input::parse_input_event` hands every event straight to
   `_parse_input_event_impl` instead of merging motion events per frame
   (`core/input/input.cpp`, `parse_input_event`). The header records
   `accumulated_input` so a run with it on would be visible.
3. **Unscaled delta.** The scene reads `InputEventMouseMotion.screen_relative`,
   which the stretch/content-scale transform never touches. `relative` would be
   scaled by the viewport's stretch settings.
4. **Handling.** `_input()` hands the delta to `RangeSession.handle_motion`, which
   calls `AimModel.apply_sensitivity` and writes the `move` record with
   `Time.get_ticks_usec()`.

What each platform backend puts into that event while captured:

| Platform | Source | Unaccelerated? | One event per OS report? | Precision |
|----------|--------|----------------|--------------------------|-----------|
| Windows | `WM_INPUT` raw input, `RAWMOUSE.lLastX/lLastY` (`platform/windows/display_server_windows.cpp`, `_get_raw_mouse_motion`) | Yes (device counts) | Yes, one event per raw packet | Integer counts |
| Linux X11 (and XWayland) | XInput2 `XI_RawMotion` `raw_values` (`platform/linuxbsd/x11/display_server_x11.cpp`) | Yes | Not guaranteed (see below) | Truncated to integer (`Point2i rel`) |
| Linux native Wayland | `zwp_relative_pointer_v1.relative_motion` `dx`/`dy` (`platform/linuxbsd/wayland/wayland_thread.cpp`, `_wp_relative_pointer_on_relative_motion`) | **No.** Godot uses the accelerated `dx`, not `dx_unaccel` | Not guaranteed (see below) | Fractional, multiplied by the window scale factor |

Details worth knowing:

- **X11:** each `XI_RawMotion` *overwrites* the pending relative motion (for
  relative devices), and the event is only emitted on the next `MotionNotify`. If
  two raw events arrive before one `MotionNotify`, the first delta is lost. The
  delta is also stored in a `Point2i`, so a fractional raw value would be
  truncated; ordinary mice report whole counts. X11 drops a raw event with the same
  timestamp and value as the pending one, to work around a duplicate-event bug.
- **Wayland:** the backend overwrites the pending relative motion in the same way
  until the pointer frame is dispatched. More importantly, it takes the
  compositor's **accelerated** delta and scales it by the window's scale factor, so
  on native Wayland `dx`/`dy` are not raw counts unless the compositor's
  acceleration profile is flat and the scale is 1.
- **Driver choice on Linux:** with `display/display_server/driver.linuxbsd =
  "default"`, 4.7.2 picked **X11 (XWayland)** on this machine's Wayland session.
  Native Wayland only runs with `--display-driver wayland`. The header's
  `display_driver` records which one was used. For Phase 1 measurements use X11 or
  XWayland, and treat native-Wayland recordings as accelerated.

### Batching and timestamps

`InputEvent` has no hardware timestamp in Godot 4. Each display server drains
the OS queue once per main-loop iteration (`DisplayServer::process_events`), and
every event is dispatched during that drain. So events that arrived between two
frames get timestamps a few microseconds apart, stamped when the drain ran.
`ts_us` gives the order and the per-frame batching of the events, not the times
at which the device reported them. The `frame` field shows the batches.

### What was observed

- **Linux, X11 via XWayland, real mouse, 0.8 s of slow movement (4.7.2, Mesa,
  Radeon RX 7800 XT).** 227 `move` events arrived. All deltas were whole counts
  (±1 to ±3); there were no fractional values. Up to 7 separate events arrived in
  a single frame, so accumulation is really off. The median gap between
  consecutive events was 18 µs, while frames were about 6 ms apart; that is the
  batching described above. Many events carried only one axis (x-only or y-only).
- **Linux, native Wayland:** the scene runs (bot mode, windowed), but it has not
  been tested with a real mouse, so the acceleration difference above comes from
  the source, not from a measurement.
- **Windows:** not run yet; the table row comes from the source.

### Checking a platform

To compare X11 and Wayland (or any two machines), use the same mouse and
settings. With desktop pointer acceleration **on**, record one run per driver:

```sh
godot --display-driver x11     --path demo -- --scenario flick --duration 20 --out /tmp/x11.jsonl
godot --display-driver wayland --path demo -- --scenario flick --duration 20 --out /tmp/wl.jsonl
godot --headless --path demo --script res://tools/summarize_recording.gd -- /tmp/x11.jsonl /tmp/wl.jsonl
```

In each run, move the mouse the same physical distance twice, slowly once and
quickly once (for example, between two marks on the mouse pad). With raw input,
the summed `|dx|` is the same for both speeds and every delta is a whole number.
With accelerated input, the fast pass sums higher and fractional deltas appear.

## Layout

| Path | What |
|------|------|
| `project.godot` | Project settings: 120 Hz physics, GL Compatibility renderer |
| `scenes/aim_range.tscn` | Main scene; everything is built in `aim_range.gd` |
| `scripts/aim_range.gd` | Scene: options, input capture, bot injection, drawing |
| `scripts/range_session.gd` | One run: weapon, targets, scoring, recording. No scene dependency |
| `scripts/aim_model.gd` | View angle, `apply_sensitivity`, `apply_recoil`, hooks |
| `scripts/scenario.gd` | Seeded scenario generation and the fixed recoil pattern |
| `scripts/recorder.gd` | In-memory log of the session's telemetry records (tests), and a telemetry file reader |
| `scripts/telemetry_out.gd` | Forwards records to the extension's file writer or server client |
| `scripts/dev_overlay.gd` | Developer overlay (debug builds only; never in study builds) |
| `scenes/study.tscn`, `scripts/study.gd` | Human-study build: consent, baseline, blind A/B comparisons, export |
| `study/` | Study protocol (`protocol.json`) and consent text (`consent_v1.txt`) |
| `scripts/scripted_bot.gd` | Scripted synthetic player for headless runs |
| `scripts/test_cheats/` | Test cheats (task 1.8), test environment only; see its README |
| `scripts/probe_hooks.gd` | Installs the extension's drift multipliers on the two aim hooks |
| `rearguard.gdextension` | Loads the Rust extension (`crates/rearguard-godot`) from `../target/` |
| `scripts/*_test.gd` | Tests, one file beside each script; `probe_binding_test.gd` checks the extension, `live_loop_test.gd` runs against a real server (`target/debug/rearguard-server`) |
| `tests/` | Test runner and base class; `live_server.gd` starts a real server for the live tests; `probe_golden.json` holds the core golden vectors the extension must reproduce; `study_export_check.gd` holds the study export checks |
| `tools/summarize_recording.gd` | Input statistics for recordings |
| `tools/check_study_export.gd` | Checks an export from a study build (smoke test, or a participant's zip with `--human`) |
| `tools/godot_notices.gd` | Writes the engine's licence notices for study builds |

The bot is an ordinary aiming controller with a reaction delay and seeded noise.
It reads only this range's own state, and its events go through
`Input.parse_input_event` and `_input` like a mouse's. They are tagged with a
reserved device id, so hardware input and bot input never mix.
