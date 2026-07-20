#!/usr/bin/env bash
# SCM pane: lazygit for the current workspace.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$script_dir/lib.sh"

if ! command -v lazygit >/dev/null 2>&1; then
  cat <<'EOF'
workbench Source Control needs `lazygit`.

Install (examples):
  # Arch
  sudo pacman -S lazygit
  # go
  go install github.com/jesseduffield/lazygit@latest

Press enter to close this pane.
EOF
  read -r _ || true
  exit 0
fi

root="$(workbench_git_root .)"
if [[ -n "$root" ]]; then
  cd "$root"
fi

exec lazygit
