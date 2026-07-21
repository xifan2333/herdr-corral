# Corral config — copy to:
#   plugin:     $(herdr plugin config-dir corral)/config.sh
#   standalone: ~/.config/corral/config.sh
#
# bind <key> <action>
# Herdr: pane split cannot take a command → split + send-text.
# WezTerm standalone: wezterm cli split-pane --right --percent 75 -- $EDITOR file

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

open() {
  local file="${1:-${CORRAL_FILE:-}}"
  [[ -n "$file" && -e "$file" ]] || return 1
  local editor="${EDITOR:-${VISUAL:-vi}}"
  local qfile
  qfile=$(printf '%q' "$file")

  # Herdr: wide right split + send editor command
  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    echo CORRAL_SUSPEND=0
    local herdr="$HERDR_BIN_PATH" out pid
    out="$("$herdr" pane split --current --direction right --focus --ratio 0.75 2>&1)" || return 1
    pid="$(printf '%s' "$out" | sed -n 's/.*"pane_id":"\([^"]*\)".*/\1/p' | head -1)"
    [[ -n "$pid" ]] || return 1
    "$herdr" pane send-text "$pid" "$editor $qfile" >/dev/null
    "$herdr" pane send-keys "$pid" enter >/dev/null
    return 0
  fi

  # WezTerm: native right split running $EDITOR
  if [[ -n "${WEZTERM_PANE:-}" ]] && command -v wezterm >/dev/null 2>&1; then
    echo CORRAL_SUSPEND=0
    # shellcheck disable=SC2086
    wezterm cli split-pane --right --percent 75 -- $editor "$file" >/dev/null
    return $?
  fi

  # Fallback: this TTY
  echo CORRAL_SUSPEND=1
  # shellcheck disable=SC2086
  exec $editor "$file"
}
