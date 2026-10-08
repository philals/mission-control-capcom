#!/usr/bin/env bash
# Open capcom, or focus the copy already open in this workspace.
# Settings for the TUI (CAPCOM_ROOT, CAPCOM_WORKDIR, ...) go in `KEY=VALUE` lines in the file
# `env` inside the plugin's config folder (`herdr plugin config-dir capcom`).
set -uo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"
workspace_id="${HERDR_WORKSPACE_ID:-}"
target_pane="${HERDR_PANE_ID:-}"
origin_tab="${HERDR_TAB_ID:-}"
if [ -z "$workspace_id" ] || [ -z "$target_pane" ] || [ -z "$origin_tab" ]; then
  printf 'capcom: cannot open without a Herdr workspace, tab and pane\n' >&2
  exit 1
fi

PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
if ! command -v capcom-tui >/dev/null 2>&1; then
  printf 'capcom: capcom-tui is not installed (run: cargo install --path . in the capcom repo)\n' >&2
  exit 1
fi

if ! panes="$("$herdr_bin" pane list --workspace "$workspace_id")"; then
  printf 'capcom: could not list panes in workspace %s\n' "$workspace_id" >&2
  exit 1
fi
found="$(printf '%s' "$panes" | capcom-tui --find-pane || true)"
if [ -n "$found" ]; then
  pane_id="${found%% *}"
  tab_id="${found##* }"
  # Herdr does not move attached clients on a pane focus alone, so activate the tab first.
  if "$herdr_bin" tab focus "$tab_id" && "$herdr_bin" plugin pane focus "$pane_id"; then
    exit 0
  fi
fi

"$herdr_bin" tab focus "$origin_tab" || exit 1
config_dir="${HERDR_PLUGIN_CONFIG_DIR:-$("$herdr_bin" plugin config-dir capcom 2>/dev/null || true)}"
env_args=()
if [ -n "$config_dir" ] && [ -f "$config_dir/env" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ''|'#'*) ;;
      *=*) env_args+=(--env "$line") ;;
    esac
  done < "$config_dir/env"
fi

exec "$herdr_bin" plugin pane open --plugin capcom --entrypoint tui --placement overlay \
  --workspace "$workspace_id" --focus "${env_args[@]}"
