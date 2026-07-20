#!/usr/bin/env bash
# Explorer pane: yazi file browser in the workspace cwd.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$script_dir/lib.sh"

if ! command -v yazi >/dev/null 2>&1; then
  cat <<'EOF'
workbench Explorer needs `yazi`.

Install (examples):
  # Arch
  sudo pacman -S yazi
  # mise / cargo
  cargo install --locked yazi-fm yazi-cli

Press enter to close this pane.
EOF
  read -r _ || true
  exit 0
fi

# Start at the pane cwd (set by plugin pane open --cwd).
exec yazi .
