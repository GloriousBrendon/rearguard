# rearguard-core

Engine-agnostic heart of Rearguard: probe definitions, the client/server telemetry
protocol types, and detection logic. Both the engine plugin and the server build on it.

**No Godot dependency.** This crate must never depend on Godot, gdext or any other
engine crate, directly or transitively. `unsafe` code is forbidden.

## Modules

- `probe`: the input probe's secret drift signal for mouse sensitivity and recoil
  scale, and its key hierarchy (root → match → player → epoch → stream, via
  HKDF-SHA256). It is a pure, integer-only function of (epoch seed, tick), so it is
  bit-identical on every platform. See `docs/probe-signal-shapes.md` for the choice
  of signal shape.
- `secret`: `SecretKey`, which is redacted in `Debug`/`Display` and zeroized on drop.

Benchmarks: `cargo bench -p rearguard-core --bench probe`. Shape comparison:
`cargo run --release -p rearguard-core --example compare_shapes`.
