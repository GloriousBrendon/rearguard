# rearguard-server

Server module: holds the probe seeds, receives client telemetry (treated as
untrusted), and decides who reacted to probes. Seeds and derived secrets never
leave the server beyond what the protocol requires and are never logged.

**No Godot dependency.** This crate must never depend on Godot, gdext or any other
engine crate, directly or transitively. `unsafe` code is forbidden.
