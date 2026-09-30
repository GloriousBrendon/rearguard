#!/usr/bin/env python3
"""Prints the licence notices of every crate compiled into the Rearguard Godot extension.

Usage: rust_notices.py REPO_ROOT

Walks the normal (non-dev, non-build) dependency graph of rearguard-godot from
`cargo metadata --locked`, for all targets, and prints each crate's name, version and
licence expression, followed by the licence and notice files the crate ships. Workspace
crates are listed without files (Rearguard: all rights reserved, licence pending).
"""

import json
import os
import subprocess
import sys

NOTICE_PREFIXES = ("license", "licence", "copying", "notice", "copyright")


def main() -> None:
    repo = sys.argv[1]
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=repo,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    workspace = set(meta["workspace_members"])
    root = next(i for i in workspace if packages[i]["name"] == "rearguard-godot")

    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])

    ordered = sorted(seen, key=lambda i: (packages[i]["name"], packages[i]["version"]))
    print("Crates (name, version, licence):\n")
    for pid in ordered:
        p = packages[pid]
        lic = "Rearguard workspace crate" if pid in workspace else (p.get("license") or "see files")
        print(f"  {p['name']} {p['version']}: {lic}")
    for pid in ordered:
        if pid in workspace:
            continue
        p = packages[pid]
        crate_dir = os.path.dirname(p["manifest_path"])
        files = sorted(
            f
            for f in os.listdir(crate_dir)
            if f.lower().startswith(NOTICE_PREFIXES) and os.path.isfile(os.path.join(crate_dir, f))
        )
        print(f"\n\n--- {p['name']} {p['version']} ({p.get('license') or 'no licence field'}) ---")
        if not files:
            print("\n(no licence file shipped in the crate package)")
        for f in files:
            with open(os.path.join(crate_dir, f), encoding="utf-8", errors="replace") as fh:
                print(f"\n[{f}]\n")
                print(fh.read().rstrip())


if __name__ == "__main__":
    main()
