#!/usr/bin/env bash
set -euo pipefail

echo "=== plugin env ==="
printf 'HERDR_ENV=%s\n' "${HERDR_ENV:-}"
printf 'HERDR_PLUGIN_ID=%s\n' "${HERDR_PLUGIN_ID:-}"
printf 'HERDR_PLUGIN_ROOT=%s\n' "${HERDR_PLUGIN_ROOT:-}"
printf 'HERDR_PLUGIN_CONFIG_DIR=%s\n' "${HERDR_PLUGIN_CONFIG_DIR:-}"
printf 'HERDR_PLUGIN_STATE_DIR=%s\n' "${HERDR_PLUGIN_STATE_DIR:-}"
printf 'HERDR_PLUGIN_ACTION_ID=%s\n' "${HERDR_PLUGIN_ACTION_ID:-}"
printf 'HERDR_BIN_PATH=%s\n' "${HERDR_BIN_PATH:-}"
printf 'HERDR_SOCKET_PATH=%s\n' "${HERDR_SOCKET_PATH:-}"
printf 'HERDR_WORKSPACE_ID=%s\n' "${HERDR_WORKSPACE_ID:-}"
printf 'HERDR_TAB_ID=%s\n' "${HERDR_TAB_ID:-}"
printf 'HERDR_PANE_ID=%s\n' "${HERDR_PANE_ID:-}"

echo
echo "=== HERDR_PLUGIN_CONTEXT_JSON ==="
if [[ -n "${HERDR_PLUGIN_CONTEXT_JSON:-}" ]]; then
  if command -v jq >/dev/null 2>&1; then
    printf '%s\n' "${HERDR_PLUGIN_CONTEXT_JSON}" | jq .
  else
    printf '%s\n' "${HERDR_PLUGIN_CONTEXT_JSON}"
  fi
else
  echo "(empty)"
fi
