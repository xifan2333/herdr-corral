# Corral default config (shell). Shipped with the plugin as config.default.sh.
#
# On first run Corral copies this to your editable config:
#   plugin:     $(herdr plugin config-dir corral)/config.sh
#   standalone: ${XDG_CONFIG_HOME:-~/.config}/corral/config.sh
# Edit THAT file — no recompile needed. Delete it to re-seed from this default.
#
# River-style: call `corral bind <key> <action>` (like `riverctl map …`).
#   internal actions: up down top bottom toggle collapse refresh open
#   any other action = a shell function of that name (defined below)

corral bind j down
corral bind down down
corral bind k up
corral bind up up
corral bind g top
corral bind G bottom
corral bind h collapse
corral bind left collapse
corral bind l toggle
corral bind right toggle
corral bind enter toggle
corral bind r refresh

CORRAL_EDITOR_LABEL="Corral Editor"

open() {
  local file="${1:-${CORRAL_FILE:-}}"
  [[ -n "$file" && -e "$file" ]] || return 1
  local editor="${EDITOR:-${VISUAL:-vi}}" qfile vfile
  qfile=$(printf '%q' "$file")
  vfile=${file// /\\ }

  # --- herdr ---
  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    echo CORRAL_SUSPEND=0
    local herdr="$HERDR_BIN_PATH" pid out
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

  # --- wezterm ---
  # Prefer remembered pane, else same-tab rightmost other pane, else split once.
  # Always send editor into that pane (never `split-pane -- $editor file`).
  if [[ -n "${WEZTERM_PANE:-}" ]] && command -v wezterm >/dev/null 2>&1; then
    echo CORRAL_SUSPEND=0
    local state="$CORRAL_CONFIG_DIR/wezterm-editor.pane" me="$WEZTERM_PANE" pid title panes
    panes="$(wezterm cli list --format json 2>/dev/null || true)"
    pid="$(jq -r --argjson me "$me" --arg s "$(cat "$state" 2>/dev/null || true)" '
      ($s | tonumber? // empty) as $saved
      | if $saved and any(.[]; .pane_id == $saved) and $saved != $me then $saved
        else
          (map(select(.pane_id == $me))[0].tab_id) as $tab
          | [ .[] | select(.tab_id == $tab and .pane_id != $me) ]
          | sort_by(-.left_col) | .[0].pane_id // empty
        end
      ' <<<"$panes" 2>/dev/null || true)"

    if [[ -z "$pid" ]]; then
      pid="$(wezterm cli split-pane --pane-id "$me" --right --percent 75 2>/dev/null | tr -d '[:space:]')" || return 1
      [[ -n "$pid" ]] || return 1
      sleep 0.15
      panes="$(wezterm cli list --format json 2>/dev/null || true)"
    fi
    printf '%s' "$pid" >"$state"

    title="$(jq -r --argjson id "$pid" '.[] | select(.pane_id == $id) | .title // empty' <<<"$panes" 2>/dev/null || true)"
    wezterm cli activate-pane --pane-id "$pid" >/dev/null 2>&1 || true
    if [[ "$editor" == *nvim* || "$editor" == *vim* || "$editor" == *vi ]] \
      && [[ "$title" == *nvim* || "$title" == *vim* ]]; then
      wezterm cli send-text --pane-id "$pid" --no-paste $'\x1b:edit '"$vfile"$'\r' >/dev/null
    else
      wezterm cli send-text --pane-id "$pid" --no-paste $'\x03'"$editor $qfile"$'\r' >/dev/null
    fi
    return 0
  fi

  echo CORRAL_SUSPEND=1
  # shellcheck disable=SC2086
  exec $editor "$file"
}
