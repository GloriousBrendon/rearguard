# rearguard-godot

The thin gdext binding that exposes `rearguard-core` to Godot 4 as a GDExtension. It
holds only glue: it converts between Godot and Rust types. All maths (the drift, the
probe-tick rule, telemetry serialisation) lives in `rearguard-core`.

This is the only crate allowed to depend on Godot. Core, sim and server never do
(`scripts/check-no-godot.sh`).

## Pins

| What | Pinned to | Why |
|------|-----------|-----|
| Godot | 4.7.2-stable | The engine the aim range (`demo/`) and its CI job use; see `demo/README.md` |
| `godot` (gdext) crate | `=0.5.5`, feature `api-4-7`, no default features | 0.5.5 is the newest gdext release. Its minimum Rust (1.94) is our pinned toolchain. `api-4-7` builds against the Godot 4.7 API, which matches the pinned engine; an extension built for 4.7 loads in 4.7.x and later 4.x, but not earlier. Default features are off, so only the few engine classes the binding uses are generated, which keeps the build short |
| `.gdextension` | `compatibility_minimum = "4.7"` | The same boundary, enforced by Godot at load time |

The pin is exact (`=0.5.5`) because the licence exception for gdext (decision D10) in
`deny.toml` names these exact versions. A bump must re-check the licences and update
both files.

gdext is MPL-2.0. Decision D10 allows it here only, and gdext's own files are never
modified; see `docs/licence-notes.md`.

## What GDScript gets

**`RearguardProbe`** (RefCounted): the input probe for one session. Decision D5, v0: one
epoch covering the whole session.

| Method | Returns | Meaning |
|--------|---------|---------|
| `start_session(epoch_seed: PackedByteArray, amplitude_ppm, probe_start_us)` | `Error` | From 32 seed bytes, as a server would deliver them. The caller's array cannot be wiped from Rust, so prefer the next two |
| `start_session_from_file(path, create_if_missing, amplitude_ppm, probe_start_us)` | `Error` | From a file of 64 hex digits. With `create_if_missing`, a missing file is created with a fresh seed from the OS CSPRNG (owner-only on Unix). The seed never passes through GDScript |
| `start_session_random(amplitude_ppm, probe_start_us)` | `Error` | A random seed held only in memory. The drift is applied but can never be analysed afterwards |
| `start_session_derived(key_path, match_id, player_id, amplitude_ppm, probe_start_us)` | `Error` | From a root key file (a study key) through the probe key hierarchy. `key_path` is a file-system path, or a `res://` path for a key packed into an exported study build, which Rust reads through Godot's `FileAccess` and wipes after parsing |
| `sensitivity_multiplier(ts_us)` | `float` | Multiplier for the view change of a raw delta at client timestamp `ts_us`; 1.0 without a session |
| `recoil_scale(ts_us)` | `float` | Multiplier for the recoil kick of a shot at `ts_us`; 1.0 without a session |
| `drift_ppb(stream, tick)`, `multiplier_at_tick(stream, tick)` | `int`, `float` | By probe tick (`STREAM_SENSITIVITY` = 0, `STREAM_RECOIL` = 1), for tests |
| `stop()`, `is_active()`, `amplitude_ppm()`, `probe_start_us()` | | Session control |
| `default_amplitude_ppm()`, `max_amplitude_ppm()` | `int` | 5000 (0.5%) and 20000 (2%) |

Timestamps map to probe ticks by the telemetry rule
`rearguard_core::telemetry::probe_tick`: `(ts_us - probe_start_us) / 1000`. Amplitudes
above 2% and negative timestamps are refused.

**`RearguardClient`** (RefCounted): a live link to a Rearguard server. It wraps
`rearguard_core::uplink`.

