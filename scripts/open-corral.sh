#!/usr/bin/env bash
# Dock Corral as a left sidebar pane (herdr-sidebar pattern).
#
# herdr `pane split` only supports right|down, so we:
#   1. find the leftmost pane in the focused tab
#   2. split it to the right with a narrow ratio for the NEW pane
#   3. swap the new pane into the left slot
#   4. run corral in that left pane  — OR use plugin pane open + swap
#
# For v1 we use `plugin pane open --placement split --direction right` then
# `pane swap` with the previous focused pane so Corral ends up on the left.
set -uo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"
plugin_id="${HERDR_PLUGIN_ID:-corral}"

# Remember the pane that had focus (will become the right neighbor).
focused="$("$herdr_bin" pane current 2>/dev/null || true)"
focused_id="$(
  printf '%s' "$focused" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    p = d.get("result", d).get("pane") or d.get("result", d)
    print(p.get("pane_id", "") if isinstance(p, dict) else "")
except Exception:
    print("")
' 2>/dev/null || true
)"

# Open corral as a split to the right of the focused pane.
open_out="$("$herdr_bin" plugin pane open \
  --plugin "$plugin_id" \
  --entrypoint sidebar \
  --placement split \
  --direction right \
  --focus 2>&1)" || true

new_id="$(
  printf '%s' "$open_out" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    pp = d.get("result", {}).get("plugin_pane") or {}
    pane = pp.get("pane") or {}
    print(pane.get("pane_id", ""))
except Exception:
    print("")
' 2>/dev/null || true
)"

# Swap so Corral sits on the left of the previous focused pane.
if [ -n "$new_id" ] && [ -n "$focused_id" ] && [ "$new_id" != "$focused_id" ]; then
  "$herdr_bin" pane swap --source-pane "$new_id" --target-pane "$focused_id" >/dev/null 2>&1 || true
  # Keep focus on the sidebar after swap.
  "$herdr_bin" pane zoom "$new_id" --on >/dev/null 2>&1 || true
  "$herdr_bin" pane zoom "$new_id" --off >/dev/null 2>&1 || true
fi

# Best-effort: shrink sidebar a bit (resize left edge of the right neighbor).
if [ -n "$focused_id" ]; then
  "$herdr_bin" pane resize --pane "$focused_id" --direction left --amount 0.15 >/dev/null 2>&1 || true
fi

printf '%s\n' "$open_out"
