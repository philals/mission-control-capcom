#!/usr/bin/env bash
# Installs capcom's Herdr mods and the `capcom` layout from herdr-mod/ (the source of truth) into the Herdr app.
# Needs `capcom` on PATH (scripts/install.sh) and the Herdr app running.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
home="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
for name in capcom-prs capcom-agents capcom-board capcom-jira capcom-plan; do
  mod="$home/mods/$name"
  mkdir -p "$mod"
  cp herdr-mod/$name/*.js herdr-mod/$name/mod.json herdr-mod/shared/*.js "$mod/"
  cp herdr-mod/$name/*.svg "$mod/" 2>/dev/null || true
  if [ "$name" = capcom-agents ]; then
    node scripts/gen-avatars.mjs "$mod" >/dev/null || echo "capcom: node is needed to draw the SG-1 portraits and rings" >&2
    mkdir -p "$mod/avatars"
    cp herdr-mod/capcom-agents/avatars/*.png "$mod/avatars/"
  fi
  herdr-app check "$mod"
done

# capcom-jira runs this to fetch your Jira tickets through Claude's Atlassian connector.
mkdir -p "$HOME/.local/bin"
install -m 755 scripts/capcom-jira-me "$HOME/.local/bin/capcom-jira-me"

# Claude Code's UserPromptSubmit hook that saves a session's first prompt as its note's task (see the agent-note skill).
install -m 755 scripts/capcom-note-hook "$HOME/.local/bin/capcom-note-hook"

# capcom-tabs is Herdr's own `tabs` mod plus Kanban and Jira buttons (herdr-mod/capcom-tabs.patch); only the patch is kept here.
tabs="$home/mods/capcom-tabs"
rm -rf "$tabs"
cp -r "$home/mods/tabs" "$tabs"
cp herdr-mod/capcom-tabs/*.svg "$tabs/"
cp herdr-mod/shared/identity.js "$tabs/"
rm -f "$tabs"/gpui-kit.d.ts
patch -s "$tabs/main.js" herdr-mod/capcom-tabs.patch
sed -i 's#mods/tabs/#mods/capcom-tabs/#g' "$tabs/main.js"
printf '{ "place": "none", "title": "Tabs" }\n' > "$tabs/mod.json"
herdr-app check "$tabs"

# capcom-pane is Herdr's own `pane` header plus the session's persona, ticket, task and branch in its ticket colour (herdr-mod/capcom-pane.patch).
pane="$home/mods/capcom-pane"
rm -rf "$pane"
cp -r "$home/mods/pane" "$pane"
cp herdr-mod/shared/identity.js "$pane/"
rm -f "$pane"/gpui-kit.d.ts
patch -s "$pane/main.js" herdr-mod/capcom-pane.patch
sed -i 's#mods/pane/#mods/capcom-pane/#g' "$pane/main.js"
printf '{ "place": "none", "title": "Pane header" }\n' > "$pane/mod.json"
herdr-app check "$pane"

mkdir -p "$home/layouts"
cp herdr-mod/layouts/capcom.json "$home/layouts/capcom.json"
herdr-app do layout.switch --name capcom >/dev/null
echo "capcom mods and layout installed in $home; capcom layout is showing"
