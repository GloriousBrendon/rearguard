# Drift study: facilitator instructions

One session per participant, about 20 minutes (16 minutes of play plus clicking between
rounds): consent, 6 one-minute rounds of plain play, then 20 blind A/B comparisons of
two 15-second rounds each. Protocol: `demo/study/protocol.json` (drift amplitudes 0,
0.25, 0.5, 1 and 2 %). There are no countdowns or timed pauses: every screen between
rounds waits for a click, and a round's clock runs only while the mouse is captured.

Volunteers run a **study build** on their own computer (Linux or Windows, x86_64). They
need no Godot, Rust or repository. Nothing leaves their machine automatically: at the
end the build writes one zip, and the participant decides whether to send it to you.

## 1. Make a release

A *release* is one pair of builds (Linux and Windows) plus the *study key* packed into
both. Every release has its own key and a label, which each export records. The label
is `YYYY-MM-letter-shorthash`: the month, a letter counting that month's releases, and
the 7-digit hash of the commit it was built from, for example `2026-10-a-06ec84a`. Use
one release for the whole study if you can; if you make a new one, keep every release's
key.

You build releases yourself, on your own machine, with a key that never leaves it. The
repository is public, and so is everything its CI produces, so **CI never makes a
release**: it uploads no key and no build that packs a real key.

### What you need

- The Godot 4.7.2 editor for Linux, as `godot`.
- The two release templates, `linux_release.x86_64` and `windows_release_x86_64.exe`,
  unpacked from `Godot_v4.7.2-stable_export_templates.tpz` into one folder (below,
  `~/godot-templates`). Check the `.tpz` against the release's `SHA512-SUMS.txt` first;
  the expected value is `GODOT_TEMPLATES_SHA512` in `.github/workflows/ci.yml`.
- The extension built in release mode for both platforms:
  `cargo build --release --locked -p rearguard-godot` on Linux, and the same on Windows,
  with the resulting `rearguard_godot.dll` copied to `target/release/` on the Linux
  machine. Build both from the same commit.

  Without a Windows machine, take the DLL from CI instead: the `godot tests (windows)`
  job uploads it as the artifact `rearguard-godot-windows-release` (it holds no key).
  Use the run for the commit you are releasing:

  ```sh
  gh run list --branch main --workflow ci.yml --limit 1     # the run's id and commit
  gh run download RUN_ID -n rearguard-godot-windows-release -D target/release
  ```

  The artifact **expires after one day**, so run CI shortly before cutting a release
  (`gh workflow run ci.yml --ref main`, or re-run the latest run on `main`).
- A folder outside the repository for the key and the builds (below, `~/study`), which
  only you can read: mode 700 for the folder, mode 600 for the key file.

### Build and test it

From a clean checkout of the commit you are releasing:

```sh
scripts/godot-import.sh godot                                      # before building the debug extension
cargo build --locked -p rearguard-godot -p rearguard-server        # the editor's extension, and the key tool
mkdir -p ~/study && chmod 700 ~/study
target/debug/rearguard-server gen-secret ~/study/study-key.hex     # a fresh key for this release; never overwrites
chmod 600 ~/study/study-key.hex                                    # already so on Linux; see below
scripts/export-study.sh --godot godot --templates ~/godot-templates \
  --study-key ~/study/study-key.hex --release my-label --out ~/study/builds --keep-build ~/study/unzipped
scripts/smoke-study-build.sh --build ~/study/unzipped/linux --godot godot \
  --key ~/study/study-key.hex --release my-label --keep-export ~/study/self-test
```

`gen-secret` prints one line, `rearguard-server: wrote a new master secret to PATH`, and
never the key itself. On Linux it creates the file with mode 600; on Windows the file
gets the folder's permissions. It fails rather than overwrite an existing file.

The smoke test and the check tool run the Godot **editor** on `demo/`, and the editor
loads the debug extension library, `target/debug/librearguard_godot.so`
(`demo/rearguard.gdextension`). The check replays every session with the extension's
probe, so without the library it stops with "the Rust extension is not loaded".
`scripts/godot-import.sh` deletes that library, which is why the `cargo build` comes
after the import; build it again after any later import.

`--keep-export` keeps the export that the build's `--self-test` run wrote. It is marked
as automated, so check it **without** `--human`:

```sh
godot --headless --path demo --script res://tools/check_study_export.gd -- \
  --zip ~/study/self-test/rearguard-study-build-smoke-P-XXXXXXXXXX.zip \
  --key ~/study/study-key.hex --release my-label
```

