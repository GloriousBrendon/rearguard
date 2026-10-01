# Rearguard: project context

Read this before any work in this repository.

## What Rearguard is

An open-source, zero-access anti-cheat SDK: an engine plugin plus a server module,
with no ring 0 access and identical behaviour on Linux and Windows.

It plants secret, server-seeded probes at the input, information and pixel stages
of the cheat chain. The server holds the seeds and watches who reacts. Detection
rate and false-positive rate are always reported together, never one without the
other.

The repository is public, on a personal GitHub account. Licensed MIT OR Apache-2.0
(decision D3). It is an early prototype: `README.md`, "Status and limits", says what
exists and what the results mean, and must stay true. Bypasses and vulnerabilities are
reported privately (`SECURITY.md`); `CONTRIBUTING.md` lists the checks.

## Decisions

- **D1 (stack):** Godot 4 demo; plain Rust core crate with no Godot dependency;
  gdext as a thin binding only; Rust server.
- **D2 (telemetry):** the client streams raw relative mouse deltas with per-event
  timestamps, the resulting view angle, and fire events. The server treats all
  of it as untrusted.
- **D3 (project licence):** dual licence, `MIT OR Apache-2.0`, the Rust convention.
  Anyone may use, modify and redistribute Rearguard, including in closed-source games.
  Apache-2.0 is included for its explicit patent grant and patent-retaliation clause.
  Texts in `LICENSE-MIT` and `LICENSE-APACHE`; `license` set in the workspace manifest
  and inherited by every crate; every first-party source file carries an SPDX header.
  Third-party and vendored code keeps its own licence (the gdext crates stay MPL-2.0,
  D10). The dependency policy in `deny.toml` is unchanged by D3.
- **D4 (name):** the project keeps the name Rearguard. It is not affiliated with the
  Re:Guard anti-cheat research project, and the README says so.
- **D5 (seed delivery):** the drift API is built around epochs: the drift is a pure
  function of an epoch seed and a time index. v0 uses a single epoch covering the whole
  match; epoch length is configuration.
- **D10 (MPL-2.0 exception for the Godot binding):** a narrow cargo-deny licence
  exception for the gdext crates (`godot`, `godot-*` 0.5.5, `gdextension-api` 0.5.1),
  which are MPL-2.0. They are reachable only from `rearguard-godot`. gdext's files are
  never modified; any change would be published as MPL-2.0 requires. Recorded in
  `deny.toml` and `docs/licence-notes.md`.
- **D11 (Godot export-template components):** the third-party components compiled into
  the official Godot release export templates are accepted for the study builds, with
  their notices shipped in every build (`THIRD-PARTY-NOTICES.txt`). This covers the
  non-permissive or credit-requiring ones too: Mozilla's CA certificate bundle
  (MPL-2.0, unmodified data, unused by the study), FreeType (FTL), the fonts (OFL-1.1)
  and the Godot logo (CC-BY-4.0). The templates are used unmodified. D11 is about
  distributed binaries, not crates: `deny.toml` is unchanged by it.

## Constraints

- Rust stable, toolchain pinned in `rust-toolchain.toml`.
- Core crates (`rearguard-core`, `rearguard-sim`, `rearguard-server`) and `xtask` never
  depend on Godot, directly or transitively. Only `rearguard-godot` may.
- `#![forbid(unsafe_code)]` in core, sim, server and xtask.
- No ring 0, no kernel drivers, no inspection of other processes, no obfuscation
  or anti-debug tricks. Behaviour identical on Linux and Windows.
- Seeds and derived secrets are never logged, never printed via `Debug` or
  `Display`, and never sent to a client beyond what the protocol requires. Wrap
  them in a type that redacts and zeroizes (a later task adds this type; until
  then, do not introduce raw seed handling).
- Test cheats run only inside Rearguard's own test environment (`rearguard-sim`, and
  the aim range's in-process test bots in `demo/scripts/test_cheats/`, which refuse to
  run without `REARGUARD_TEST_ENV=1` and a loopback or allowlisted server). Nothing
  aimed at live servers or other people's games. Ground-truth labels are session
  metadata for the evaluation harness only; the detector never reads them.
- Evaluation (`cargo xtask eval`, task 1.12): thresholds are calibrated on one set of
  sessions and false-positive rates measured on another. Human sessions are split by
  participant, and the split is enforced by types (`xtask::split`): never calibrate on
  an evaluation session, and never report a false-positive rate from calibration
  sessions. Rates carry 95% Wilson intervals. Every result file records the git
  revision, the configuration and digests of its inputs, and no seed, key, path or
  participant id.
- Add dependencies sparingly. **Every dependency addition (normal, dev or build)
  must pass `cargo deny --locked check` and be listed in the dependency table
  below with its licence.** Never weaken `deny.toml` to make a crate pass; a
  licence exception or advisory ignore needs an explicit decision, recorded here.
