#!/usr/bin/env bash
# Summon the corral host pane as a split beside the current work.
# (Idempotent launch/focus/toggle can land later; for now always open.)
set -uo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"
plugin_id="${HERDR_PLUGIN_ID:-corral}"

exec "$herdr_bin" plugin pane open \
  --plugin "$plugin_id" \
  --entrypoint workbench \
  --placement split \
  --direction right \
  --focus
