# Corral config — copy to:
#   plugin:     $(herdr plugin config-dir corral)/config.sh
#   standalone: ~/.config/corral/config.sh
#
# Not executed at startup. Sourced when an action runs.
# bind <key> <action>
#   internal actions: up down top bottom toggle collapse refresh open
#   anything else: shell function of that name (define below)
# Split modules yourself:  source "${CORRAL_CONFIG_DIR}/git.sh"

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

# Open selected file in $EDITOR (Herdr: new split; standalone: this TTY).
open() {
  local file="${1:-${CORRAL_FILE:-}}"
  [[ -n "$file" && -e "$file" ]] || return 1
  local editor="${EDITOR:-${VISUAL:-vi}}"
  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    # shellcheck disable=SC2086
    exec "$HERDR_BIN_PATH" pane split --current --direction right --focus -- \
      sh -c "$editor \"\$1\"" _ "$file"
  fi
  # shellcheck disable=SC2086
  exec $editor "$file"
}