With `--human` the same export fails ("a participant's run (not automated)"), as it
should: `--human` is only for zips that participants send back (section 5).

`~/study/builds` then holds:

| File | Who gets it |
|------|-------------|
| `rearguard-study-<release>-linux-x86_64.zip` | Linux volunteers |
| `rearguard-study-<release>-windows-x86_64.zip` | Windows volunteers |
| `build-sizes.md` | Nobody; for reference |

To test the Windows build, copy `~/study/unzipped/windows` and the key to a Windows
machine with a checkout of the same commit, and in Git Bash run
`scripts/godot-import.sh`, copy the build's `rearguard_godot.dll` to `target/debug/`,
then run `scripts/smoke-study-build.sh` with the Windows editor (its `_console.exe`).
Delete the key from that machine afterwards.

### What CI does, and does not do

CI (`.github/workflows/ci.yml`) only checks that the export and the builds still work,
on every pull request and every push to `main`:

- `study build (export, linux smoke)` exports the Linux build with a throwaway key made
  inside the job, smoke-tests it, and deletes both. Neither is uploaded.
- It also exports a Windows build that packs the repository's public test key
  (`demo/tests/public-test-key.hex`), which `study build (windows smoke)` tests.

**Never send a CI build to a volunteer.** The one build CI uploads has a key anyone can
read, so its exports prove nothing, and it is deleted after a day. Release from a commit
where both jobs passed.

## 2. Keep the key

The study key is the root of every round's drift. The analysis needs it to re-derive
each session's drift and replay the telemetry, so:

- Store `study-key.hex` together with its release label, somewhere only you can read,
  for as long as the analysis and any withdrawal requests last.
- It is never committed (`.gitignore` blocks `demo/study/study-key.hex`), never uploaded
  to CI or any other shared place, and never sent to participants on its own. Exports
  never contain it.
- Keep it outside the repository folder, so that no `git add` can pick it up.

### What a participant could do with the key

The key is packed into both builds (`res://study/study-key.hex` inside the program's
pack), because the build derives each round's drift from it. It is not obfuscated
(the project rules forbid obfuscation), so **a determined participant can extract it**:
Godot pack files can be listed and unpacked with freely available tools, and the key is
64 hexadecimal digits in plain text. So can anyone else a build is passed on to.

With the key and this repository's open code, someone could:
- compute the drift signal of any round whose participant ID and label they know;
- produce made-up telemetry whose drift replays correctly with the key, so the replay
  check (`check_study_export.gd`) no longer shows that data was edited or invented.

It does **not**:
- reveal which round of a comparison is drifted, or at what amplitude. That comes from
  each session's plan, drawn from a fresh random seed when the participant agrees, and
  is never shown during the session. (Someone who modifies the build could still
  display it, key or not; that is a limit of any client-side study.)
- expose anything personal: the key is not personal data and identifies nobody.
- affect other releases, each of which has its own key, or Rearguard servers, whose
  master secrets are separate.
- let anyone read or change other participants' data. Exports stay on each
  participant's machine until they send them.

So a leaked key weakens only the integrity checks on data from that release. If you
suspect a leak, make a new release for the remaining participants and treat that
release's exports with care in the analysis.

## 3. Send a build to a volunteer

1. Ask which operating system they use: Windows or Linux, 64-bit PC (not Mac, not ARM).
2. Send them the matching zip, for example as a link to a private file share, and
   nothing else. Do not send the key or the other build.
3. Tell them:
   - unzip it anywhere and read `README.txt` (starting it, removing it). On Windows:
     right-click the zip, choose "Extract All", then open the extracted folder; the
     build does not run from inside the zip;
   - the build is **not code-signed**: Windows SmartScreen may warn ("More info", then
     "Run anyway"). On Linux they may need `chmod +x RearguardStudy.x86_64`;
   - use a wired mouse if they have one, play with their usual mouse settings and
     change nothing, and close overlays and screen recorders;
   - the study opens in a 1280x720 window. They must not resize or maximise it, so that
     everyone sees the same view;
   - set aside about 20 minutes without interruptions;
   - if they cannot tell which round felt different, they should guess;
   - send back the export zip exactly as the build wrote it: not renamed, not opened
     and saved again, nothing in it changed.
4. Do not ask for their name or contact details as part of the study, and do not keep a
   list linking people to participant IDs.

