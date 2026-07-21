# Corral config — copy to:
#   plugin:     $(herdr plugin config-dir corral)/config.sh
#   standalone: ~/.config/corral/config.sh
#
# bind <key> <action>
#   internal: up down top bottom toggle collapse refresh open
#   other:    shell function of that name
#
# Herdr note: `pane split` cannot take a command. Open editors with:
#   split → send-text → send-keys enter
# and do NOT suspend the TUI (echo CORRAL_SUSPEND=0).

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

  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    echo CORRAL_SUSPEND=0
    local herdr="$HERDR_BIN_PATH"
    local out pid qfile
    out="$("$herdr" pane split --current --direction right --focus --ratio 0.55 2>&1)" || return 1
    pid="$(printf '%s' "$out" | sed -n 's/.*"pane_id":"\([^"]*\)".*/\1/p' | head -1)"
    [[ -n "$pid" ]] || return 1
    qfile=$(printf '%q' "$file")
    "$herdr" pane send-text "$pid" "$editor $qfile" >/dev/null
    "$herdr" pane send-keys "$pid" enter >/dev/null
    return 0
  fi

  echo CORRAL_SUSPEND=1
  # shellcheck disable=SC2086
  exec $editor "$file"
}
