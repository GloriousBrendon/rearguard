# rearguard

A zero-access anti-cheat: no kernel driver, and the same behaviour on Linux and Windows.

An open-source, zero-access anti-cheat SDK (engine plugin plus server module).
See [CLAUDE.md](CLAUDE.md) for project context, constraints and the crate map.

## Status and limits

Rearguard is an early prototype of an open-source anti-cheat that needs no kernel driver
and behaves the same on Linux and Windows. The server plants small, secret, per-match
signals in the game (so far in mouse sensitivity and recoil) and watches who reacts to
them.

**What exists:** the first layer, for input cheats (macros and aimbots), built in Rust
and Godot 4, with a server, a detector, simulated and in-game test cheats, and a human
study build. The server stores each session's detector evidence, and it listens on
loopback only: it has no TLS or authentication yet.

**What is not built yet:** the layers for information cheats (decoys, split truth,
server-side culling) and for cheats that only read the screen, any review or punishment
pipeline acting on the stored evidence, and hardware identity.

**What the results mean:** every detection number in this repository comes from
simulated players and simulated cheats. No real player's data has been through the
detector yet, and nothing has been tested in a real game. False-positive rates in
particular cannot be claimed until that happens.

**Limits:** nothing here is unbreakable. A cheat that reads the secret signal from the
player's own memory, or that corrects its own aim continuously, may evade parts of it.
Even in simulation, at the default signal size (0.5%) and over a one-minute session, the
humanised aimbot is flagged 11% of the time and the recoil macro 15%, while the flick,
adaptive and fast adaptive aimbots are flagged every time
([docs/eval-1.12.md](docs/eval-1.12.md)). At the same signal size, over 15 minutes
of spraying, the recoil macro reaches 99.8% when scored once at the end and 84.8% when
the detector monitors continuously, as a live server would
([docs/detector-1.3a.md](docs/detector-1.3a.md)). All of these figures are simulated.
The aim is to cover all kinds of cheats, but today only the input layer exists.

**How to help:** try to break it, run the study build (no builds are published; you
make one as [docs/study/facilitator-instructions.md](docs/study/facilitator-instructions.md)
describes), or wire it into a game of your own (on one machine for now, because the
server is loopback only). See
[CONTRIBUTING.md](CONTRIBUTING.md); report bypasses and vulnerabilities privately, as
[SECURITY.md](SECURITY.md) describes.

Rearguard is not affiliated with the Re:Guard anti-cheat research project.

## Build

```sh
cargo build --workspace
cargo test --workspace
```

## Demo

`demo/` is the Godot 4.7.2 aim range used for the Phase 1 measurements. See
[demo/README.md](demo/README.md) for how to run it, its tests and the recording format.

## Licence

Licensed under either of Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or
MIT license ([LICENSE-MIT](LICENSE-MIT)) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
