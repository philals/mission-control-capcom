#!/usr/bin/env bash
# Installs capcom's Herdr mods and the `capcom` layout from herdr-mod/ (the source of truth) into the Herdr app.
# Needs `capcom` on PATH (scripts/install.sh) and the Herdr app running.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
home="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
for name in capcom-prs capcom-agents capcom-board capcom-tabs; do
  mod="$home/mods/$name"
  mkdir -p "$mod"
  cp herdr-mod/$name/*.js herdr-mod/$name/mod.json herdr-mod/shared/*.js "$mod/"
  cp herdr-mod/$name/*.svg "$mod/" 2>/dev/null || true
  if [ "$name" = capcom-agents ]; then
    node scripts/gen-avatars.mjs "$mod" >/dev/null || echo "capcom: node is needed to draw the SG-1 portraits and rings" >&2
    mkdir -p "$mod/avatars"
  fi
  herdr-app check "$mod"
done

mkdir -p "$home/layouts"
cp herdr-mod/layouts/capcom.json "$home/layouts/capcom.json"
herdr-app do layout.switch --name capcom >/dev/null
echo "capcom mods and layout installed in $home; capcom layout is showing"
