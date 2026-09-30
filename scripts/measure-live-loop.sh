#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Measures the aim range's frame-time cost with the input probe, the telemetry recorder
# and the server uplink on versus off, and the uplink's bandwidth per minute of play
# (task 1.7). Headless, scripted bot, `--fixed-fps 120` (frames run back to back, so a
# frame's wall time is its processing cost). Linux; run from the repository root:
#
#   scripts/measure-live-loop.sh /path/to/godot-4.7.2 [seconds-of-play] [repeats]
#
# Build first: cargo build -p rearguard-godot -p rearguard-server (for numbers that
# reflect an optimised library, build the extension with CARGO_PROFILE_DEV_OPT_LEVEL=3;
# demo/rearguard.gdextension loads target/debug either way). Output: one JSON line per
# run, then a summary table, under target/measure-live-loop/.
set -euo pipefail

godot=${1:?usage: $0 GODOT [seconds] [repeats]}
seconds=${2:-60}
repeats=${3:-3}
out="$PWD/target/measure-live-loop"
rm -rf "$out"
mkdir -p "$out"

# A server on a free loopback port, with a throwaway master secret and database.
head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$out/master.hex"
sed -e 's/127.0.0.1:7461/127.0.0.1:0/' -e 's/server-master.hex/master.hex/' -e 's/rearguard.sqlite3/evidence.sqlite3/' \
    crates/rearguard-server/config/server.example.json > "$out/server.json"
target/debug/rearguard-server run --config "$out/server.json" 2> "$out/server.log" &
server_pid=$!
trap 'kill $server_pid 2>/dev/null || true' EXIT
for _ in $(seq 100); do
    addr=$(sed -n 's/.*listening on //p' "$out/server.log" | head -1)
    [ -n "$addr" ] && break
    sleep 0.1
done
[ -n "$addr" ] || { echo "server did not start" >&2; exit 1; }

run() { # name scenario repeat args...
    local name=$1 scenario=$2 rep=$3
    shift 3
    "$godot" --headless --path demo --fixed-fps 120 -- --bot --scenario "$scenario" --seed "$rep" \
        --duration "$seconds" --out "$out/$name-$scenario-$rep.jsonl" \
        --frame-stats "$out/$name-$scenario-$rep.stats.json" "$@" > "$out/$name-$scenario-$rep.log" 2>&1
    printf '{"config":"%s","scenario":"%s","repeat":%s,"stats":%s}\n' "$name" "$scenario" "$rep" \
        "$(cat "$out/$name-$scenario-$rep.stats.json")" | tee -a "$out/runs.jsonl"
}

for scenario in flick spray; do
    for rep in $(seq "$repeats"); do
        run off "$scenario" "$rep" --no-probe --no-record
        run probe-recorder "$scenario" "$rep"
        run probe-recorder-uplink "$scenario" "$rep" --server "$addr"
    done
done

python3 - "$out/runs.jsonl" "$seconds" <<'PY'
import json, sys, statistics
runs = [json.loads(l) for l in open(sys.argv[1])]
minutes = float(sys.argv[2]) / 60
print("\n| Scenario | Configuration | p50 frame (us) | p99 frame (us) | Uplink KiB per minute of play | Records per minute |")
print("|---|---|---:|---:|---:|---:|")
for scenario in ("flick", "spray"):
    for config in ("off", "probe-recorder", "probe-recorder-uplink"):
        rs = [r["stats"] for r in runs if r["scenario"] == scenario and r["config"] == config]
        p50 = statistics.median(r["p50_us"] for r in rs)
        p99 = statistics.median(r["p99_us"] for r in rs)
        kib = statistics.median(r.get("bytes_sent", 0) for r in rs) / 1024 / minutes
        recs = statistics.median(r.get("records_queued", 0) for r in rs) / minutes
        up = f"{kib:.1f}" if config.endswith("uplink") else "-"
        rec = f"{recs:.0f}" if config.endswith("uplink") else "-"
        print(f"| {scenario} | {config} | {p50:.0f} | {p99:.0f} | {up} | {rec} |")
PY
