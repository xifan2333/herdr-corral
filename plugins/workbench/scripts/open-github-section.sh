#!/usr/bin/env bash
# Open (or focus/toggle) the GitHub hub on a specific section.
# Usage: open-github-section.sh <issues|prs|actions>
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$script_dir/lib.sh"

section="${1:-menu}"
workbench_lock

# If a GitHub pane is already open in this tab, close it first so the new
# section env takes effect on reopen. Then always open fresh with the section.
decision="$(workbench_decision "GitHub")"
case "$decision" in
  "FOCUS "*|"CLOSE "*)
    pid="${decision#* }"
    "$herdr_bin" plugin pane close "$pid" >/dev/null 2>&1 || true
    sleep 0.15
    ;;
esac

workbench_open_pane github --env "WORKBENCH_GITHUB_SECTION=$section"
