#!/usr/bin/env bash
# GitHub hub pane: interactive menu for Issues / PRs / Actions via `gh` + fzf.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$script_dir/lib.sh"

section="${WORKBENCH_GITHUB_SECTION:-menu}"

need_tools() {
  if ! command -v gh >/dev/null 2>&1; then
    cat <<'EOF'
workbench GitHub needs the GitHub CLI (`gh`).

  # Arch
  sudo pacman -S github-cli
  gh auth login

Press enter to close this pane.
EOF
    read -r _ || true
    exit 0
  fi
  if ! command -v fzf >/dev/null 2>&1; then
    cat <<'EOF'
workbench GitHub needs `fzf` for the picker UI.

  # Arch
  sudo pacman -S fzf

Press enter to close this pane.
EOF
    read -r _ || true
    exit 0
  fi
}

need_tools

repo="$(workbench_gh_repo . || true)"
if [[ -z "$repo" ]]; then
  cat <<'EOF'
Could not resolve a GitHub repository from the current directory.

Make sure:
  - you are inside a git checkout
  - origin points at github.com
  - `gh auth status` is logged in

Press enter to close this pane.
EOF
  read -r _ || true
  exit 0
fi

header() {
  clear
  printf 'GitHub  %s\n' "$repo"
  printf '────────────────────────────────────────\n'
}

pick_or_back() {
  # Read newline-separated choices from stdin; print selection or empty on cancel.
  fzf --height=100% --reverse --border --prompt="$1 > " \
    --header="$2" \
    --bind 'esc:abort' || true
}

view_issue() {
  local number="$1"
  header
  printf 'Issue #%s\n\n' "$number"
  gh issue view "$number" --repo "$repo" --comments || true
  printf '\n'
  read -r -p '[o]pen browser  [b]ack  > ' key || true
  case "${key:-}" in
    o|O) gh issue view "$number" --repo "$repo" --web || true; sleep 0.5 ;;
  esac
}

view_pr() {
  local number="$1"
  header
  printf 'Pull Request #%s\n\n' "$number"
  gh pr view "$number" --repo "$repo" --comments || true
  printf '\n── checks ──\n'
  gh pr checks "$number" --repo "$repo" 2>/dev/null || true
  printf '\n'
  read -r -p '[o]pen browser  [d]iff  [b]ack  > ' key || true
  case "${key:-}" in
    o|O) gh pr view "$number" --repo "$repo" --web || true; sleep 0.5 ;;
    d|D)
      header
      gh pr diff "$number" --repo "$repo" | ${PAGER:-less -R} || true
      ;;
  esac
}

view_run() {
  local run_id="$1"
  header
  printf 'Workflow run %s\n\n' "$run_id"
  gh run view "$run_id" --repo "$repo" || true
  printf '\n'
  read -r -p '[o]pen browser  [l]ogs  [b]ack  > ' key || true
  case "${key:-}" in
    o|O) gh run view "$run_id" --repo "$repo" --web || true; sleep 0.5 ;;
    l|L)
      header
      gh run view "$run_id" --repo "$repo" --log | ${PAGER:-less -R} || true
      ;;
  esac
}

browse_issues() {
  while true; do
    header
    printf 'Loading open issues…\n'
    local list choice number
    list="$(gh issue list --repo "$repo" --limit 40 \
      --json number,title,author,updatedAt \
      --jq '.[] | "#\(.number)\t\(.title)\t@\(.author.login)\t\(.updatedAt)"' 2>/dev/null || true)"
    if [[ -z "$list" ]]; then
      printf 'No open issues.\n\n'
      read -r -p 'Press enter to go back… ' _ || true
      return 0
    fi
    choice="$(printf '%s\n' "$list" | pick_or_back "issues" "$repo  ·  esc back")"
    [[ -z "$choice" ]] && return 0
    number="$(printf '%s' "$choice" | sed -n 's/^#\([0-9]*\).*/\1/p')"
    [[ -n "$number" ]] && view_issue "$number"
  done
}

browse_prs() {
  while true; do
    header
    printf 'Loading open pull requests…\n'
    local list choice number
    list="$(gh pr list --repo "$repo" --limit 40 \
      --json number,title,author,headRefName,updatedAt \
      --jq '.[] | "#\(.number)\t\(.title)\t@\(.author.login)\t\(.headRefName)\t\(.updatedAt)"' 2>/dev/null || true)"
    if [[ -z "$list" ]]; then
      printf 'No open pull requests.\n\n'
      read -r -p 'Press enter to go back… ' _ || true
      return 0
    fi
    choice="$(printf '%s\n' "$list" | pick_or_back "pulls" "$repo  ·  esc back")"
    [[ -z "$choice" ]] && return 0
    number="$(printf '%s' "$choice" | sed -n 's/^#\([0-9]*\).*/\1/p')"
    [[ -n "$number" ]] && view_pr "$number"
  done
}

browse_actions() {
  while true; do
    header
    printf 'Loading recent workflow runs…\n'
    local list choice run_id
    list="$(gh run list --repo "$repo" --limit 30 \
      --json databaseId,displayTitle,status,conclusion,workflowName,headBranch,createdAt \
      --jq '.[] | "\(.databaseId)\t\(.status)/\(.conclusion // "-")\t\(.workflowName)\t\(.displayTitle)\t\(.headBranch)\t\(.createdAt)"' 2>/dev/null || true)"
    if [[ -z "$list" ]]; then
      printf 'No workflow runs found.\n\n'
      read -r -p 'Press enter to go back… ' _ || true
      return 0
    fi
    choice="$(printf '%s\n' "$list" | pick_or_back "actions" "$repo  ·  esc back")"
    [[ -z "$choice" ]] && return 0
    run_id="$(printf '%s' "$choice" | awk -F'\t' 'NF{print $1; exit}')"
    [[ -n "$run_id" ]] && view_run "$run_id"
  done
}

main_menu() {
  while true; do
    header
    printf '  [1] Issues\n'
    printf '  [2] Pull Requests\n'
    printf '  [3] Actions\n'
    printf '  [w] Open repo in browser\n'
    printf '  [q] Quit (close pane)\n\n'
    read -r -p 'Select > ' key || true
    case "${key:-}" in
      1|i|I) browse_issues ;;
      2|p|P) browse_prs ;;
      3|a|A) browse_actions ;;
      w|W) gh repo view "$repo" --web || true; sleep 0.5 ;;
      q|Q|"") exit 0 ;;
    esac
  done
}

case "$section" in
  issues) browse_issues; main_menu ;;
  prs|pulls) browse_prs; main_menu ;;
  actions|runs) browse_actions; main_menu ;;
  *) main_menu ;;
esac
