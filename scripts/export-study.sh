#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Exports the human-study build (task 1.9a) for Linux and Windows x86_64.
#
#   scripts/export-study.sh --godot BIN --templates DIR --study-key FILE --release LABEL --out DIR
#
#   --godot BIN        the Godot 4.7.2 editor binary (Linux)
#   --templates DIR    holds linux_release.x86_64 and windows_release_x86_64.exe from the
#                      official 4.7.2-stable export templates (checked against SHA-512)
#   --study-key FILE   this release's study key (64 hex digits, rearguard-server gen-secret)
#   --release LABEL    this release's label, written into every export manifest
#   --out DIR          where the two zips go
#   --keep-build DIR   optional: also copy the unzipped builds to DIR/linux and DIR/windows
#   --only PLATFORM    optional: export only `linux` or only `windows` (CI's smoke builds)
#
# The Rust extension must already be built in release mode for both platforms:
# target/release/librearguard_godot.so and target/release/rearguard_godot.dll (with
# --only, just that platform's library and template).
#
# The export runs from a staging copy of demo/ in a temporary directory, so the key never
# enters the working tree. The staging copy:
# - packs the study key (res://study/study-key.hex) and the release label
#   (res://study/release.txt);
# - drops the developer overlay, the tests and the tools;
# - makes the study the main scene;
# - turns off Godot's log file and shader cache, so the build writes nothing to disk
#   before the participant agrees.
# The staging directory, and the key copy in it, is deleted on exit.
#
# Output: rearguard-study-<release>-linux-x86_64.zip and ...-windows-x86_64.zip, each with
# the program, the extension library, README.txt, THIRD-PARTY-NOTICES.txt and Rearguard's
# licence texts (LICENSE-MIT, LICENSE-APACHE). Sizes are
# printed (and added to the GitHub step summary in CI).
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
# Exports the human-study build (task 1.9a) for Linux and Windows x86_64.
#
#   scripts/export-study.sh --godot BIN --templates DIR --study-key FILE --release LABEL --out DIR
#
#   --godot BIN        the Godot 4.7.2 editor binary (Linux)
#   --templates DIR    holds linux_release.x86_64 and windows_release_x86_64.exe from the
#                      official 4.7.2-stable export templates (checked against SHA-512)
#   --study-key FILE   this release's study key (64 hex digits, rearguard-server gen-secret)
#   --release LABEL    this release's label, written into every export manifest
#   --out DIR          where the two zips go
#   --keep-build DIR   optional: also copy the unzipped builds to DIR/linux and DIR/windows
#   --only PLATFORM    optional: export only `linux` or only `windows` (CI's smoke builds)
#
# The Rust extension must already be built in release mode for both platforms:
# target/release/librearguard_godot.so and target/release/rearguard_godot.dll (with
# --only, just that platform's library and template).
USAGE
  exit 2
}

godot="" templates="" key="" release="" out="" keep="" only=""
while [ $# -gt 0 ]; do
  case "$1" in
    --godot) godot="$2"; shift 2 ;;
    --templates) templates="$2"; shift 2 ;;
    --study-key) key="$2"; shift 2 ;;
    --release) release="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --keep-build) keep="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$godot" ] && [ -n "$templates" ] && [ -n "$key" ] && [ -n "$release" ] && [ -n "$out" ] || usage
case "$only" in
  "") platforms=(linux windows) ;;
  linux|windows) platforms=("$only") ;;
  *) usage ;;
esac
has() { local p; for p in "${platforms[@]}"; do [ "$p" = "$1" ] && return 0; done; return 1; }

repo="$(cd "$(dirname "$0")/.." && pwd)"
templates="$(cd "$templates" && pwd)"
so="$repo/target/release/librearguard_godot.so"
dll="$repo/target/release/rearguard_godot.dll"
needed=("$godot" "$key")
if has linux; then needed+=("$templates/linux_release.x86_64" "$so"); fi
if has windows; then needed+=("$templates/windows_release_x86_64.exe" "$dll"); fi
for f in "${needed[@]}"; do
  [ -f "$f" ] || { echo "export-study: missing $f" >&2; exit 1; }
