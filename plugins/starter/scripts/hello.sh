#!/usr/bin/env bash
set -euo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"
plugin_id="${HERDR_PLUGIN_ID:-workbench.starter}"
config_dir="${HERDR_PLUGIN_CONFIG_DIR:-}"
state_dir="${HERDR_PLUGIN_STATE_DIR:-}"

echo "hello from ${plugin_id}"
echo "herdr: ${herdr_bin}"
echo "root: ${HERDR_PLUGIN_ROOT:-}"
echo "config: ${config_dir}"
echo "state: ${state_dir}"
echo "workspace: ${HERDR_WORKSPACE_ID:-}"
echo "tab: ${HERDR_TAB_ID:-}"
echo "pane: ${HERDR_PANE_ID:-}"
