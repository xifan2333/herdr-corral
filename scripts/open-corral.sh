#!/usr/bin/env bash
# Idempotently open/focus Corral, docked at the true left edge at 32 columns.
#
# Herdr only splits right/down, so we split the layout's leftmost/topmost pane
# to the right and swap the new pane into its slot. JSON decisions and ids are
# parsed/validated by the Rust binary; this shell is only an argv adapter.
set -uo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"
herdr_bin="$(command -v "$herdr_bin" 2>/dev/null || printf '%s' "$herdr_bin")"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
plugin_root="$(cd "$script_dir/.." && pwd)"
bin="$plugin_root/target/release/corral"
# User config is always ${XDG_CONFIG_HOME:-~/.config}/corral/config.sh (see
# config::config_dir). Host mode is $HERDR_ENV / $HERDR_BIN_PATH inside shell.
[ -x "$bin" ] || exit 1

# Focus by id (Herdr has no dedicated focus-by-id command).
focus_pane() {
  local pane=$1
  "$herdr_bin" pane zoom "$pane" --on >/dev/null 2>&1 \
    && "$herdr_bin" pane zoom "$pane" --off >/dev/null 2>&1
}

# Restore an existing left sidebar to exactly 32 host-layout columns.
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

# Manual invocations can race (key repeat / multiple clients). Use the private
# runtime dir where possible and store the owning PID. Never evict a live owner;
# cleanup removes only a lock still owned by this process.
runtime_dir="${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"
lock_dir="$runtime_dir/corral-open-${UID:-$(id -u)}.lock"
owner_file="$lock_dir/owner"
locked=""
for _ in $(seq 1 100); do
  if mkdir "$lock_dir" 2>/dev/null; then
    printf '%s\n' "$$" > "$owner_file"
    locked=1
    break
  fi
  owner=$(cat "$owner_file" 2>/dev/null || true)
  if [[ "$owner" =~ ^[0-9]+$ ]]; then
    if ! kill -0 "$owner" 2>/dev/null; then
      rm -rf "$lock_dir" 2>/dev/null || true
      continue
    fi
  else
    # A crash between mkdir and writing owner is reclaimable only after 30s;
    # a live creator gets time to finish publishing its PID.
    now=$(date +%s)
    born=$(stat -c %Y "$lock_dir" 2>/dev/null || stat -f %m "$lock_dir" 2>/dev/null || echo "$now")
    if [ $((now - born)) -gt 30 ]; then
      rm -rf "$lock_dir" 2>/dev/null || true
      continue
    fi
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

# Drain stale owned panes one at a time, re-deciding after every close. If any
# live candidate exists, Rust prefers it regardless of pane-list ordering.
while true; do
  decision="$(printf '%s' "$panes" | "$bin" --launch-decision 2>/dev/null || echo OPEN)"
  case "$decision" in
    "FOCUS "*)
      pid="${decision#FOCUS }"
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

# A missing/invalid layout plan is not safe to degrade: splitting the focused
# pane could place Corral in the middle of an existing multi-column layout.
layout="$("$herdr_bin" pane layout --pane "$fid" 2>/dev/null || true)"
plan="$(printf '%s' "$layout" | "$bin" --open-plan 2>/dev/null || true)"
[ -n "$plan" ] || exit 1
target="${plan%%	*}"

# If the leftmost target is already sidebar-narrow, grow it before splitting so
# the new 32-column pane does not lose columns to Herdr's minimum right child.
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
ratio="${plan#*	}"

split_args=(pane split "$target" --direction right --ratio "$ratio" --no-focus)
[ -n "$fcwd" ] && split_args+=(--cwd "$fcwd")
out="$("$herdr_bin" "${split_args[@]}" 2>/dev/null || true)"
np="$(printf '%s' "$out" | "$bin" --split-pane-id 2>/dev/null || true)"
[ -n "$np" ] || exit 1
cleanup_new() { "$herdr_bin" pane close "$np" >/dev/null 2>&1 || true; }

if ! "$herdr_bin" pane swap --source-pane "$np" --target-pane "$target" >/dev/null 2>&1; then
  cleanup_new
  exit 1
fi
printf -v run_cmd 'exec env HERDR_ENV=1 HERDR_BIN_PATH=%q HERDR_PLUGIN_ID=corral HERDR_PLUGIN_ENTRYPOINT_ID=sidebar HERDR_PLUGIN_ROOT=%q %q' \
  "$herdr_bin" "$plugin_root" "$bin"
if ! "$herdr_bin" pane run "$np" "$run_cmd" >/dev/null 2>&1; then
  cleanup_new
  exit 1
fi
if ! "$herdr_bin" pane rename "$np" Corral >/dev/null 2>&1; then
  cleanup_new
  exit 1
fi

# Keep the lock until the TUI proves liveness. Queued invocations then see a
# fresh metadata heartbeat instead of racing the process startup.
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