- Dependency licences are permissive-only (D3 did not change this). The allowlist
  lives in `deny.toml`; copyleft and unknown licences fail.
- Godot crates (`godot`, `godot-*`, `gdext*`, `gdextension*`, `gdnative*`) and
  `rearguard-godot` itself must never enter the graph of core, sim or server,
  even transitively. Enforced by `scripts/check-no-godot.sh`.
- Tests live alongside the code. `cargo fmt`, clippy with warnings as errors, and
  `cargo test` must pass in CI on Linux and Windows.
- Rearguard is `MIT OR Apache-2.0` (D3). Every first-party source file (`.rs`, `.gd`,
  `.sh`, `.py`, and any new source type in its own comment syntax) starts with
  `SPDX-License-Identifier: MIT OR Apache-2.0` as its first line, or its second after a
  shebang; `scripts/check-spdx.sh` enforces it. Never add the header to, or relicense,
  third-party or vendored code. The licence texts in `LICENSE-MIT` and `LICENSE-APACHE`
  are the unmodified originals.
- Work goes on a branch with a pull request, never straight to `main`.
- The repository is public: nothing secret or personal is committed, and CI never
  uploads a study key or a build that packs one. Study releases for volunteers are built
  locally with a key kept outside the repository
  (`docs/study/facilitator-instructions.md`). The only study build CI uploads packs the
  public test key (`demo/tests/public-test-key.hex`), for the Windows smoke job.
- Every action in the workflow is pinned to a full commit SHA, with the version in a
  comment.

## Crate map

| Path                      | Purpose                                                         | Godot? |
|---------------------------|-----------------------------------------------------------------|--------|
| `crates/rearguard-core`   | Probes, telemetry schema, detector (`detect`). Engine-agnostic. | Never  |
| `crates/rearguard-sim`    | Closed test environment: synthetic players, test cheats, DR/FPR; `rearguard-sim` CLI. | Never  |
| `crates/rearguard-server` | Issues seeds (derived, never stored), ingests telemetry over the `protocol` (loopback only), runs the detector, stores evidence in SQLite; see its README. | Never  |
| `crates/rearguard-godot`  | Thin gdext binding over core: `RearguardProbe`, `RearguardRecorder` (GDExtension, see its README). | Only here |
| `crates/xtask`            | `cargo xtask eval`: simulated, test-bot and human sessions through the detector; DR and FPR together, per-scenario thresholds; see its README. | Never  |
| `demo/`                   | Godot 4.7.2 aim range (GDScript, no addons) and the human-study scene; see `demo/README.md`. Study builds for volunteers: `scripts/export-study.sh`. | n/a    |

Dependency direction: `sim`, `server` and `godot` depend on `core`; `core`
depends on nothing in the workspace. `xtask` depends on `core` and `sim`.

The server's `detector` setting is one threshold set for every scenario, or one per
scenario (`rearguard_core::detect::DetectorSet`, task 1.12); the evaluation writes the
per-scenario form.

