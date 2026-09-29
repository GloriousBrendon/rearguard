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
- Add dependencies sparingly; record each one and its licence in the table below.
- Tests live alongside the code. `cargo fmt`, clippy with warnings as errors, and
  `cargo test` must pass in CI on Linux and Windows.
- Do not add a LICENSE file. The root README states "All rights reserved,
  licence pending".
- Work goes on a branch with a pull request, never straight to `main`.

## Crate map

| Path                      | Purpose                                                         | Godot? |
|---------------------------|-----------------------------------------------------------------|--------|
| `crates/rearguard-core`   | Probes, protocol types, detection logic. Engine-agnostic.       | Never  |
| `crates/rearguard-sim`    | Closed test environment: synthetic players, test cheats, DR/FPR. | Never  |
| `crates/rearguard-server` | Holds seeds, ingests untrusted telemetry, judges reactions.     | Never  |
| `crates/rearguard-godot`  | Thin gdext binding over core (placeholder, no gdext yet).       | Only here |
| `demo/`                   | Reserved for the Godot 4 demo project.                          | n/a    |

Dependency direction: `sim`, `server` and `godot` depend on `core`; `core`
depends on nothing in the workspace.

## Commands (same as CI)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI: `.github/workflows/ci.yml`, matrix `ubuntu-latest` and `windows-latest`, on
every branch push (so pull requests show the runs as checks).

## Third-party dependencies

| Crate | Used by | Licence | Why |
|-------|---------|---------|-----|
| _none yet_ | | | |
