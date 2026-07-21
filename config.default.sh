# Corral default config (shell). Shipped with the plugin as config.default.sh.
#
# On first run Corral copies this to your editable config:
#   plugin:     $(herdr plugin config-dir corral)/config.sh
#   standalone: ${XDG_CONFIG_HOME:-~/.config}/corral/config.sh
# Edit THAT file — no recompile needed. Delete it to re-seed from this default.
#
# bind <key> <action>   (internal: up down top bottom toggle collapse refresh open)
# Any other action name = a shell function of that name below.
# open() reuses one "Corral Editor" pane when possible (no new split every time).

bind j down
bind down down
bind k up
bind up up
bind g top
bind G bottom
bind h collapse
bind left collapse
bind l toggle
bind right toggle
bind enter toggle
bind r refresh

CORRAL_EDITOR_LABEL="Corral Editor"

open() {
  local file="${1:-${CORRAL_FILE:-}}"
  [[ -n "$file" && -e "$file" ]] || return 1
  local editor="${EDITOR:-${VISUAL:-vi}}"
  # qfile: shell-quoted; vfile: vim :edit escaped (spaces only)
  local qfile vfile
  qfile=$(printf '%q' "$file")
  vfile=${file// /\\ }

  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    echo CORRAL_SUSPEND=0
    local herdr="$HERDR_BIN_PATH" pid="" out
    pid="$("$herdr" pane list 2>/dev/null \
      | jq -r --arg l "$CORRAL_EDITOR_LABEL" \
          'first(.result.panes[] | select(.label == $l) | .pane_id) // empty' 2>/dev/null || true)"

    if [[ -z "$pid" ]]; then
      out="$("$herdr" pane split --current --direction right --focus --ratio 0.75 2>&1)" || return 1
      pid="$(printf '%s' "$out" | jq -r '.result.pane.pane_id // empty' 2>/dev/null || true)"
      [[ -n "$pid" ]] || return 1
      "$herdr" pane rename "$pid" "$CORRAL_EDITOR_LABEL" >/dev/null 2>&1 || true
      "$herdr" pane send-text "$pid" "$editor $qfile" >/dev/null
      "$herdr" pane send-keys "$pid" enter >/dev/null
    else
      "$herdr" pane zoom "$pid" --on >/dev/null 2>&1 || true
      "$herdr" pane zoom "$pid" --off >/dev/null 2>&1 || true
      case "$editor" in
        *nvim*|*vim*|*vi)
          "$herdr" pane send-keys "$pid" esc >/dev/null
          "$herdr" pane send-text "$pid" ":edit $vfile" >/dev/null
          "$herdr" pane send-keys "$pid" enter >/dev/null
          ;;
        *)
          "$herdr" pane send-keys "$pid" ctrl+c >/dev/null
          "$herdr" pane send-text "$pid" "$editor $qfile" >/dev/null
          "$herdr" pane send-keys "$pid" enter >/dev/null
          ;;
      esac
    fi
    return 0
  fi

  if [[ -n "${WEZTERM_PANE:-}" ]] && command -v wezterm >/dev/null 2>&1; then
    echo CORRAL_SUSPEND=0
    local state="${CORRAL_CONFIG_DIR:-/tmp}/wezterm-editor.pane" pid=""
    [[ -f "$state" ]] && pid="$(cat "$state" 2>/dev/null || true)"
    if [[ -n "$pid" ]] && ! wezterm cli list 2>/dev/null | awk '{print $1}' | grep -qx "$pid"; then
      pid=""
    fi
    if [[ -z "$pid" ]]; then
      # shellcheck disable=SC2086
      pid="$(wezterm cli split-pane --right --percent 75 -- $editor "$file" 2>/dev/null | tr -d '[:space:]')" || return 1
      mkdir -p "${CORRAL_CONFIG_DIR:-/tmp}"
      printf '%s' "$pid" >"$state"
    else
      wezterm cli activate-pane --pane-id "$pid" >/dev/null 2>&1 || true
      case "$editor" in
        *nvim*|*vim*|*vi)
          wezterm cli send-text --pane-id "$pid" --no-paste $'\x1b:edit '"$vfile"$'\r' >/dev/null
          ;;
        *)
          wezterm cli send-text --pane-id "$pid" --no-paste $'\x03' 2>/dev/null || true
          # shellcheck disable=SC2086
          wezterm cli send-text --pane-id "$pid" --no-paste "$editor $qfile"$'\r' >/dev/null
          ;;
      esac
    fi
    return 0
  fi

  echo CORRAL_SUSPEND=1
  # shellcheck disable=SC2086
  exec $editor "$file"
}
