# Rearguard: project context

Read this before any work in this repository.

## What Rearguard is

An open-source, zero-access anti-cheat SDK: an engine plugin plus a server module,
with no ring 0 access and identical behaviour on Linux and Windows.

It plants secret, server-seeded probes at the input, information and pixel stages
of the cheat chain. The server holds the seeds and watches who reacts. Detection
rate and false-positive rate are always reported together, never one without the
other.

The repository is private, on a personal GitHub account. Licence not chosen yet.

## Decisions

- **D1 (stack):** Godot 4 demo; plain Rust core crate with no Godot dependency;
  gdext as a thin binding only; Rust server.
- **D2 (telemetry):** the client streams raw relative mouse deltas with per-event
  timestamps, the resulting view angle, and fire events. The server treats all
  of it as untrusted.
- **D5 (seed delivery):** the drift API is built around epochs: the drift is a pure
  function of an epoch seed and a time index. v0 uses a single epoch covering the whole
  match; epoch length is configuration.
- **D10 (MPL-2.0 exception for the Godot binding):** a narrow cargo-deny licence
  exception for the gdext crates (`godot`, `godot-*` 0.5.5, `gdextension-api` 0.5.1),
  which are MPL-2.0. They are reachable only from `rearguard-godot`. gdext's files are
  never modified; any change would be published as MPL-2.0 requires. Recorded in
  `deny.toml` and `docs/licence-notes.md`.

## Constraints

- Rust stable, toolchain pinned in `rust-toolchain.toml`.
- Core crates (`rearguard-core`, `rearguard-sim`, `rearguard-server`) never depend
  on Godot, directly or transitively. Only `rearguard-godot` may.
- `#![forbid(unsafe_code)]` in core, sim and server.
- No ring 0, no kernel drivers, no inspection of other processes, no obfuscation
  or anti-debug tricks. Behaviour identical on Linux and Windows.
- Seeds and derived secrets are never logged, never printed via `Debug` or
  `Display`, and never sent to a client beyond what the protocol requires. Wrap
  them in a type that redacts and zeroizes (a later task adds this type; until
  then, do not introduce raw seed handling).
- Test cheats run only inside Rearguard's own test environment (`rearguard-sim`).
  Nothing aimed at live servers or other people's games.
- Add dependencies sparingly. **Every dependency addition (normal, dev or build)
  must pass `cargo deny --locked check` and be listed in the dependency table
  below with its licence.** Never weaken `deny.toml` to make a crate pass; a
  licence exception or advisory ignore needs an explicit decision, recorded here.
- Licences are permissive-only until decision D3 (leading option: Apache-2.0).
  The allowlist lives in `deny.toml`; copyleft and unknown licences fail.
- Godot crates (`godot`, `godot-*`, `gdext*`, `gdextension*`, `gdnative*`) and
  `rearguard-godot` itself must never enter the graph of core, sim or server,
  even transitively. Enforced by `scripts/check-no-godot.sh`.
- Tests live alongside the code. `cargo fmt`, clippy with warnings as errors, and
  `cargo test` must pass in CI on Linux and Windows.
- Do not add a LICENSE file. The root README states "All rights reserved,
  licence pending".
- Work goes on a branch with a pull request, never straight to `main`.

## Crate map

| Path                      | Purpose                                                         | Godot? |
|---------------------------|-----------------------------------------------------------------|--------|
| `crates/rearguard-core`   | Probes, telemetry schema, detector (`detect`). Engine-agnostic. | Never  |
| `crates/rearguard-sim`    | Closed test environment: synthetic players, test cheats, DR/FPR; `rearguard-sim` CLI. | Never  |
| `crates/rearguard-server` | Issues seeds (derived, never stored), ingests telemetry over the `protocol` (loopback only), runs the detector, stores evidence in SQLite; see its README. | Never  |
| `crates/rearguard-godot`  | Thin gdext binding over core: `RearguardProbe`, `RearguardRecorder` (GDExtension, see its README). | Only here |
| `demo/`                   | Godot 4.7.2 aim range (GDScript, no addons); see `demo/README.md`. | n/a    |

Dependency direction: `sim`, `server` and `godot` depend on `core`; `core`
depends on nothing in the workspace.