done
# Check the key's shape without ever printing it.
if ! tr -d ' \r\n\t' < "$key" | grep -Eq '^[0-9A-Fa-f]{64}$'; then
  echo "export-study: $key is not 64 hex digits" >&2
  exit 1
fi
if ! [[ "$release" =~ ^[A-Za-z0-9._-]{1,64}$ ]]; then
  echo "export-study: release label must be 1-64 of A-Z a-z 0-9 . _ -" >&2
  exit 1
fi

mkdir -p "$out"
out="$(cd "$out" && pwd)"
umask 077
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

# 1. Staging copy without the import cache, the overlay, tests, tools and test cheats.
cp -R "$repo/demo" "$stage/demo"
proj="$stage/demo"
rm -rf "$proj/.godot" "$proj/tests" "$proj/tools" "$proj/README.md" "$proj/scripts/test_cheats"
rm -f "$proj"/scripts/*_test.gd "$proj"/scripts/*_test.gd.uid "$proj"/scripts/dev_overlay.gd "$proj"/scripts/dev_overlay.gd.uid

# 2. Study build settings.
sed -i.bak 's|^run/main_scene=.*|run/main_scene="res://scenes/study.tscn"|' "$proj/project.godot"
sed -i.bak 's|^\[rendering\]$|[rendering]\n\nshader_compiler/shader_cache/enabled=false|' "$proj/project.godot"
printf '\n[debug]\n\nfile_logging/enable_file_logging.pc=false\n' >> "$proj/project.godot"
rm -f "$proj/project.godot.bak"
grep -q 'run/main_scene="res://scenes/study.tscn"' "$proj/project.godot"
grep -q 'shader_cache/enabled=false' "$proj/project.godot"

# 3. Import first, while the .gdextension still points at ../target (missing here): with
#    the extension present, Godot 4.7.2's first import crashes on shutdown
#    (crates/rearguard-godot/README.md). Godot logs one load error here; that is expected.
timeout 300 "$godot" --headless --path "$proj" --import > "$stage/import.log" 2>&1 || {
  cat "$stage/import.log" >&2; echo "export-study: import failed" >&2; exit 1; }

# 4. Point the extension at the release libraries, then pack the key and the label.
cat > "$proj/rearguard.gdextension" <<EOF
[configuration]
entry_symbol = "gdext_rust_init"
compatibility_minimum = "4.7"
reloadable = false

[libraries]
linux.debug.x86_64 = "$so"
linux.release.x86_64 = "$so"
windows.debug.x86_64 = "$dll"
windows.release.x86_64 = "$dll"
EOF
tr -d ' \r\n\t' < "$key" > "$proj/study/study-key.hex"
printf '\n' >> "$proj/study/study-key.hex"
printf '%s\n' "$release" > "$proj/study/release.txt"
sed -e "s|@LINUX_TEMPLATE@|$templates/linux_release.x86_64|" \
    -e "s|@WINDOWS_TEMPLATE@|$templates/windows_release_x86_64.exe|" \
    "$repo/scripts/study-export/export_presets.cfg" > "$proj/export_presets.cfg"

# 5. Export each platform asked for.
export_one() { # preset, output file
  mkdir -p "$(dirname "$2")"
  timeout 600 "$godot" --headless --path "$proj" --export-release "$1" "$2" > "$stage/export-$1.log" 2>&1 || true
  if [ ! -f "$2" ]; then
    cat "$stage/export-$1.log" >&2
    echo "export-study: $1 export failed" >&2
    exit 1
  fi
}
if has linux; then
  export_one Linux "$stage/build/linux/RearguardStudy.x86_64"
  [ -f "$stage/build/linux/librearguard_godot.so" ] || { echo "export-study: library missing from the Linux export" >&2; exit 1; }
  chmod 755 "$stage/build/linux/RearguardStudy.x86_64"
fi
if has windows; then
  export_one Windows "$stage/build/windows/RearguardStudy.exe"
  [ -f "$stage/build/windows/rearguard_godot.dll" ] || { echo "export-study: library missing from the Windows export" >&2; exit 1; }
fi

# 6. Licence notices: the engine's and the Rust crates compiled into the extension.
#    Export templates cannot run scripts (official builds disable --script), so the
#    editor binary writes the engine's notices; Godot builds them from its full
#    COPYRIGHT.txt, identical in the editor and the templates of the same release.
notices="$stage/THIRD-PARTY-NOTICES.txt"
timeout 120 "$godot" --headless --path "$proj" --script "$repo/demo/tools/godot_notices.gd" -- \
  --out "$stage/godot-notices.txt" > "$stage/notices.log" 2>&1 || true
[ -s "$stage/godot-notices.txt" ] || { cat "$stage/notices.log" >&2; echo "export-study: no Godot notices" >&2; exit 1; }
python3 "$repo/scripts/study-export/rust_notices.py" "$repo" > "$stage/rust-notices.txt"
{
  echo "Third-party software in the Rearguard study build ($release)"
  echo
  echo "Rearguard itself: MIT OR Apache-2.0, at your option (LICENSE-MIT, LICENSE-APACHE)."
  echo
  echo "The build contains the Godot Engine release export template (MIT, with the"
  echo "third-party components listed below) and the Rearguard extension library, built"
  echo "from Rust crates listed below. The gdext crates (godot, godot-*, gdextension-api)"
  echo "are MPL-2.0; their unmodified source is at https://github.com/godot-rust/gdext"
  echo "(version 0.5.5) and on crates.io."
  echo
  echo "================================================================================"
  echo "PART 1: Rust crates in the extension library"
  echo "================================================================================"
  cat "$stage/rust-notices.txt"
  echo
  echo "================================================================================"
  echo "PART 2: Godot Engine"
  echo "================================================================================"
  cat "$stage/godot-notices.txt"
} > "$notices"

# 7. Zip each build with the participant README, the notices and Rearguard's licence.
sed "s|@RELEASE@|$release|" "$repo/scripts/study-export/README.txt" > "$stage/README.txt"
for p in "${platforms[@]}"; do
  cp "$stage/README.txt" "$notices" "$repo/LICENSE-MIT" "$repo/LICENSE-APACHE" "$stage/build/$p/"
  python3 - "$stage/build/$p" "$out/rearguard-study-$release-$p-x86_64.zip" "rearguard-study-$release-$p" <<'EOF'
import os, sys, zipfile
src, dest, top = sys.argv[1:4]
with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for name in sorted(os.listdir(src)):
        path = os.path.join(src, name)
        info = zipfile.ZipInfo.from_file(path, f"{top}/{name}")
        info.compress_type = zipfile.ZIP_DEFLATED
        with open(path, "rb") as f:
            z.writestr(info, f.read(), compresslevel=9)
EOF
done
# The kept builds are exactly what the zips hold.
if [ -n "$keep" ]; then
  mkdir -p "$keep"
  for p in "${platforms[@]}"; do cp -R "$stage/build/$p" "$keep/"; done
fi

# 8. Sizes.
report="$out/build-sizes.md"
{
  echo "| Build | File | Size (bytes) |"
  echo "|-------|------|--------------|"
  for p in "${platforms[@]}"; do
    for f in "$stage/build/$p"/*; do
      echo "| $p | $(basename "$f") | $(wc -c < "$f" | tr -d ' ') |"
    done
    z="$out/rearguard-study-$release-$p-x86_64.zip"
    echo "| $p | **$(basename "$z")** (download) | **$(wc -c < "$z" | tr -d ' ')** |"
  done
} > "$report"
cat "$report"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  { echo "### Study build sizes ($release)"; echo; cat "$report"; } >> "$GITHUB_STEP_SUMMARY"
fi
for p in "${platforms[@]}"; do echo "export-study: wrote $out/rearguard-study-$release-$p-x86_64.zip"; done