| Method | Meaning |
|--------|---------|
| `connect_to(address, client_name) -> Error` | Opens a server session. Blocks for at most a few seconds |
| `record_header(dict)`, `record_move(...)` and the other `record_*` | Same calls as the recorder. Each one only queues the record; a worker thread sends it |
| `finish()`, `is_done()`, `close()` | End the session, wait for the final verdict, stop the worker |
| `status()` | `"connected"`, `"reconnecting"`, `"finishing"`, `"finished"`, `"lost: <reason>"` or `"not connected"` |
| `session_id()`, `match_id()`, `player_id()`, `amplitude_ppm()` | What the server assigned (never the seed) |
| `verdict()`, `stats()`, `request_verdict()` | **Debug builds only** (`#[cfg(debug_assertions)]`), for the developer overlay: the latest verdict, and uplink counters |

`RearguardProbe.start_session_from_client(client, probe_start_us)` moves the
server-issued seed from the client into the probe, inside Rust. The seed can be taken
once, and GDScript never sees it.

**`RearguardRecorder`** (RefCounted): writes client telemetry in the
`rearguard.telemetry` v1 schema, with exact floats.
- `open(path, header: Dictionary) -> Error` takes the `Header` fields except `format`
  and `version`.
- The `record_*` methods (`move`, `button`, `fire`, `recoil`, `target`, `end`) return
  `false` after an error or an invalid argument.
- `flush()` and `close()` finish the file.

The aim range installs the probe on its two hooks (`demo/scripts/probe_hooks.gd`) and
mirrors its recording into a recorder when asked (`--telemetry`); see `demo/README.md`.

## Building

From the repository root, with the pinned toolchain (`rust-toolchain.toml`) and the
Godot 4.7.2 editor binary as `godot`. On a fresh checkout, import the project *before*
building the library (see "Known issue" below):

```sh
godot --headless --path demo --import     # fresh checkout only, before the first build
cargo build -p rearguard-godot            # debug: what `godot` and `godot --headless` load
cargo build -p rearguard-godot --release  # release: for exported builds
```

This produces `target/debug/librearguard_godot.so` (Linux) or
`target/debug/rearguard_godot.dll` (Windows). `demo/rearguard.gdextension` loads the
library straight from `target/`, so nothing needs copying. No Godot binary or C
toolchain is needed to build: gdext ships the 4.7 API.

Then run the tests:

```sh
REARGUARD_REQUIRE_EXTENSION=1 godot --headless --path demo --fixed-fps 120 --script res://tests/run_tests.gd
```

Without `REARGUARD_REQUIRE_EXTENSION=1`, the extension tests are skipped when the library
has not been built. With it (as in CI), they fail instead.

Linux and Windows x86_64 only; macOS, Android, iOS and web are out of scope.

### Known issue: the first import crashes when the library is present

Godot 4.7.2 segfaults while shutting down after the project's *first* headless
`--import` (no `demo/.godot/` yet) if the extension library is already built:
- exit status 134;
- the stack is inside Godot's own binary, with no frames in our library.

The import itself completes. Every later run is unaffected: a second import, the
editor, or the tests.

It is not caused by anything in this crate. An extension with one empty class
(`#[derive(GodotClass)] struct Trivial {}`) crashes the same way, and one with no
classes does not. So it looks like an interaction between gdext 0.5.5 and Godot 4.7.2.
It is not yet reported upstream.

The workaround, used by CI too, is to import before building the library on a fresh
checkout. If the library already exists, move it away for the import, or just run the
import a second time. The windowed editor has not been checked for the same behaviour.

## Tests

- `cargo test -p rearguard-godot` checks the conversions, which need no engine:
  - hex seed files, including creation and permissions;
  - integer range checks;
  - header fields;
  - `demo/tests/probe_golden.json` against `rearguard-core`, byte for byte. Regenerate
    it with `cargo test -p rearguard-godot print_probe_golden -- --ignored --nocapture`.
- The headless Godot tests check the loaded extension:
  - `demo/scripts/probe_binding_test.gd` checks the core golden vectors bit for bit,
    the timestamp rule, seed files, refusals and the recorder;
  - `demo/scripts/aim_range_test.gd` checks the drift is applied through the hooks.

## Unsafe code

One `unsafe impl ExtensionLibrary`, the entry point gdext requires. Everything else is
safe Rust. The `forbid(unsafe_code)` rule covers core, sim and server, not this crate.
