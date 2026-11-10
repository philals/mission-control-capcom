#!/usr/bin/env bash
# Installs the PR sidebar mod into the Herdr app and adds it to a `capcom` layout (a clone of `main`).
# Needs `capcom` on PATH (scripts/install.sh) and the Herdr app running.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
home="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
mod="$home/mods/capcom-prs"

mkdir -p "$mod"
cp herdr-mod/capcom-prs/main.js herdr-mod/capcom-prs/mod.json "$mod/"
herdr-app check "$mod"

if [ ! -f "$home/layouts/capcom.json" ]; then
  herdr-app do layout.clone --name main --as capcom >/dev/null
fi
python3 -I - "$home/layouts/capcom.json" <<'PY'
import json, sys

path = sys.argv[1]
layout = json.load(open(path))

def find(node, id):
    if node.get("id") == id:
        return node
    for child in node.get("children") or []:
        found = find(child, id)
        if found:
            return found

lists = find(layout, "sidebar-lists")
if lists is None:
    sys.exit("capcom: no sidebar-lists block in the layout; add a block running the capcom-prs mod yourself")
if not find(layout, "prs"):
    lists["children"].append({"id": "prs", "type": "block", "size": {"weight": 1}, "min": 120, "content": {"mod": "capcom-prs"}})
    json.dump(layout, open(path, "w"), indent=2)
PY
herdr-app do layout.switch --name capcom >/dev/null
echo "capcom-prs installed in $home and shown in the capcom layout"
