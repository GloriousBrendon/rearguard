# Drift study: facilitator instructions

One session per participant, about 25 minutes: consent, 6 one-minute rounds of plain
play, then 20 blind A/B comparisons of two 15-second rounds each. Protocol:
`demo/study/protocol.json` (drift amplitudes 0, 0.25, 0.5, 1 and 2 %).

## Before the study (once per machine)

1. **Build** from the repository root, following `demo/README.md`, "Running". Use the
   pinned Godot 4.7.2 and import the project first. Then run
   `CARGO_PROFILE_DEV_OPT_LEVEL=3 cargo build -p rearguard-godot -p rearguard-server`.
   The Godot binary loads the `target/debug` library; opt-level 3 makes it fast.
2. **Create the study key.** Do this once for the whole study, not per participant:
   `target/debug/rearguard-server gen-secret study-key.hex`.
   - Put it where the study reads it, the Godot user folder `study/study-key.hex`
     (for example `~/.local/share/godot/app_userdata/Rearguard Aim Range/study/` on
     Linux, `%APPDATA%\Godot\app_userdata\Rearguard Aim Range\study\` on Windows), or
     pass `--study-key PATH`.
   - Keep a copy somewhere safe: the analysis needs it to re-derive each session's
     drift. Never send it to participants and never put it in an export.
3. **Test run:** start the study (below), decline consent, and check that it quits
   with nothing written. Then run it once yourself.
4. **Machine setup:** a wired mouse, the OS pointer acceleration off, full screen,
   and no overlays or background recording.

## Running a session

```sh
target/godot/godot --path demo res://scenes/study.tscn
# optional, after `--`: --study-key PATH  --protocol PATH  --export-dir DIR
```

1. **Consent.** Let the participant read the consent screen alone and choose.
   - If they decline, the study quits and nothing is recorded.
   - Do not collect their name or contact details anywhere, including a sign-up
     sheet linked to their participant ID.
2. **Participant ID.** The next screen shows a random ID (`P-` plus 10 characters).
   The participant notes it down. You do not record who has which ID; the ID is the
   only way to find their data later (for example, to withdraw it).
3. **Baseline rounds.** The participant plays each round normally. They click in the
   game to start, and Esc pauses.
4. **Comparisons.** Each has round A, then round B, then a forced choice: "Which
   round felt different?"
   - Say only: "If you cannot tell, make your best guess."
   - Do not hint, and do not watch the screen during the choice.
   - You do not know which round is drifted either; nothing on screen shows it.
5. **End.** The last screen shows the export file,
   `rearguard-study-phase1-drift-<ID>.zip`, in the export folder (default: the
   Godot user folder `exports/`). The participant decides whether to send it to you.

## Rules

- Do not change the protocol file during a study: every participant must run the same
  one. The export embeds the protocol and the plan, so any change shows.
- Do not run the study with a debug overlay, a server, or any other tool attached.
- If a session is interrupted (crash, window closed), nothing is exported. Discard it
  and, if the participant agrees, start again. They get a new ID.
- If the same person takes part twice, both exports stand alone. You cannot link them,
  by design.

## What the export contains

The export is one zip file with two parts:

- **`manifest.json`:**
  - study id, protocol and consent version;
  - participant ID;
  - randomisation seed and the full plan;
  - Godot version, OS name, display driver and input settings;
  - per session: condition, amplitude, scenario and result;
  - per comparison: amplitude, drifted interval, order, answer, correct or not, and
    response time.
- **`sessions/<label>.jsonl`:** each round's telemetry (mouse deltas with game-clock
  timestamps, aim, clicks, targets). The format is described in `demo/README.md`,
  "Recording format".

It contains no names, no dates in the data, and no key or seed material. The plan can
be regenerated from the embedded protocol and randomisation seed (`rearguard_core::study::plan`).
