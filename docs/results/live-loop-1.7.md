# Live loop: frame-time cost and bandwidth (task 1.7)

Measured on 2026-09-30 with `scripts/measure-live-loop.sh`.

**Setup:**
- Linux 7.2 (CachyOS), AMD Ryzen 7 7700X, Godot 4.7.2-stable.
- The server (debug build) ran on the same machine, over loopback.
- 3 repeats of 60 s of play per row; each run is 7,198 frames.
- Medians are over the repeats. Each run's p50 and p99 are over its frames.

**Method:**
- The demo's scripted bot plays headless with `--fixed-fps 120`, so frames run back to
  back. A frame's wall time is then its processing cost, with no rendering and no idle
  wait.
- `off` is `--no-probe --no-record`; `probe-recorder` is the default (probe from a local
  seed, telemetry file on); `probe-recorder-uplink` adds `--server`.
- The developer overlay was on in every configuration (debug build).

## Frame time

### Optimised library

The extension was built with `CARGO_PROFILE_DEV_OPT_LEVEL=3`, which is representative of
a release build. Frame times in µs:

| Scenario | Configuration | p50 | p99 | Mean |
|---|---|---:|---:|---:|
| flick | off | 10 | 74 | 29 |
| flick | probe + recorder | 11 | 88 | 33 |
| flick | probe + recorder + uplink | 11 | 235 | 41 |
| spray | off | 57 | 78 | 53 |
| spray | probe + recorder | 63 | 90 | 60 |
| spray | probe + recorder + uplink | 71 | 245 | 72 |

### Unoptimised library

The default `cargo build -p rearguard-godot` output, which is what `godot` loads out of the
box. Frame times in µs:

| Scenario | Configuration | p50 | p99 |
|---|---|---:|---:|
| flick | off | 10 | 79 |
| flick | probe + recorder | 11 | 131 |
| flick | probe + recorder + uplink | 12 | 288 |
| spray | off | 57 | 79 |
| spray | probe + recorder | 89 | 134 |
| spray | probe + recorder + uplink | 103 | 307 |

**Reading the numbers:**

- **Probe and recorder:** with the optimised library, they add 1 to 6 µs at p50 and
  about 12 to 14 µs at p99. That is under 0.2% of a 120 Hz frame (8,333 µs).
- **Uplink:** adds little at p50. At p99 it adds about 160 µs, for a worst p99 of
  245 µs, or 3% of a 120 Hz frame. Recording calls only queue a record for the uplink's
  worker thread, so the spikes are most likely that thread and the server competing for
  the CPU. In these runs a whole minute of play takes about half a second of wall time,
  so the uplink sends 60 s worth of chunks in that half second. That exaggerates the
  contention compared with real-time play.
- **The debug build:** unoptimised Rust roughly doubles the probe and recorder cost.
  Use an optimised build for measurements.

## Bandwidth

Bytes the client sends, including framing, acks excluded:

| Scenario | Records per minute of play | Uplink KiB per minute | Bytes per record |
|---|---:|---:|---:|
| flick (bot) | 5,679 | 208 | 37.6 |
| spray (bot) | 11,696 | 460 | 40.3 |

The bot moves the mouse at most twice per 120 Hz tick. A real 1000 Hz mouse moving
continuously produces up to 60,000 move records a minute. At about 40 bytes each, that
is roughly **2.3 MiB per minute (about 40 KiB/s) at the worst**. Real play moves the mouse
only part of the time. The task-1.4 hands-on check saw bursts of about 280 events per
second in slow movement under X11.

Nothing is compressed yet: records are postcard-encoded with no delta or entropy
coding. Compression is the obvious lever if bandwidth matters in a later phase.

## Not measured

- Real mouse input on real hardware. Everything here is the headless bot.
- Windows. CI runs Godot on Linux only; Windows Godot runs are task 3.4.
- A remote server. Everything here used loopback, which is the only mode until TLS and
  authentication (task 3.3).
