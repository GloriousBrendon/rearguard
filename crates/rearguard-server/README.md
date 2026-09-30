# rearguard-server

The Rearguard server (task 1.6). It issues session seeds, ingests untrusted client
telemetry, runs the input-probe detector (`rearguard_core::detect`) and stores the
evidence in SQLite.

**Loopback only.** TLS and authentication are task 3.3; until then the server refuses
any listen address that is not loopback.

**No Godot dependency.** This crate must never depend on Godot, gdext or any other
engine crate, directly or transitively. `unsafe` code is forbidden.

## Running

```sh
cargo run --release -p rearguard-server -- gen-secret server-master.hex   # once; never overwrites
cp crates/rearguard-server/config/server.example.json server.json          # then edit
cargo run --release -p rearguard-server -- run --config server.json        # until Ctrl-C
cargo run --release -p rearguard-server -- verdict --db rearguard.sqlite3 --session ID
```

Relative paths in the config are resolved from the config file's directory. Ctrl-C
stops the server gracefully: it closes every connection and stores every open session
as `abandoned`, with its evidence.

## Configuration

JSON, with every field required and unknown fields refused. There are no built-in
defaults, so every threshold and limit is a deliberate choice. See
`config/server.example.json`.

| Field | Meaning |
|-------|---------|
| `listen` | Loopback address and port (`127.0.0.1:0` picks a free port) |
| `master_secret_file` | 64 hex digits, from `gen-secret` (owner-only permissions on Unix) |
| `database` | SQLite file, created if missing |
| `amplitude_ppm` | Drift amplitude handed to clients, 0 to 20000 |
| `detector` | `rearguard_core::detect::DetectorConfig`: κ bound, flag scores, window length, and so on |
| `limits.max_frame_bytes` | Largest frame payload; a longer prefix closes the connection before anything is read |
| `limits.max_records_per_message` | Most telemetry records in one message |
| `limits.messages_per_second` / `message_burst` | Per-connection message token bucket |
| `limits.bytes_per_second` / `byte_burst` | Per-connection byte token bucket |
| `limits.idle_timeout_ms` | A connection with no complete frame for this long is closed (half-open peers) |
| `limits.resume_timeout_ms` | How long a dropped session can be resumed before it is closed as `abandoned` |
| `limits.max_open_sessions` | Server-wide cap |

The example config's detector thresholds are the task-1.3 sequential calibration at
0.5% amplitude for flick play:
- `error_flag_score` is 11.25;
- `steps_flag_score` is 4.25;
- the budget is a 0.1% false-positive rate over 15 minutes, against simulated humans
  only.

These must be recalibrated on real players (task 1.10).

## Seeds and secrets

- **Derivation (decision D5, v0):** a session's seed is derived as master secret →
  `match_id` → `player_id` → epoch 0, using HKDF-SHA256 from `rearguard_core::probe`.
  One epoch covers the whole match. v0 has one player per match (`player_id` 1).
- **What is stored:** the database holds only the identifiers. Seeds are never
  stored, and can be re-derived whenever needed.
- **What crosses the wire:** the seed goes to the client exactly once, in `Welcome`,
  inside a type that redacts itself in `Debug` and is wiped on drop. The master secret
  lives in a `RootSeed` with the same properties.
- **Resume tokens:** each session gets a random 16-byte capability. It is compared in
  constant time, redacted in `Debug`, and never logged or stored.
- **Logs:** log lines carry only session, match and connection ids, counts and scores.
  `tests/secrets.rs` checks this in CI:
  - it collects the master secret, every issued seed and every token (in hex, upper
    hex, 6-byte prefixes and Debug byte-array form);
  - it scans the captured logs of normal, error, resume, abandon and shutdown flows;
  - it scans the real binary's stderr, `gen-secret` and `verdict` output, and the raw
    bytes of the database file;
  - a negative-control test proves the scanner does catch those forms.

## Protocol

This is `rearguard_core::protocol`. Each frame is a `u32` little-endian length, then a
version byte (1), then a postcard-encoded message.

1. `Hello`: the server opens a session and replies `Welcome` (session id, resume
   token, `match_id`, `player_id`, epoch 0, epoch seed, amplitude).
2. `Telemetry { seq, records }` chunks, with `seq = 0, 1, 2, ...` and no gaps. Each is
   acknowledged with `Ack`.
3. `Finish { seq = next }`: the server replies with the final `Verdict`.
4. `Verdict { session_id }` works for open sessions (live) and closed ones (from the
   store).
5. `Resume { session_id, token }` re-attaches a session after a dropped connection.
   The server replies `Resumed { next_seq }`.

Rejections, as `Error { code }`:

| Problem | Code | Connection |
|---|---|---|
| Oversized, empty, undecodable or wrong-version frame | `Protocol(..)` | closed |
| Rate limit exceeded | `RateLimited` | closed |
| Repeated chunk number (a replay) | `Replayed` | stays open |
| Skipped chunk number | `OutOfOrder` | stays open |
| Telemetry for a session this connection doesn't hold | `NotAllowed` | stays open |
| Non-finite numbers, too many records, or records the detector refuses | `InvalidTelemetry` | stays open |
| Bad resume token | `BadToken` | stays open |
| Session attached to another connection | `SessionBusy` | stays open |
| Too many open sessions | `ServerBusy` | stays open |

A rejected chunk still uses up its number. After an invalid record, the records before
it in the chunk have already been counted.

## Session lifecycle

`open` → `finished` (the client sent `Finish`), or `open` → `abandoned`:
- the client's connection dropped, or went silent past `idle_timeout_ms`;
- the session was not resumed within `resume_timeout_ms`;
- or the server shut down.

Either way, the final evidence and verdict are stored.

## Evidence store

| Table | Holds |
|-------|-------|
| `sessions` | Id, `match_id`, `player_id`, amplitude, client name, status, created/ended time, record count |
| `evidence` | Per completed 30 s window and for the whole session: both statistics (pairs, r, slope, slope se, residual sd, κ, z, score, confidence, flagged). Windows are written as they complete |
| `verdicts` | The final verdict per session: score, confidence, flagged, counts, flagged windows |

The verdict score is the higher of the two statistics' scores (fire-time error and
step response). The verdict is flagged if either is.

## Tests

| File | Covers |
|------|--------|
| `tests/end_to_end.rs` | Acceptance criteria 1 and 5. Simulated clients (`rearguard-sim` flick, smoothing and human players, playing under server-issued seeds) through the server to verdicts, via `Finish`, the `Verdict` API and the database. Checks that seeds re-derive from the master secret |
| `tests/robustness.rs` | Criterion 4, and criterion 2 at the socket: oversized, empty, malformed and wrong-version frames; random byte streams; replays and skips; invalid telemetry; rate limits; the session cap; disconnect and resume; abandonment after the resume timeout; half-open connections past the idle timeout; graceful shutdown; non-loopback refusal |
| `tests/secrets.rs` | Criterion 3, as above |
| `rearguard-core` `protocol` tests | Criterion 2: property tests. Arbitrary bytes never panic, every message round-trips, corrupted and truncated payloads are handled, and length prefixes are checked before allocation |
