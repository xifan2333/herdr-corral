# Corral default config (shell). Shipped with the plugin as config.default.sh.
#
# On first run Corral copies this to your editable config:
#   plugin:     $(herdr plugin config-dir corral)/config.sh
#   standalone: ${XDG_CONFIG_HOME:-~/.config}/corral/config.sh
# Edit THAT file — no recompile needed. Delete it to re-seed from this default.
#
# River-style: call `corral bind <key> <action>` (like `riverctl map …`).
#   global actions: quit feature-explorer feature-scm feature-github
#   navigation:     up down top bottom page-up page-down toggle expand
#                   collapse collapse-all toggle-hidden refresh open
#   SCM actions:    scm-toggle-stage scm-stage-all scm-unstage-all scm-open-diff
#                   scm-focus-message scm-discard scm-confirm scm-cancel scm-sync
#   text editing:   edit-backspace edit-delete edit-home edit-end
#   any other action = a shell function of that name (defined below)
#
# Views may emit shell actions after a configured internal action:
#   open    Explorer  → open a file in the reused side pane / $EDITOR
#   diff*   SCM        → staged / worktree / untracked diff in the reused pane
#   commit_message SCM → commit the panel's inline message and report status

corral bind q quit
corral bind ctrl+c quit
corral bind 1 feature-explorer
corral bind 2 feature-scm
corral bind 3 feature-github

corral bind j down
corral bind down down
corral bind k up
corral bind up up
corral bind g top
corral bind G bottom
corral bind pageup page-up
corral bind pagedown page-down
corral bind h collapse
corral bind left collapse
corral bind l expand
corral bind right expand
corral bind enter toggle
corral bind . toggle-hidden
corral bind z collapse-all
corral bind r refresh

corral bind s scm-toggle-stage
corral bind space scm-toggle-stage
corral bind a scm-stage-all
corral bind u scm-unstage-all
corral bind o scm-open-diff
corral bind c scm-focus-message
corral bind D scm-discard
corral bind y scm-confirm
corral bind n scm-cancel
corral bind esc scm-cancel
corral bind S scm-sync
corral bind backspace edit-backspace
corral bind delete edit-delete
corral bind home edit-home
corral bind end edit-end

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
  # Resolve a target pane: remembered → same-tab rightmost → split once.
  # Always send editor into it; never `split-pane -- $editor file`.
  if [[ -n "${WEZTERM_PANE:-}" ]] && command -v wezterm >/dev/null 2>&1; then
    echo CORRAL_SUSPEND=0
    local me="$WEZTERM_PANE" state="$CORRAL_CONFIG_DIR/wezterm-editor.pane"
    local pid title panes saved

    panes="$(wezterm cli list --format json 2>/dev/null || true)"
    saved="$(cat "$state" 2>/dev/null || true)"

    pid="$(jq -r --argjson me "$me" --arg saved "$saved" '
      ($saved | tonumber? // 0) as $s
      | if $s > 0 and any(.[]; .pane_id == $s) and $s != $me then $s
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
    # nvim/vim already running in the pane → :edit; otherwise launch editor.
    if [[ "$title" == *nvim* || "$title" == *vim* ]]; then
      wezterm cli send-text --pane-id "$pid" --no-paste $'\e:edit '"$vfile"$'\r' >/dev/null
    else
      wezterm cli send-text --pane-id "$pid" --no-paste $'\003'"$editor $qfile"$'\r' >/dev/null
    fi
    return 0
  fi

  echo CORRAL_SUSPEND=1
  # shellcheck disable=SC2086
  exec $editor "$file"
}

# --- shared: find or create the reused Corral side pane (herdr), echo pane id ---
_corral_pane() {
  local herdr="$HERDR_BIN_PATH" pid out
  pid="$("$herdr" pane list 2>/dev/null \
    | jq -r --arg l "$CORRAL_EDITOR_LABEL" \
        'first(.result.panes[] | select(.label == $l) | .pane_id) // empty' 2>/dev/null || true)"
  if [[ -z "$pid" ]]; then
    out="$("$herdr" pane split --current --direction right --focus --ratio 0.75 2>&1)" || return 1
    pid="$(printf '%s' "$out" | jq -r '.result.pane.pane_id // empty' 2>/dev/null || true)"
    [[ -n "$pid" ]] || return 1
    "$herdr" pane rename "$pid" "$CORRAL_EDITOR_LABEL" >/dev/null 2>&1 || true
  else
    "$herdr" pane zoom "$pid" --on >/dev/null 2>&1 || true
    "$herdr" pane zoom "$pid" --off >/dev/null 2>&1 || true
  fi
  printf '%s' "$pid"
}

# Send a command line into the reused pane, interrupting whatever runs there.
_corral_run() {
  local cmd="$1" herdr="$HERDR_BIN_PATH" pid
  pid="$(_corral_pane)" || return 1
  "$herdr" pane send-keys "$pid" ctrl+c >/dev/null 2>&1 || true
  "$herdr" pane send-text "$pid" "$cmd" >/dev/null
  "$herdr" pane send-keys "$pid" enter >/dev/null
}

# Diff renderer for SCM previews:
#   corral (default) | difft | fancy | delta | git
# `corral` is our standalone filter: same Palette as the sidebar, dual line
# gutters, red/green row tints, and word-level change highlighting.
CORRAL_DIFF_TOOL="${CORRAL_DIFF_TOOL:-corral}"

