#!/usr/bin/env bash
# Open/focus Corral on the true left edge (32 columns).
# Expects `corral` on PATH (/usr/bin). Plugin scripts live under HERDR_PLUGIN_ROOT.
set -uo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"
herdr_bin="$(command -v "$herdr_bin" 2>/dev/null || printf '%s' "$herdr_bin")"
bin="$(command -v corral 2>/dev/null || true)"
if [[ -z "$bin" || ! -x "$bin" ]]; then
  printf 'corral: not on PATH (install package: /usr/bin/corral)\n' >&2
  exit 1
fi

focus_pane() {
  local pane=$1
  "$herdr_bin" pane zoom "$pane" --on >/dev/null 2>&1 \
    && "$herdr_bin" pane zoom "$pane" --off >/dev/null 2>&1
}

resize_sidebar() {
  local pane=$1 plan direction amount
  plan="$("$herdr_bin" pane layout --pane "$pane" 2>/dev/null \
    | "$bin" --resize-plan "$pane" 2>/dev/null || true)"
  [ -n "$plan" ] || return 0
  direction="${plan%%	*}"
  amount="${plan#*	}"
  "$herdr_bin" pane resize --pane "$pane" --direction "$direction" --amount "$amount" \
    >/dev/null 2>&1
}

runtime_dir="${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"
lock_dir="$runtime_dir/corral-open-${UID:-$(id -u)}.lock"
owner_file="$lock_dir/owner"
locked=""
for _ in $(seq 1 100); do
  if mkdir "$lock_dir" 2>/dev/null; then
    printf '%s\n' "$$" >"$owner_file"
    locked=1
    break
  fi
  owner=$(cat "$owner_file" 2>/dev/null || true)
  if [[ "$owner" =~ ^[0-9]+$ ]] && ! kill -0 "$owner" 2>/dev/null; then
    rm -rf "$lock_dir" 2>/dev/null || true
    continue
  fi
  sleep 0.05
done
[ -n "$locked" ] || exit 1
cleanup_lock() {
  [ "$(cat "$owner_file" 2>/dev/null || true)" = "$$" ] && rm -rf "$lock_dir" 2>/dev/null || true
}
trap cleanup_lock EXIT

panes="$("$herdr_bin" pane list 2>/dev/null || true)"
[ -n "$panes" ] || exit 1

while true; do
  decision="$(printf '%s' "$panes" | "$bin" --launch-decision 2>/dev/null || echo OPEN)"
  case "$decision" in
    "FOCUS "*)
      pid="${decision#FOCUS }"
      # Toggle: if already focused → close. Otherwise → focus.
      if [[ "$fid" == "$pid" ]]; then
        "$herdr_bin" pane close "$pid" >/dev/null 2>&1 || exit 1
        exit 0
      fi
      resize_sidebar "$pid" || exit 1
      focus_pane "$pid" || exit 1
      exit 0
      ;;
    "REPLACE "*)
      pid="${decision#REPLACE }"
      "$herdr_bin" pane close "$pid" >/dev/null 2>&1 || exit 1
      panes="$("$herdr_bin" pane list 2>/dev/null || true)"
      [ -n "$panes" ] || exit 1
      ;;
    *) break ;;
  esac
done

focused="$(printf '%s' "$panes" | "$bin" --focused-pane 2>/dev/null || true)"
fid="${focused%%	*}"
fcwd="${focused#*	}"
[ -n "$fid" ] || exit 1

layout="$("$herdr_bin" pane layout --pane "$fid" 2>/dev/null || true)"
plan="$(printf '%s' "$layout" | "$bin" --open-plan 2>/dev/null || true)"
[ -n "$plan" ] || exit 1
target="${plan%%	*}"

prepare="$(printf '%s' "$layout" | "$bin" --prepare-split-plan "$target" 2>/dev/null || true)"
if [ -n "$prepare" ]; then
  direction="${prepare%%	*}"
  amount="${prepare#*	}"
  "$herdr_bin" pane resize --pane "$target" --direction "$direction" --amount "$amount" \
    >/dev/null 2>&1 || exit 1
  layout="$("$herdr_bin" pane layout --pane "$target" 2>/dev/null || true)"
  plan="$(printf '%s' "$layout" | "$bin" --open-plan 2>/dev/null || true)"
  [ -n "$plan" ] || exit 1
  target="${plan%%	*}"
fi

# Direct plugin spawn + env (no pane-run shell echo).
spawn_args=(
  plugin pane open
  --plugin corral
  --entrypoint sidebar
  --placement split
  --direction right
  --target-pane "$target"
  --no-focus
  --env "HERDR_ENV=1"
  --env "HERDR_BIN_PATH=$herdr_bin"
)
[ -n "$fcwd" ] && spawn_args+=(--cwd "$fcwd")
out="$("$herdr_bin" "${spawn_args[@]}" 2>/dev/null || true)"
np="$(printf '%s' "$out" | jq -r '.result.plugin_pane.pane.pane_id // empty' 2>/dev/null || true)"
[ -n "$np" ] || exit 1
cleanup_new() { "$herdr_bin" pane close "$np" >/dev/null 2>&1 || true; }

if ! "$herdr_bin" pane swap --source-pane "$np" --target-pane "$target" >/dev/null 2>&1; then
  cleanup_new
  exit 1
fi
if ! "$herdr_bin" pane rename "$np" Corral >/dev/null 2>&1; then
  cleanup_new
  exit 1
fi

live=""
for _ in $(seq 1 30); do
  if [ "$("$herdr_bin" pane list 2>/dev/null | "$bin" --pane-live "$np" 2>/dev/null || true)" = LIVE ]; then
    live=1
    break
  fi
  sleep 0.1
done
if [ -z "$live" ]; then
  cleanup_new
  exit 1
fi

resize_sidebar "$np" || { cleanup_new; exit 1; }
focus_pane "$np" || exit 1
