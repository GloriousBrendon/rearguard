#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Imports demo/ headless and fails if Godot printed an error. Linux or Windows (Git Bash).
#
#   scripts/godot-import.sh GODOT
#
#   GODOT   the Godot 4.7.2 editor binary
#
# The import must run without the Rust extension: with a GDExtension class registered,
# Godot 4.7.2's first headless import crashes while shutting down (see
# crates/rearguard-godot/README.md). So the debug library is removed first (build it
# again afterwards), and Godot reports that it cannot open it. Those messages are the
# only errors allowed; any other line with `ERROR:` (which covers `SCRIPT ERROR:` and
# `USER ERROR:`) fails the import, as does a non-zero exit code or a timeout.
set -euo pipefail

[ $# -eq 1 ] || { sed -n '3,7p' "$0" >&2; exit 2; }
godot="$1"
repo="$(cd "$(dirname "$0")/.." && pwd)"

rm -f "$repo/target/debug/librearguard_godot.so" "$repo/target/debug/rearguard_godot.dll"
log="$(mktemp)"
trap 'rm -f "$log"' EXIT
code=0
timeout 300 "$godot" --headless --path "$repo/demo" --import > "$log" 2>&1 || code=$?
cat "$log"

# The missing-library messages, on Linux and on Windows.
allowed="ERROR: Can't open dynamic library, file not found: '.*rearguard_godot\.(so|dll)'\.\$"
allowed="$allowed|ERROR: Condition \"!FileAccess::exists\(path\)\" is true\. Returning: ERR_FILE_NOT_FOUND\$"
allowed="$allowed|ERROR: GDExtension dynamic library not found: 'res://rearguard\.gdextension'\.\$"
errors="$(tr -d '\r' < "$log" | sed 's/\x1b\[[0-9;]*m//g' | grep 'ERROR:' | grep -Ev "$allowed" || true)"

echo
if [ "$code" != 0 ]; then
  echo "godot-import: FAILED (exit code $code)"
  exit 1
fi
if [ -n "$errors" ]; then
  echo "godot-import: FAILED, the import printed errors:"
  printf '%s\n' "$errors" | sed 's/^/    /'
  exit 1
fi
echo "godot-import: ok, no errors besides the missing extension library"
