#!/usr/bin/env bash
# One-shot GitHub issue/PR detail pane (from Ctrl-click link handler).
set -euo pipefail

url="${GITHUB_URL:-${HERDR_PLUGIN_CLICKED_URL:-}}"

finish() {
  printf '\npress enter to close this preview…'
  read -r _ || true
}
trap finish EXIT

if [[ -z "$url" ]]; then
  echo "missing GITHUB_URL"
  exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "gh is required for this preview."
  echo "install GitHub CLI and authenticate with: gh auth login"
  exit 0
fi

if [[ "$url" =~ ^https://github\.com/([^/]+)/([^/]+)/(issues|pull)/([0-9]+)/?$ ]]; then
  owner="${BASH_REMATCH[1]}"
  repo="${BASH_REMATCH[2]}"
  kind="${BASH_REMATCH[3]}"
  number="${BASH_REMATCH[4]}"
else
  echo "unsupported GitHub URL:"
  echo "$url"
  exit 0
fi

repo_arg="$owner/$repo"
clear
printf 'github %s #%s\n' "$kind" "$number"
printf 'repo: %s\n\n' "$repo_arg"

if [[ "$kind" == "pull" ]]; then
  gh pr view "$number" --repo "$repo_arg" --comments || true
  printf '\n── checks ──\n'
  gh pr checks "$number" --repo "$repo_arg" 2>/dev/null || true
else
  gh issue view "$number" --repo "$repo_arg" --comments || true
fi
