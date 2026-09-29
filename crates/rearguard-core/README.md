# rearguard-core

Engine-agnostic heart of Rearguard: probe definitions, the client/server telemetry
protocol types, and detection logic. Both the engine plugin and the server build on it.

**No Godot dependency.** This crate must never depend on Godot, gdext or any other
engine crate, directly or transitively. `unsafe` code is forbidden.