## Commands (same as CI)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny --locked check        # cargo install cargo-deny --locked (CI: 0.20.2)
scripts/check-no-godot.sh
```

CI: `.github/workflows/ci.yml`, on every branch push (so pull requests show the
runs as checks). fmt, clippy and tests run on `ubuntu-latest` and
`windows-latest` with build caching; the dependency policy job (cargo-deny plus
the Godot ban) runs on Linux only, because both evaluate the lockfile graph for
every target platform and give the same answer on any host.

The `godot tests (linux)` job runs the demo's GDScript suite headless on
`ubuntu-latest`, separately from the Rust jobs. It uses Godot 4.7.2-stable from
the official `godot-builds` release. The version (`GODOT_VERSION`) and the zip's
SHA-512 (`GODOT_ZIP_SHA512`) are pinned in the workflow, and the checksum is
verified on every run, so a mismatch fails the job. The zip is cached, keyed on
both values. After the import and before the tests, the job builds the Rust extension
(`cargo build --locked -p rearguard-godot`) and sets `REARGUARD_REQUIRE_EXTENSION=1`,
so the extension tests fail rather than skip if it does not load. Import comes first
because Godot 4.7.2 crashes on shutdown after a first import that loads a GDExtension
(see `crates/rearguard-godot/README.md`). To run it locally,
use the 4.7.2 binary as `godot`:

```sh
godot --headless --path demo --import          # once per fresh checkout, BEFORE the build
cargo build -p rearguard-godot -p rearguard-server   # the GDExtension library and the server the live-loop tests run
REARGUARD_REQUIRE_EXTENSION=1 godot --headless --path demo --fixed-fps 120 --script res://tests/run_tests.gd
```

To bump Godot, change `GODOT_VERSION` and `GODOT_ZIP_SHA512` together; the
checksum comes from the release's `SHA512-SUMS.txt`. The full steps are in
`demo/README.md`, under "CI".

## Third-party dependencies

| Crate | Used by | Licence | Why |
|-------|---------|---------|-----|
| `chacha20` 0.10 (features `rng`, `zeroize`) | core | MIT OR Apache-2.0 | ChaCha20 CSPRNG for probe signals (RustCrypto; what `rand` 0.10 uses); zeroizes its state on drop |
| `hkdf` 0.13 | core | MIT OR Apache-2.0 | HKDF (RFC 5869) for the probe key hierarchy |
| `sha2` 0.11 (feature `zeroize`) | core | MIT OR Apache-2.0 | SHA-256 for HKDF |
| `zeroize` 1.9 | core | Apache-2.0 OR MIT | Wipes secrets on drop |
| `criterion` 0.8 (no default features) | core (dev) | Apache-2.0 OR MIT | Benchmarks |
| `proptest` 1.11 (feature `std` only) | core (dev) | MIT OR Apache-2.0 | Property tests |
| `serde` 1 (feature `derive`) | core, sim | MIT OR Apache-2.0 | Telemetry schema and sim manifest serialisation |
| `serde_json` 1.0.151 (feature `float_roundtrip`) | core, sim | MIT OR Apache-2.0 | JSON Lines telemetry; `float_roundtrip` makes float parsing exact |
| `chacha20` 0.10 (feature `rng`) | sim | MIT OR Apache-2.0 | Deterministic random streams for the simulator (already used by core) |
| `libm` 0.2 | core, sim | MIT | Pure-Rust maths functions, so simulations and detector scores are bit-identical across platforms |
| `zeroize` 1.9 | sim | Apache-2.0 OR MIT | Wipes the simulation seed on drop (already used by core) |
| `postcard` 1.1 (feature `alloc`, no default features) | core | MIT OR Apache-2.0 | Binary encoding of wire-protocol messages |
| `tokio` 1.53 (features `rt-multi-thread`, `net`, `io-util`, `time`, `sync`, `macros`, `signal`) | server | MIT | Async runtime and TCP |
| `rusqlite` 0.40 (feature `bundled`) | server | MIT | Evidence store; `bundled` compiles SQLite (public domain) from source so Linux and Windows use the same version |
| `getrandom` 0.3 | server | MIT OR Apache-2.0 | Master secrets, session ids and resume tokens from the OS CSPRNG |
| `serde` 1 (feature `derive`), `serde_json` 1.0.151 (feature `float_roundtrip`) | server | MIT OR Apache-2.0 | Config file and `verdict` output (already used by core and sim) |
| `zeroize` 1.9 | server | Apache-2.0 OR MIT | Wipes the master secret file's text (already used by core) |
| `godot` (gdext) `=0.5.5` (feature `api-4-7`, no default features) | godot | **MPL-2.0 (exception, D10)** | The GDExtension binding. Pulls in `godot-*` 0.5.5 and `gdextension-api` 0.5.1, all MPL-2.0 and all listed in the `deny.toml` exception |
| `getrandom` 0.3 | godot | MIT OR Apache-2.0 | OS CSPRNG for creating probe seed files, so seeds never pass through GDScript |
| `zeroize` 1.9 | godot | Apache-2.0 OR MIT | Wipes seed text read from files (already used by core) |

Every row must match a crate in `Cargo.lock` that passed `cargo deny`. The table lists
direct dependencies; `cargo deny` checks their transitive crates too, and every one
resolves to an allowlisted licence.

CI tooling (not crates, pinned by commit SHA in the workflow):
`actions/cache` (MIT), `taiki-e/install-action` (Apache-2.0 OR MIT),
`cargo-deny` (Apache-2.0 OR MIT). The Godot job downloads the Godot 4.7.2-stable
editor binary (MIT), pinned by SHA-512 rather than by commit. Keep CI tooling
permissive too.
