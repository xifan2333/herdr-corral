#!/usr/bin/env bash
# Link-handler action: open a clicked GitHub issue/PR URL in a preview pane.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$script_dir/lib.sh"

url="${HERDR_PLUGIN_CLICKED_URL:-}"
if [[ -z "$url" ]]; then
  echo "missing HERDR_PLUGIN_CLICKED_URL" >&2
  exit 0
fi

workbench_lock
workbench_open_pane link-preview --env "GITHUB_URL=$url"
