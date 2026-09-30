#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Headless smoke test of an exported study build (task 1.9a), on Linux or Windows (Git Bash).
#
#   scripts/smoke-study-build.sh --build DIR --godot EDITOR --key KEY.hex --release LABEL
#
#   --build DIR      the unzipped build: RearguardStudy.x86_64 or RearguardStudy.exe, with
#                    its extension library
#   --godot EDITOR   the Godot 4.7.2 editor binary, for the export check. demo/ must be
#                    imported and the Rust extension loadable by the editor
#   --key KEY.hex    the facilitator's copy of the release's study key
#   --release LABEL  the release label the build was exported with
#
# Every run of the build gets fresh, empty user folders (HOME and XDG_* on Linux, APPDATA
# and LOCALAPPDATA on Windows) in a temporary sandbox.
#
# 1. Decline: `--smoke-decline` shows the consent screen, then presses "I do not agree"
#    and "Quit". The test checks that the build launched and exited 0, that the consent
#    text on screen is byte for byte demo/study/consent_v1.txt, that the build packs the
#    key but not the developer overlay, tests or tools, and that no file was written
#    anywhere in the sandbox or in the build folder.
#    It also checks that the build folder holds Rearguard's licence texts, identical to
#    LICENSE-MIT and LICENSE-APACHE at the repository root.
# 2. Export: `--self-test` runs the study with the scripted bot on a short protocol and
#    writes an export, which tools/check_study_export.gd checks with the same
#    no-personal-data checks as task 1.9, plus a replay of every session with KEY.hex.
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
# Headless smoke test of an exported study build (task 1.9a), on Linux or Windows (Git Bash).
#
#   scripts/smoke-study-build.sh --build DIR --godot EDITOR --key KEY.hex --release LABEL
#
#   --build DIR      the unzipped build: RearguardStudy.x86_64 or RearguardStudy.exe, with
#                    its extension library
#   --godot EDITOR   the Godot 4.7.2 editor binary, for the export check. demo/ must be
#                    imported and the Rust extension loadable by the editor
#   --key KEY.hex    the facilitator's copy of the release's study key
#   --release LABEL  the release label the build was exported with
USAGE
  exit 2
}

build="" godot="" key="" release=""
while [ $# -gt 0 ]; do
  case "$1" in
    --build) build="$2"; shift 2 ;;
    --godot) godot="$2"; shift 2 ;;
    --key) key="$2"; shift 2 ;;
    --release) release="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$build" ] && [ -n "$godot" ] && [ -n "$key" ] && [ -n "$release" ] || usage

repo="$(cd "$(dirname "$0")/.." && pwd)"
build="$(cd "$build" && pwd)"
key="$(cd "$(dirname "$key")" && pwd)/$(basename "$key")"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) windows=1; bin="$build/RearguardStudy.exe" ;;
  *) windows=0; bin="$build/RearguardStudy.x86_64" ;;