## Commands (same as CI)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny --locked check        # cargo install cargo-deny --locked (CI: 0.20.2)
scripts/check-no-godot.sh
scripts/check-spdx.sh
```

The detector evaluation (task 1.12; not part of CI's checks on the code, about a minute):

```sh
cargo xtask eval --seed N        # options and outputs: crates/xtask/README.md
```

CI: `.github/workflows/ci.yml`, on pushes to `main`, on every pull request and on
request (`workflow_dispatch`); a pull request from a branch of this repository runs
once. fmt, clippy and tests run on `ubuntu-latest` and
`windows-latest` with build caching; the dependency policy job (cargo-deny plus
the Godot ban) runs on Linux only, because both evaluate the lockfile graph for
every target platform and give the same answer on any host. The `licence headers` job
(`scripts/check-spdx.sh`, Linux only, since it reads the same tracked files on any host)
fails if a first-party source file lacks its SPDX header.

The `godot tests (linux)` and `godot tests (windows)` jobs run the demo's GDScript
suite headless on `ubuntu-latest` and `windows-latest` (Git Bash), separately from the
Rust jobs. It uses Godot 4.7.2-stable from
the official `godot-builds` release. The version (`GODOT_VERSION`) and the zip's
SHA-512 (`GODOT_ZIP_SHA512`) are pinned in the workflow, and the checksum is
verified on every run, so a mismatch fails the job. The zip is cached, keyed on
both values. After the import and before the tests, the job builds the Rust extension
(`cargo build --locked -p rearguard-godot`) and sets `REARGUARD_REQUIRE_EXTENSION=1`,
so the extension tests fail rather than skip if it does not load. The import goes
through `scripts/godot-import.sh`, which fails if Godot prints any error other than the
missing extension library. The test runner fails if fewer tests ran than
`demo/tests/expected_test_count.txt` says; raise that number when adding tests. The Linux job then
records the aim range's test bots (`scripts/record-test-bots.sh`) and runs
`cargo xtask eval --strict` on the recordings, so a recording that no longer replays
fails the job. Import comes first
because Godot 4.7.2 crashes on shutdown after a first import that loads a GDExtension
(see `crates/rearguard-godot/README.md`). To run it locally,
use the 4.7.2 binary as `godot`:

```sh
scripts/godot-import.sh godot                  # once per fresh checkout, BEFORE the build
cargo build -p rearguard-godot -p rearguard-server   # the GDExtension library and the server the live-loop tests run
REARGUARD_REQUIRE_EXTENSION=1 godot --headless --path demo --fixed-fps 120 --script res://tests/run_tests.gd
```

To bump Godot, change `GODOT_VERSION` and every pinned checksum (`GODOT_ZIP_SHA512`,
`GODOT_WIN_ZIP_SHA512`, `GODOT_TEMPLATES_SHA512`) in every job together; the checksums
come from the release's `SHA512-SUMS.txt`. The full steps are in `demo/README.md`, under
"CI".

Study builds (task 1.9a): CI checks that the study build still exports and works; it
makes no releases. The `study build (export, linux smoke)` job exports the Linux build
(`scripts/export-study.sh --only linux`) with the official 4.7.2 release templates
(SHA-512 checked every run) and a throwaway key made in the job. It smoke-tests that
build (`scripts/smoke-study-build.sh`), runs
`cargo xtask eval --strict --allow-test-exports` on the export the smoke test wrote (so a
change to the export format that the evaluation cannot read fails the job), then deletes
the key and the build. Neither is uploaded. It also exports the Windows build
(`--only windows`) with the release DLL from `godot tests (windows)` and the public test
key, and uploads it for one day; `study build (windows smoke)` smoke-tests it with the
key from the checkout. Releases for volunteers are built locally, with a key that is
never committed and stays with the facilitator
(`docs/study/facilitator-instructions.md`).

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
| `serde_json` 1.0.151 (feature `float_roundtrip`) | godot | MIT OR Apache-2.0 | Study plan JSON for `RearguardStudy` (already used by core) |
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
| `miniz_oxide` 0.9 (feature `with-alloc`, no default features) | xtask | MIT OR Zlib OR Apache-2.0 | Inflates the study export zips (task 1.12). Pulls in `adler2` (0BSD OR MIT OR Apache-2.0) |
| `sha2` 0.11 | xtask | MIT OR Apache-2.0 | Participant split and input digests (already used by core) |
| `serde` 1 (feature `derive`), `serde_json` 1.0.151 (feature `float_roundtrip`) | xtask | MIT OR Apache-2.0 | Evaluation config, export manifests, result files (already used by core) |
| `libm` 0.2 | xtask | MIT | Platform-independent maths for intervals and plots (already used by core) |
| `zeroize` 1.9 | xtask | Apache-2.0 OR MIT | Wipes key file text (already used by core) |

Every row must match a crate in `Cargo.lock` that passed `cargo deny`. The table lists
direct dependencies; `cargo deny` checks their transitive crates too, and every one
resolves to an allowlisted licence.

CI tooling (not crates, pinned by commit SHA in the workflow):
`actions/cache` (MIT), `actions/upload-artifact` and `actions/download-artifact`
(MIT), `taiki-e/install-action` (Apache-2.0 OR MIT), `cargo-deny` (Apache-2.0 OR MIT).
The Godot jobs download the Godot 4.7.2-stable editor binaries for Linux and Windows
(MIT), pinned by SHA-512 rather than by commit. Keep CI tooling permissive too.

Godot export templates (shipped inside the study builds, task 1.9a): the official
4.7.2-stable release templates `linux_release.x86_64` and
`windows_release_x86_64.exe`, from `Godot_v4.7.2-stable_export_templates.tpz`, pinned
by SHA-512. Godot itself is MIT. The compiled-in third-party components are
MIT/Expat, BSD-2-Clause, BSD-3-Clause, Apache-2.0, Zlib, Unlicense, MIT-0, CC0-1.0,
BSL-1.0, Unicode, X11, IJG, and the HarfBuzz and glslang licences; also FreeType under
FTL (BSD-style with a credit clause), fonts under OFL-1.1, the Godot logo under
CC-BY-4.0, and Mozilla's CA certificate bundle under **MPL-2.0** (unmodified data,
unused by the study). All accepted by decision D11, with the notices shipped. Details
and the notices shipped in each build: `docs/licence-notes.md`.
