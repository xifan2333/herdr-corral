#!/usr/bin/env bash
# Toggle a workbench pane by entrypoint + title.
# Usage: toggle-pane.sh <entrypoint> <title>
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$script_dir/lib.sh"

entrypoint="${1:?entrypoint required}"
title="${2:?title required}"

workbench_lock
workbench_apply_decision "$entrypoint" "$title"
