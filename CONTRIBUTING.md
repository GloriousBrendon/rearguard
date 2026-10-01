# Contributing

Rearguard is an early prototype; read "Status and limits" in [README.md](README.md)
first. [CLAUDE.md](CLAUDE.md) holds the project's decisions, constraints and crate map.

Bypasses and vulnerabilities go through [SECURITY.md](SECURITY.md), not public issues.

## Checks

Work on a branch and open a pull request. CI runs these on Linux and Windows; run them
before you push:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny --locked check        # cargo install cargo-deny --locked (CI: 0.20.2)
scripts/check-no-godot.sh        # no Godot crate outside rearguard-godot
scripts/check-spdx.sh            # SPDX header in every first-party source file
```

The toolchain is pinned in `rust-toolchain.toml`. Changes under `demo/` also need the
Godot tests; [demo/README.md](demo/README.md) explains how to run them.

## Rules that the checks enforce

- Every first-party source file starts with `SPDX-License-Identifier: MIT OR Apache-2.0`.
- Dependencies are permissive-only (`deny.toml`). A new dependency must pass
  `cargo deny --locked check` and be added to the dependency table in `CLAUDE.md`.
  `deny.toml` is not weakened to make a crate pass.
- Only `rearguard-godot` may depend on Godot crates. Core, sim, server and xtask forbid
  `unsafe` code.

## Rules that they do not

- No kernel drivers, no inspection of other processes, no obfuscation or anti-debug
  tricks. Behaviour is the same on Linux and Windows.
- Seeds and derived secrets are never logged, printed or committed.
- Test cheats run only inside Rearguard's own test environment. Nothing is aimed at live
  servers or other people's games.
- A detection rate is always reported with its false-positive rate.

## Licence

Rearguard is licensed `MIT OR Apache-2.0`. Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in this work by you, as defined in
the Apache-2.0 license, shall be dual licensed as above, without any additional terms or
conditions.