# Optional delta fallback settings when CORRAL_DIFF_TOOL=delta.
CORRAL_DELTA_THEME="${CORRAL_DELTA_THEME:-Catppuccin Mocha}"
_corral_delta_opts() {
  printf -- '--syntax-theme=%q --line-numbers --hunk-header-decoration-style=none --file-decoration-style="#45475a ul" --file-style="#89b4fa bold" --paging=always' \
    "$CORRAL_DELTA_THEME"
}

# A plain unified-diff producer. `kind` is explicit because an MM path appears
# in both lists with different patches. Untracked files need `--no-index`;
# git's exit 1 then means "different", not failure. Rename/copy previews pass
# both pathspecs so Git retains their structural semantics.
_corral_diff_source() {
  local kind=$1 qdir=$2 qfile=$3 qorig=${4:-} color=${5:-never} color_arg="" paths
  [[ "$color" == always ]] && color_arg='-c color.ui=always '
  paths=$qfile
  [[ -n "$qorig" ]] && paths="$paths $qorig"
  case "$kind" in
    staged) printf 'git -C %s %sdiff --cached -- %s' "$qdir" "$color_arg" "$paths" ;;
    untracked) printf '{ git -C %s %sdiff --no-index -- /dev/null %s || test $? -eq 1; }' "$qdir" "$color_arg" "$qfile" ;;
    *) printf 'git -C %s %sdiff -- %s' "$qdir" "$color_arg" "$paths" ;;
  esac
}

# Build one complete diff command for <kind> <dir> <file> [orig]. Resolve
# external programs to absolute paths here: the reused pane's interactive shell
# may have a different PATH from Corral's action process.
_corral_diff_cmd() {
  local kind=$1 dir=$2 file=$3 orig=${4:-} qdir qfile qorig source tool bin
  qdir=$(printf '%q' "$dir"); qfile=$(printf '%q' "$file")
  qorig=""; [[ -n "$orig" ]] && qorig=$(printf '%q' "$orig")
  tool=$CORRAL_DIFF_TOOL
  source=$(_corral_diff_source "$kind" "$qdir" "$qfile" "$qorig")

  case "$tool" in
    corral)
      bin=$(command -v corral-diff 2>/dev/null || true)
      if [[ -n "$bin" ]]; then
        printf '%s | %q | less -R' "$source" "$bin"
        return
      fi
      # A partial install without corral-diff degrades to plain colored git.
      ;;
    difft)
      bin=$(command -v difft 2>/dev/null || true)
      if [[ -n "$bin" ]]; then
        if [[ "$kind" == untracked ]]; then
          printf 'DFT_COLOR=always %q /dev/null %s | less -R' "$bin" "$qfile"
        else
          local cached=""
          [[ "$kind" == staged ]] && cached='--cached '
          printf 'DFT_COLOR=always GIT_EXTERNAL_DIFF=%q git -C %s --no-pager diff %s-- %s %s | less -R' "$bin" "$qdir" "$cached" "$qfile" "$qorig"
        fi
        return
      fi
      ;;
    fancy)
      bin=$(command -v diff-so-fancy 2>/dev/null || true)
      if [[ -n "$bin" ]]; then
        source=$(_corral_diff_source "$kind" "$qdir" "$qfile" "$qorig" always)
        printf '%s | %q | less -R' "$source" "$bin"
        return
      fi
      ;;
    delta)
      bin=$(command -v delta 2>/dev/null || true)
      if [[ -n "$bin" ]]; then
        printf '%s | %q %s' "$source" "$bin" "$(_corral_delta_opts)"
        return
      fi
      ;;
    git) ;;
    *) printf 'printf %q >&2; false' "corral: unknown CORRAL_DIFF_TOOL=$tool"; return ;;
  esac

  source=$(_corral_diff_source "$kind" "$qdir" "$qfile" "$qorig" always)
  printf '%s | less -R' "$source"
}

_corral_show_diff() {
  local kind=$1 file="${2:-${CORRAL_FILE:-}}" dir path orig cmd
  [[ -n "$file" ]] || return 1
  # SCM provides a stable repository root and repo-relative path. The fallback
  # keeps custom/manual calls compatible, while deleted SCM paths no longer
  # depend on a parent directory that may not exist.
  dir="${CORRAL_GIT_ROOT:-$(dirname "$file")}"
  path="${CORRAL_GIT_PATH:-$file}"
  orig="${CORRAL_GIT_ORIG:-}"
  cmd=$(_corral_diff_cmd "$kind" "$dir" "$path" "$orig")
  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    echo CORRAL_SUSPEND=0
    _corral_run "$cmd"
    return 0
  fi
  echo CORRAL_SUSPEND=1
  eval "$cmd"
}

# Feature actions emitted by SCM; keep kind explicit across the shell boundary.
diff() { _corral_show_diff unstaged "$@"; }
diff_staged() { _corral_show_diff staged "$@"; }
diff_untracked() { _corral_show_diff untracked "$@"; }

# Commit the SCM panel's inline message. The TUI receives success/failure and
# keeps the message on failure; no terminal handoff is needed.
commit_message() {
  local dir="${1:-${CORRAL_FILE:-.}}" message="${CORRAL_COMMIT_MESSAGE:-}"
  [[ -n "$message" ]] || return 1
  echo CORRAL_SUSPEND=0
  git -C "$dir" commit -m "$message"
}

# Optional interactive commit action for custom user bindings.
commit() {
  local dir="${1:-${CORRAL_FILE:-.}}"
  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    echo CORRAL_SUSPEND=0
    _corral_run "git -C $(printf '%q' "$dir") commit -v"
    return 0
  fi
  echo CORRAL_SUSPEND=1
  git -C "$dir" commit -v
}
