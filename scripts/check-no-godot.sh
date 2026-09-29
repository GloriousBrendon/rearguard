#!/usr/bin/env bash
# Fails if rearguard-core, -sim or -server reach any Godot binding crate, directly or
# transitively, as a normal, build or dev dependency, on any target, with any features.
# Only rearguard-godot may depend on Godot. Run from the workspace root.
set -euo pipefail

crates=(rearguard-core rearguard-sim rearguard-server)
# gdext (godot, godot-*), GDExtension API crates, Godot 3 gdnative, and our own binding.
pattern='^(godot|godot-[A-Za-z0-9_-]+|gdext[A-Za-z0-9_-]*|gdextension[A-Za-z0-9_-]*|gdnative[A-Za-z0-9_-]*|rearguard-godot) v'

status=0
for crate in "${crates[@]}"; do
    tree=$(cargo tree --locked -p "$crate" --edges normal,build,dev \
        --all-features --target all --prefix none --format '{p}')
    if hits=$(grep -E "$pattern" <<<"$tree" | sed "s/ (\*)$//" | sort -u); then
        echo "error: $crate depends on Godot crates (only rearguard-godot may):"
        sed 's/^/  /' <<<"$hits"
        status=1
    else
        echo "ok: $crate has no Godot dependency"
    fi
done
exit "$status"
