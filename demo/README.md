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

```sh
# Play (windowed). Click to capture the mouse and start; Esc releases and pauses.
godot --path demo -- --scenario flick --seed 7

# Scripted bot, headless, faster than real time, then quit.
godot --headless --path demo --fixed-fps 120 -- --bot --scenario spray --seed 7 --duration 20 --out /tmp/spray.jsonl

# Tests (exit code 0 on success, 1 on any failure).
godot --headless --path demo --fixed-fps 120 --script res://tests/run_tests.gd

# Input statistics for one or more recordings.
godot --headless --path demo --script res://tools/summarize_recording.gd -- /tmp/spray.jsonl
```

On a fresh checkout, run `godot --headless --path demo --import` once first so
Godot builds its `.godot/` cache (git-ignored). CI should wrap each call in
`timeout`.

Options (after `--`):

| Option | Default | Meaning |
|--------|---------|---------|
| `--scenario NAME` | `flick` | `tracking`, `flick` or `spray` |
| `--seed N` | `1` | Scenario seed; fixes every target position |
| `--duration S` | `60` | Scenario length in seconds of scenario time |
| `--sens DEG` | `0.022` | Degrees of view per raw count |
| `--bot` | off | Scripted bot plays instead of the mouse; implies `--quit` |
| `--bot-seed N` | `1` | Seed for the bot's noise |
| `--out PATH` | `user://recordings/<scenario>-seed<N>-<time>.jsonl` | Recording file |
| `--quit` / `--no-quit` | off | Quit when the scenario ends |

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

Angle convention: yaw is Godot `rotation.y`, where positive turns left, and it is
never wrapped. Pitch is `rotation.x`, where positive looks up, clamped to ±89°. At
yaw 0 and pitch 0 the camera looks along -Z. Mouse right (+dx) turns right, and
mouse down (+dy) looks down.

## Recording format

A recording is JSON Lines: one JSON object per line, in UTF-8, with LF line
endings. Every record has `type`, `ts_us`, `frame` and `tick`.

- `ts_us` is `Time.get_ticks_usec()`: microseconds since the engine started, from a
  monotonic clock (`CLOCK_MONOTONIC_RAW` on Linux, `QueryPerformanceCounter` on
  Windows). It is when Godot *handled* the event, not when the mouse reported it.
  See "Batching and timestamps".
- `frame` is `Engine.get_process_frames()`. Events with the same `frame` were
  delivered in the same batch.
- `tick` is the scenario tick (1/120 s) at the time of the record.

| `type` | Written when | Extra fields |
|--------|--------------|--------------|
| `header` | First line | `format` (`"rearguard.aimrange.recording"`), `version` (1), `scenario`, `scenario_seed`, `duration_s`, `physics_hz`, `deg_per_count`, `recoil_pattern`, `target_distance_m`, `target_radius_m`, `source` (`human`/`bot`), `bot_seed`, `godot_version`, `os`, `display_driver`, `accumulated_input`, `delta_source`, `clock` |
| `move` | Every raw mouse motion event | `dx`, `dy`: raw counts before sensitivity. `yaw`, `pitch`: resulting view in degrees |
| `button` | Fire button press or release | `button` (`"fire"`), `pressed` |
| `fire` | Every shot | `shot` (run-wide index), `burst_shot`, `yaw`, `pitch` (view when fired, before recoil), `target`, `target_yaw`, `target_pitch`, `hit` |
| `recoil` | After each shot with a kick | `shot`, `burst_shot`, `kick_yaw`, `kick_pitch` (nominal pattern kick), `yaw`, `pitch` (view after recoil) |
| `target` | Flick or spray target appears | `target`, `target_yaw`, `target_pitch` |
| `end` | Last line | `shots`, `hits`, `complete` (false if the window was closed early) |

Replaying a recording from its own contents gives back every view angle exactly:
start at yaw 0 and pitch 0, add `-dx × deg_per_count` and `-dy × deg_per_count`
for each `move`, and the kick for each `recoil`. The tests check this, and so does
a replay of the bot's output files in a correctly rounded parser.

Precision: floats are written at full round-trip precision, and a correctly
rounded parser reads them back bit for bit; this was checked with Python's parser
on 200,000 values. In Rust, `serde_json` needs its `float_roundtrip` feature for
this, because its default float parsing can be off in the last bit. **Godot's own `JSON.parse_string` and
`String.to_float` are not correctly rounded**, and about 14% of doubles come back
one ulp off. So the tests compare in-memory records for exactness and use parsed
text only for structure.

The file is flushed once per second of scenario time. If the process is killed,
everything but a possibly truncated final line is valid, so discard a trailing
line that does not parse.

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
| `scripts/recorder.gd` | JSON Lines writer (file or memory) and reader |
| `scripts/scripted_bot.gd` | Scripted synthetic player for headless runs |
| `scripts/*_test.gd` | Tests, one file beside each script |
| `tests/` | Test runner and base class |
| `tools/summarize_recording.gd` | Input statistics for recordings |

The bot is an ordinary aiming controller with a reaction delay and seeded noise.
It reads only this range's own state, and its events go through
`Input.parse_input_event` and `_input` like a mouse's. They are tagged with a
reserved device id, so hardware input and bot input never mix.