Mouse settings differ between participants, and so does what the build receives. On
native Wayland, Godot 4.7.2 delivers the compositor's **accelerated** movement, not raw
counts; on X11, XWayland and Windows it reads raw counts (`demo/README.md`, "Raw input
path"; the Windows and Wayland rows come from the engine source, not from a
measurement). Each export's manifest records the `display_driver`. The analysis must
not pool sessions from different display drivers without checking that they are
comparable.

## 4. What the participant sees

1. **Consent** (`demo/study/consent_v1.txt`, packed unchanged; CI checks the text on
   screen byte for byte). If they decline, the build quits and writes nothing.
2. **Participant ID:** a random ID (`P-` plus 10 characters). They note it down; it is
   the only way to find their data later, for example to withdraw it.
3. **Baseline rounds:** they click in the game to start each round; Esc pauses.
4. **Comparisons:** round A, round B, then "Which round felt different?". Nothing on
   screen shows which round is drifted.
5. **End:** the build shows the export file,
   `rearguard-study-phase1-drift-<ID>.zip`, in its user folder `exports/`
   (Windows `%APPDATA%\Godot\app_userdata\Rearguard Aim Range\exports\`, Linux
   `~/.local/share/godot/app_userdata/Rearguard Aim Range/exports/`), with a button to
   open that folder.

If a session is interrupted (crash, window closed), nothing is exported. They can start
again and get a new ID.

## 5. Collect the zip

1. The participant sends the zip by hand, in whatever way you agreed (e-mail attachment,
   file share upload). Nothing is uploaded automatically.
2. Store each zip unchanged, under the release it came from. The manifest's `release`
   field names the release; `participant_id` is the only identifier.
3. Check it against that release's key (with the editor, `demo/` imported and the
   extension built):

   ```sh
   godot --headless --path demo --script res://tools/check_study_export.gd -- \
     --zip rearguard-study-phase1-drift-P-XXXXXXXXXX.zip --key study-key.hex --release <label> --human
   ```

   It checks that only allowlisted fields are present, that the key is not in the
   export, that the export came from a release build and was not automated, and that
   every session replays with the drift re-derived from the key.
4. For a withdrawal request, delete the zip whose `participant_id` matches the ID quoted.

## Analysing the exports

`cargo xtask eval --seed N --humans FOLDER --study-key study-key.hex` runs every
participant's baseline sessions through the detector (`crates/xtask/README.md`). Give the
folder holding the zips, and the key of each release they came from. The results name no
participant: they record counts and a digest of the exports. After a withdrawal, rerun
the analysis without that participant's zip.

## What the export contains

The export is one zip file with two parts:

- **`manifest.json`:**
  - study id, protocol and consent version, release label, and whether the run was
    automated (only `--self-test` build checks are);
  - participant ID;
  - randomisation seed and the full plan;
  - Godot version, OS name, display driver, input settings, and whether the build was a
    debug build;
  - per session: condition, amplitude, scenario and result;
  - per comparison: amplitude, drifted interval, order, answer, correct or not, and
    response time.
- **`sessions/<label>.jsonl`:** each round's telemetry (mouse deltas with game-clock
  timestamps, aim, clicks, targets). The format is described in `demo/README.md`,
  "Recording format".

It contains no names, no dates in the data, and no key or seed material. The plan can be
regenerated from the embedded protocol and randomisation seed
(`rearguard_core::study::plan`).

## Rules

- Do not change the protocol during a study: every participant must run the same one.
  The build packs `demo/study/protocol.json`, and the export embeds the protocol and the
  plan, so any change shows.
- Participants run the build without a debug overlay, a server or other tools. The
  build has no developer overlay at all.
- If the same person takes part twice, both exports stand alone. You cannot link them,
  by design.

## Running from the repository (development)

The study also runs from the repository with the pinned Godot 4.7.2, for trying changes:

1. Build: `CARGO_PROFILE_DEV_OPT_LEVEL=3 cargo build -p rearguard-godot -p rearguard-server`
   after importing the project (`demo/README.md`, "Running").
2. Key: `target/debug/rearguard-server gen-secret study-key.hex`, placed in the Godot user
   folder as `study/study-key.hex`, or passed with `--study-key PATH`.
3. Run: `godot --path demo res://scenes/study.tscn` (options after `--`:
   `--study-key PATH`, `--protocol PATH`, `--export-dir DIR`). Exports from the
   repository have the release label `dev` and are marked as debug builds; do not use
   them as study data.
