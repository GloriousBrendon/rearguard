# Licence notes

Rearguard's own code is licensed `MIT OR Apache-2.0` at the user's option (decision D3);
the texts are `LICENSE-MIT` and `LICENSE-APACHE` at the repository root, and every
first-party source file carries an SPDX header (checked by `scripts/check-spdx.sh`).
Third-party code keeps its own licence: Rearguard's files can be MIT OR Apache-2.0 while
depending on gdext (MPL-2.0, below), whose files are neither relicensed nor given our header.
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

## Study builds: Godot export templates (task 1.9a)

The exported study builds (`scripts/export-study.sh`) contain the official Godot
4.7.2-stable **release export templates** (`linux_release.x86_64`,
`windows_release_x86_64.exe` from `Godot_v4.7.2-stable_export_templates.tpz`, checked
against the release's SHA-512). These are distributed binaries, not crates, so
`cargo deny` does not see them. Their licences, from the engine's own copyright data
(`Engine.get_copyright_info()`, which Godot builds from its `COPYRIGHT.txt`):

- **Godot Engine:** MIT (Expat).
- **Third-party components compiled in:** mostly MIT/Expat, BSD-2-Clause,
  BSD-3-Clause, Apache-2.0, Zlib, Unlicense, MIT-0, CC0-1.0, BSL-1.0 (Clipper2),
  Unicode (ICU), X11, IJG (libjpeg-turbo), and the HarfBuzz and glslang licences
  (both permissive).
- **Worth noting:**
  - **FreeType:** used under the FreeType Licence (FTL), a BSD-style licence with an
    advertising-credit clause (FreeType is dual FTL/GPL-2.0; Godot uses FTL).
  - **Fonts** (Inter, JetBrains Mono, Noto Sans, Open Sans, Vazirmatn): SIL OFL-1.1.
  - **Godot logo:** CC-BY-4.0.
  - **CA certificates** (Mozilla's root store, as data): **MPL-2.0**, file-level weak
    copyleft on unmodified data. The builds never use it (they make no network
    connections), but it is compiled into the templates. This is outside the `deny.toml`
    policy, which covers crates only, and has no recorded decision yet (see the open
    question in the task 1.9a report).

Every build ships Rearguard's own licence texts (`LICENSE-MIT`, `LICENSE-APACHE`) and
`THIRD-PARTY-NOTICES.txt`:
- the engine's licence text, per-component copyright notices and every licence text,
  from the Godot binary (`demo/tools/godot_notices.gd`);
- the name, version, licence and shipped licence files of every crate compiled into the
  extension (`scripts/study-export/rust_notices.py`), with a pointer to gdext's source
  for MPL-2.0.