esac
[ -f "$bin" ] || { echo "smoke: no $bin" >&2; exit 1; }
native() { if [ "$windows" = 1 ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

fail=0
ok() { echo "ok    $*"; }
bad() { echo "FAIL  $*"; fail=1; }
real_home="${HOME:-}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Runs the build with an empty sandbox for its user folders and working directory.
# Usage: run_build SANDBOX LOG ARGS...
run_build() {
  local sandbox="$1" log="$2"; shift 2
  mkdir -p "$sandbox/home" "$sandbox/cwd"
  (
    cd "$sandbox/cwd"
    if [ "$windows" = 1 ]; then
      APPDATA="$(native "$sandbox/home/appdata")" LOCALAPPDATA="$(native "$sandbox/home/localappdata")" \
        TEMP="$(native "$sandbox/home/temp")" TMP="$(native "$sandbox/home/temp")" \
        timeout 600 "$bin" "$@"
    else
      HOME="$sandbox/home" XDG_DATA_HOME="$sandbox/home/.local/share" \
        XDG_CONFIG_HOME="$sandbox/home/.config" XDG_CACHE_HOME="$sandbox/home/.cache" \
        timeout 600 "$bin" "$@"
    fi
  ) > "$log" 2>&1
}

# --- 1. Declining consent -------------------------------------------------------------
before="$(cd "$build" && find . -type f | sort)"
code=0
run_build "$work/decline" "$work/decline.log" --headless -- --smoke-decline || code=$?
sed 's/^/        | /' "$work/decline.log"
[ "$code" = 0 ] && ok "launched and exited 0 after declining" || bad "exit code $code after declining"

consent_hash="$(tr -d '\r' < "$repo/demo/study/consent_v1.txt" | sha256sum | cut -d' ' -f1)"
grep -q "study: smoke: consent sha256 $consent_hash" "$work/decline.log" \
  && ok "consent screen shows demo/study/consent_v1.txt unchanged" \
  || bad "consent text on screen differs from demo/study/consent_v1.txt (expected sha256 $consent_hash)"
grep -q "study: smoke: release $release, debug build false" "$work/decline.log" \
  && ok "release build, label $release" || bad "release label or build type"
grep -q "study: smoke: packed: key true, dev overlay false, tests false, tools false, test cheats false" "$work/decline.log" \
  && ok "packs the study key; no developer overlay, tests, tools or test cheats" || bad "build contents"

written="$(find "$work/decline" -type f | sed "s|^$work/decline/||")"
if [ -z "$written" ]; then ok "no file written in the user folders or working directory"
else bad "files written after declining:"; echo "$written" | sed 's/^/        /'; fi
after="$(cd "$build" && find . -type f | sort)"
[ "$before" = "$after" ] && ok "no file written in the build folder" || bad "the build folder changed"

for f in LICENSE-MIT LICENSE-APACHE; do
  cmp -s "$build/$f" "$repo/$f" && ok "ships $f unchanged" || bad "$f missing from the build or not the repository's"
done

# --- 2. An automated export, checked like task 1.9's ----------------------------------
cat > "$work/short-protocol.json" <<'EOF'
{
  "protocol_version": 1,
  "study_id": "build-smoke",
  "baseline": [
    { "scenario": "flick", "amplitude_ppm": 0, "duration_s": 2.0, "repeats": 1 },
    { "scenario": "spray", "amplitude_ppm": 20000, "duration_s": 2.0, "repeats": 1 }
  ],
  "blind": { "scenario": "flick", "interval_duration_s": 1.5, "amplitudes_ppm": [0, 20000], "repeats": 1 }
}
EOF
mkdir -p "$work/export/exports"
code=0
run_build "$work/export" "$work/export.log" --headless --fixed-fps 120 -- --self-test \
  --protocol "$(native "$work/short-protocol.json")" --export-dir "$(native "$work/export/exports")" || code=$?
tail -n 5 "$work/export.log" | sed 's/^/        | /'
zip="$(find "$work/export/exports" -name 'rearguard-study-build-smoke-*.zip' | head -n 1)"
[ "$code" = 0 ] || bad "self-test exit code $code"
if [ -z "$zip" ]; then
  bad "self-test wrote no export"
else
  ok "self-test export $(basename "$zip") ($(wc -c < "$zip" | tr -d ' ') bytes)"
  # The checker runs with the same user folders as the build, so its forbidden strings
  # (home, user data folder) are the ones the build saw; the real home and the build
  # folder are added.
  code=0
  if [ "$windows" = 1 ]; then
    APPDATA="$(native "$work/export/home/appdata")" LOCALAPPDATA="$(native "$work/export/home/localappdata")" \
      timeout 300 "$godot" --headless --path "$(native "$repo/demo")" --script res://tools/check_study_export.gd -- \
      --zip "$(native "$zip")" --key "$(native "$key")" --release "$release" \
      --forbid "$(native "$build")" --forbid "${USERPROFILE:-$build}" > "$work/check.log" 2>&1 || code=$?
  else
    HOME="$work/export/home" XDG_DATA_HOME="$work/export/home/.local/share" \
      XDG_CONFIG_HOME="$work/export/home/.config" XDG_CACHE_HOME="$work/export/home/.cache" \
      timeout 300 "$godot" --headless --path "$repo/demo" --script res://tools/check_study_export.gd -- \
      --zip "$zip" --key "$key" --release "$release" \
      --forbid "$build" --forbid "$real_home" > "$work/check.log" 2>&1 || code=$?
  fi
  grep -E '^(ok|FAIL) |check_study_export|ERROR|SCRIPT ERROR' "$work/check.log" | sed 's/^/        | /'
  [ "$code" = 0 ] && ok "export passes the task 1.9 checks" || bad "export check failed (exit $code)"
fi

echo
if [ "$fail" = 0 ]; then echo "smoke-study-build: ok ($bin)"; else echo "smoke-study-build: FAILED ($bin)"; fi
exit "$fail"
