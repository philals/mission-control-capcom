#!/usr/bin/env bash
# Builds capcom and installs `capcom` and `capcom-tui` into ~/.local/bin (or $CAPCOM_INSTALL_ROOT/bin).
# Herdr runs this during `herdr plugin install`; it needs Rust (https://rustup.rs).
set -euo pipefail

PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1; then
  printf 'capcom: Rust is required to build it, but cargo was not found (https://rustup.rs)\n' >&2
  exit 1
fi

cd "$(dirname "${BASH_SOURCE[0]}")/.."
exec cargo install --path . --locked --root "${CAPCOM_INSTALL_ROOT:-$HOME/.local}"
