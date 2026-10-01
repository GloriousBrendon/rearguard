#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Records the aim range's test bots (task 1.8) for the detector evaluation (task 1.12,
# `cargo xtask eval --bots`). Headless, `--fixed-fps 120`, against a throwaway
# rearguard-server on loopback, which issues every session's drift seed. Linux and
# Windows (Git Bash); run from the repository root:
#
#   scripts/record-test-bots.sh GODOT OUT [seconds-of-play] [runs] [amplitude-ppm]
#
#   GODOT          the Godot 4.7.2 binary; demo/ must be imported
#   OUT            a new or empty directory
#   seconds        length of every session (default 60; must equal the evaluation's horizon_s)
#   runs           sessions per bot and scenario (default 5)
#   amplitude-ppm  drift amplitude the server hands out (default 5000)
#
# Build first: cargo build -p rearguard-godot -p rearguard-server.
#
# Per run: the three flick cheats and the scripted player on flick, the recoil macro and
# the scripted player on spray. Output: OUT/recordings/NAME.jsonl with NAME.jsonl.env.json
# (which holds the ground-truth label), and OUT/master.hex, the server's master secret,
# which the evaluation needs to re-derive the seeds. The bots run only in Rearguard's own
# test environment (demo/scripts/test_cheats/README.md): this script sets
# REARGUARD_TEST_ENV=1 and uses a loopback server.
set -euo pipefail

godot=${1:?usage: $0 GODOT OUT [seconds] [runs] [amplitude-ppm]}
out=${2:?usage: $0 GODOT OUT [seconds] [runs] [amplitude-ppm]}
seconds=${3:-60}
runs=${4:-5}
amplitude=${5:-5000}

server=target/debug/rearguard-server
[ -x "$server" ] || server=target/debug/rearguard-server.exe
[ -x "$server" ] || { echo "record-test-bots: build first: cargo build -p rearguard-godot -p rearguard-server" >&2; exit 1; }
mkdir -p "$out"
[ -z "$(ls -A "$out")" ] || { echo "record-test-bots: $out is not empty" >&2; exit 1; }
out="$(cd "$out" && pwd)"
mkdir -p "$out/recordings" "$out/logs"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) native() { cygpath -w "$1"; } ;;
  *) native() { printf '%s' "$1"; } ;;
esac

# A server on a free loopback port, with its own master secret and database.
"$server" gen-secret "$out/master.hex"
sed -e 's/127.0.0.1:7461/127.0.0.1:0/' -e 's/server-master.hex/master.hex/' \
    -e "s/\"amplitude_ppm\": 5000/\"amplitude_ppm\": $amplitude/" \
    crates/rearguard-server/config/server.example.json > "$out/server.json"
"$server" run --config "$out/server.json" 2> "$out/logs/server.log" &
server_pid=$!
trap 'kill $server_pid 2>/dev/null || true' EXIT
addr=""
for _ in $(seq 100); do
    addr=$(sed -n 's/.*listening on //p' "$out/logs/server.log" | head -1 | tr -d '\r')
    [ -n "$addr" ] && break
    sleep 0.1
done
[ -n "$addr" ] || { echo "record-test-bots: the server did not start" >&2; exit 1; }

run() { # name scenario repeat args...
    local name=$1 scenario=$2 rep=$3
    shift 3
    local file="$out/recordings/$name-$scenario-$rep.jsonl"
    REARGUARD_TEST_ENV=1 "$godot" --headless --path demo --fixed-fps 120 -- "$@" --scenario "$scenario" \
        --seed "$rep" --bot-seed "$rep" --duration "$seconds" --server "$addr" \
        --out "$(native "$file")" > "$out/logs/$name-$scenario-$rep.log" 2>&1 \
        || { echo "record-test-bots: $name $scenario $rep failed; see $out/logs" >&2; exit 1; }
    [ -s "$file" ] && [ -s "$file.env.json" ] \
        || { echo "record-test-bots: $name $scenario $rep wrote no recording; see $out/logs" >&2; exit 1; }
    echo "recorded $name $scenario $rep"
}

for rep in $(seq "$runs"); do
    for cheat in flick-aimbot humanised-aimbot adaptive-aimbot; do
        run "$cheat" flick "$rep" --cheat "$cheat"
    done
    run scripted flick "$rep" --bot
    run recoil-macro spray "$rep" --cheat recoil-macro
    run scripted spray "$rep" --bot
done

echo "record-test-bots: $(find "$out/recordings" -name '*.jsonl' | wc -l | tr -d ' ') sessions in $out/recordings"
echo "evaluate with: cargo xtask eval --seed N --bots $out/recordings --bots-master-secret $out/master.hex"
