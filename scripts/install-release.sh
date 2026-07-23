#!/usr/bin/env bash
# Install a packaged herdr-corral release tree.
#
# Layout expected (relative to this script):
#   bin/corral
#   bin/corral-diff
#   bin/corral-github
#   share/herdr-corral/...
#
# Prefix:
#   - if PATH contains $HOME/.local/bin → $HOME/.local
#   - else → /usr (needs write access / sudo)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
home="${HOME:-}"

if [[ -n "$home" && ":${PATH}:" == *":${home}/.local/bin:"* ]]; then
  prefix="${home}/.local"
  need_root=0
else
  prefix="/usr"
  need_root=1
fi

bin_dir="${prefix}/bin"
share_dir="${prefix}/share/herdr-corral"

run() {
  if [[ "$need_root" -eq 1 && ! -w "$(dirname "$bin_dir")" ]]; then
    exec sudo env "PATH=$PATH" bash "$0" "$@"
  fi
}

if [[ "${1:-}" == "--prefix" ]]; then
  printf '%s\n' "$prefix"
  exit 0
fi

if [[ "$need_root" -eq 1 && ! -w "$(dirname "$bin_dir")" && "${EUID:-$(id -u)}" -ne 0 ]]; then
  echo "install prefix: ${prefix} (PATH has no ${home}/.local/bin; elevating)" >&2
  exec sudo env "PATH=$PATH" bash "$0"
fi

echo "install prefix: ${prefix}"

install -d "$bin_dir" "$share_dir/scripts"
install -Dm755 "${root}/bin/corral" "${bin_dir}/corral"
install -Dm755 "${root}/bin/corral-diff" "${bin_dir}/corral-diff"
install -Dm755 "${root}/bin/corral-github" "${bin_dir}/corral-github"
install -Dm644 "${root}/share/herdr-corral/config.default.sh" "${share_dir}/config.default.sh"
install -Dm644 "${root}/share/herdr-corral/herdr-plugin.toml" "${share_dir}/herdr-plugin.toml"
install -Dm755 "${root}/share/herdr-corral/scripts/open-corral.sh" "${share_dir}/scripts/open-corral.sh"
if [[ -f "${root}/share/herdr-corral/README.md" ]]; then
  install -Dm644 "${root}/share/herdr-corral/README.md" "${share_dir}/README.md"
fi

echo "installed:"
echo "  ${bin_dir}/corral"
echo "  ${bin_dir}/corral-diff"
echo "  ${bin_dir}/corral-github"
echo "  ${share_dir}/"
echo
echo "Herdr plugin still needs to be linked/installed, e.g.:"
echo "  herdr plugin install xifan2333/herdr-corral"
echo "  # or: herdr plugin link /path/to/checkout"
