# Licence notes

Rearguard's own licence is not chosen yet (decision D3; the leading option is
Apache-2.0). Until then the repository is "All rights reserved, licence pending".
Dependencies must be permissive; `deny.toml` holds the allowlist, and CI enforces it
with `cargo deny --locked check`.

## Exception: gdext (MPL-2.0), decision D10

The Godot binding (`crates/rearguard-godot`) uses gdext, the Rust bindings for Godot 4,
which are licensed MPL-2.0. MPL-2.0 is weak (file-level) copyleft: it applies to gdext's
own files and to modifications of them, not to code that merely uses them.

Decision D10 accepts a narrow exception:

- **Which crates:** exactly the gdext crates, at the pinned versions:
  - `godot`, `godot-bindings`, `godot-cell`, `godot-codegen`, `godot-core`, `godot-ffi`
    and `godot-macros` at 0.5.5;
  - `gdextension-api` at 0.5.1.

  Each is listed in `deny.toml` under `[licenses] exceptions`. A gdext bump has to update
  those entries, so it cannot slip in unreviewed.
- **Where:** only `rearguard-godot` may reach them. `scripts/check-no-godot.sh` (in CI)
  fails if core, sim or server depend on any Godot crate, directly or transitively.
- **How:** gdext's files are used unmodified, from crates.io. If a change to gdext is
  ever needed, the changed files are published under MPL-2.0 as the licence requires.
- **Distribution:** anyone shipping a build of the extension must make gdext's source
  (the published crates) available under MPL-2.0 and keep its notices. Our own files
  are not affected.

No other licence exceptions exist.
