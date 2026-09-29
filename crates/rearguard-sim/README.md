# rearguard-sim

Rearguard's own closed test environment: synthetic players, test cheats and the
harness that measures detection rate and false-positive rate together. Test cheats
live here and run only inside this environment, never against live servers or
other people's games.

**No Godot dependency.** This crate must never depend on Godot, gdext or any other
engine crate, directly or transitively. `unsafe` code is forbidden.
