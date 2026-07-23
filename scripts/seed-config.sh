#!/usr/bin/env bash
# Herdr [[startup]] hook — runs once per install / upgrade.
# Seed the user XDG config if it does not exist.
set -euo pipefail

cfg="${XDG_CONFIG_HOME:-$HOME/.config}/corral/config.sh"
if [[ -f "$cfg" ]]; then
  exit 0
fi

root="${HERDR_PLUGIN_ROOT:?HERDR_PLUGIN_ROOT required}"
template="${root}/config.default.sh"
if [[ ! -f "$template" ]]; then
  echo "corral: config.default.sh not found in plugin root" >&2
  exit 1
fi

mkdir -p "$(dirname "$cfg")"
cp "$template" "$cfg"
echo "corral: seeded ${cfg}"
