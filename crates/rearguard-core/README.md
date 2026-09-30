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
- `telemetry`: the versioned client telemetry schema (decision D2), as JSON Lines.
- `uplink`: the client side of the protocol, for game clients. It streams one
  session's telemetry from a background thread, never blocks the caller, and resumes
  after dropped connections. It has a defined "lost" state (see its docs).
- `protocol`: the client–server wire protocol. Frames are a length prefix, a version
  byte and a postcard message. It covers session seeds, telemetry chunks with replay
  numbers, resume and verdicts; `rearguard-server` speaks it.
- `detect`: the server-side input-probe detector. It streams one player's telemetry,
  replays it with and without the drift, and scores two statistics as signed
  log-likelihood ratios with confidences, per window and per session:
  - fire-time error against drift effect, tested against a strength bound (κ);
  - the per-frame step response to the drift.

  Thresholds come only from `DetectorConfig`, which has no defaults and is never sent
  to a client. Results: `docs/detector-1.3.md`.

Benchmarks: `cargo bench -p rearguard-core --bench probe`. Shape comparison:
`cargo run --release -p rearguard-core --example compare_shapes`.
