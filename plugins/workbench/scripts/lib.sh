#!/usr/bin/env bash
# Shared helpers for workbench.vscode panes.
# shellcheck shell=bash

set -euo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"

workbench_lock() {
  # Serialize concurrent toggle/open actions so two keypresses don't race.
  local lock_file="${TMPDIR:-/tmp}/herdr-workbench-vscode.lock"
  if ( : >>"$lock_file" ) 2>/dev/null; then
    exec 9>>"$lock_file"
    python3 -c '
import fcntl, sys, time
deadline = time.time() + 10.0
while True:
    try:
        fcntl.flock(9, fcntl.LOCK_EX | fcntl.LOCK_NB)
        sys.exit(0)
    except OSError:
        if time.time() >= deadline:
            sys.exit(1)
        time.sleep(0.05)
' || exit 0
  fi
}

workbench_target_dir() {
  python3 - <<'PY'
import json, os
d = ""
raw = os.environ.get("HERDR_PLUGIN_CONTEXT_JSON") or ""
try:
    ctx = json.loads(raw)
    if isinstance(ctx, dict):
        for key in ("focused_pane_cwd", "workspace_cwd"):
            v = ctx.get(key)
            if isinstance(v, str) and v:
                d = v
                break
except Exception:
    d = ""
print(d or os.environ.get("HOME") or os.getcwd())
PY
}

workbench_require() {
  local cmd="$1"
  local hint="${2:-}"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "workbench: missing required command: $cmd" >&2
    if [[ -n "$hint" ]]; then
      echo "$hint" >&2
    fi
    return 1
  fi
}

# Decide OPEN / FOCUS <id> / CLOSE <id> for a pane title in the current tab.
# Usage: workbench_decision <pane_title>
workbench_decision() {
  local title="$1"
  local panes_json current_json
  panes_json="$("$herdr_bin" pane list 2>/dev/null || true)"
  current_json="$("$herdr_bin" pane current 2>/dev/null || true)"

  HERDR_PANES_JSON="$panes_json" \
  HERDR_CURRENT_JSON="$current_json" \
  HERDR_PANE_TITLE="$title" \
  python3 - <<'PY' || echo OPEN
import json, os, re, sys

SAFE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9:_-]*$")
title = os.environ.get("HERDR_PANE_TITLE") or ""

def emit(s):
    print(s)
    sys.exit(0)

try:
    panes = json.loads(os.environ.get("HERDR_PANES_JSON") or "")["result"]["panes"]
    cur = json.loads(os.environ.get("HERDR_CURRENT_JSON") or "")["result"]["pane"]
except Exception:
    emit("OPEN")

matches = [
    p for p in panes
    if isinstance(p, dict)
    and p.get("workspace_id") == cur.get("workspace_id")
    and p.get("tab_id") == cur.get("tab_id")
    and p.get("label") == title
    and SAFE.match(p.get("pane_id") or "")
]

if not matches:
    emit("OPEN")

for p in matches:
    if p.get("focused"):
        emit("CLOSE " + p["pane_id"])

emit("FOCUS " + matches[0]["pane_id"])
PY
}

# Open a plugin pane split, optionally with extra --env KEY=VALUE args.
# Usage: workbench_open_pane <entrypoint> [extra herdr args...]
workbench_open_pane() {
  local entrypoint="$1"
  shift
  local target_dir
  target_dir="$(workbench_target_dir)"

  # Herdr only accepts split direction right|down. Open on the right (VS Code
  # activity-bar panels are left-docked in the GUI, but terminal splits land
  # beside the focused work pane on the right).
  exec "$herdr_bin" plugin pane open \
    --plugin workbench.vscode \
    --entrypoint "$entrypoint" \
    --placement split \
    --direction right \
    --cwd "$target_dir" \
    --focus \
    "$@"
}

# Apply OPEN/FOCUS/CLOSE decision for a pane.
# Usage: workbench_apply_decision <entrypoint> <title> [extra open args...]
workbench_apply_decision() {
  local entrypoint="$1"
  local title="$2"
  shift 2
  local decision
  decision="$(workbench_decision "$title")"

  case "$decision" in
    "FOCUS "*)
      exec "$herdr_bin" plugin pane focus "${decision#FOCUS }"
      ;;
    "CLOSE "*)
      exec "$herdr_bin" plugin pane close "${decision#CLOSE }"
      ;;
    *)
      workbench_open_pane "$entrypoint" "$@"
      ;;
  esac
}

workbench_git_root() {
  local dir="${1:-.}"
  git -C "$dir" rev-parse --show-toplevel 2>/dev/null || true
}

workbench_gh_repo() {
  local dir="${1:-.}"
  # Prefer gh's view of the remote; fall back to parsing origin URL.
  if command -v gh >/dev/null 2>&1; then
    local name
    name="$(gh -R "$(git -C "$dir" remote get-url origin 2>/dev/null || true)" repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
    if [[ -n "$name" ]]; then
      printf '%s\n' "$name"
      return 0
    fi
    name="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
    if [[ -n "$name" ]]; then
      printf '%s\n' "$name"
      return 0
    fi
  fi

  local url
  url="$(git -C "$dir" remote get-url origin 2>/dev/null || true)"
  if [[ "$url" =~ github\.com[:/]([^/]+)/([^/.]+)(\.git)?$ ]]; then
    printf '%s/%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
    return 0
  fi
  return 1
}
