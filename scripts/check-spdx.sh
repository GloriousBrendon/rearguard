#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Fails if a first-party source file lacks the SPDX licence header as its first line
# (second line after a shebang). Checks the files git tracks; third-party and vendored
# code is excluded and keeps its own licence. Run from the repository root.
set -euo pipefail

id='SPDX-License-Identifier: MIT OR Apache-2.0'
# Third-party and vendored code: never given our header. None is tracked today; Godot
# addons (demo/addons/) and vendored crates would go here.
exclude='^(vendor|third[_-]party|demo/addons)/|/(vendor|third[_-]party)/'

status=0
count=0
while IFS= read -r -d '' file; do
    [[ "$file" =~ $exclude ]] && continue
    case "$file" in
        *.rs) want="// $id" ;;
        *) want="# $id" ;;
    esac
    first=$(head -n 1 -- "$file" | tr -d '\r')
    if [[ "$first" == '#!'* ]]; then
        first=$(sed -n '2p' -- "$file" | tr -d '\r')
    fi
    count=$((count + 1))
    if [[ "$first" != "$want" ]]; then
        echo "error: $file: missing header: $want"
        status=1
    fi
done < <(git ls-files -z -- '*.rs' '*.gd' '*.sh' '*.py')

if [ "$status" -eq 0 ]; then
    echo "ok: $count first-party source files carry the SPDX header"
fi
exit "$status"
