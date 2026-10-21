#!/usr/bin/env bash
# Install published Linux binaries into the current user's ~/.local prefix.
set -euo pipefail

usage() {
  printf 'Usage: bash scripts/install-fedora.sh [VERSION]\n'
  printf 'Omit VERSION for the latest release, or specify e.g. v0.1.9.\n'
}

if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
  usage
  exit 0
fi
if (( $# > 1 )); then
  usage >&2
  exit 2
fi
if [[ "$(uname -s)" != Linux ]]; then
  printf 'corral: this installer requires Linux\n' >&2
  exit 1
fi
case "$(uname -m)" in
  x86_64) arch=x86_64 ;;
  aarch64) arch=aarch64 ;;
  *) printf 'corral: supported architectures are x86_64 and aarch64\n' >&2; exit 1 ;;
esac

repo=https://github.com/xifan2333/herdr-corral
tag="${1:-}"
if [[ -z "$tag" ]]; then
  latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "$repo/releases/latest")"
  if [[ "$latest_url" != "$repo/releases/tag/"* ]]; then
    printf 'corral: could not resolve the latest release tag\n' >&2
    exit 1
  fi
  tag="${latest_url##*/}"
fi
tag="v${tag#v}"
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
  printf 'corral: invalid release version: %s\n' "$tag" >&2
  exit 2
fi

version="${tag#v}"
archive="herdr-corral-${version}-${arch}.tar.xz"
download_dir="$(mktemp -d)"
stage_dir=""
cleanup() {
  rm -rf -- "$download_dir"
  if [[ -n "$stage_dir" ]]; then
    rm -rf -- "$stage_dir"
  fi
}
trap cleanup EXIT

url="$repo/releases/download/$tag"
curl -fsSL -o "$download_dir/$archive" "$url/$archive"
curl -fsSL -o "$download_dir/$archive.sha256" "$url/$archive.sha256"
(
  cd "$download_dir"
  sha256sum --check "$archive.sha256"
)
tar -xJf "$download_dir/$archive" -C "$download_dir"
release_dir="$download_dir/herdr-corral-${version}-${arch}"

prefix="$HOME/.local"
mkdir -p "$prefix/bin" "$prefix/share/herdr-corral"
stage_dir="$(mktemp -d "$prefix/bin/.corral-install.XXXXXX")"
install -m755 "$release_dir/bin/corral" "$stage_dir/corral"
install -m755 "$release_dir/bin/corral-github" "$stage_dir/corral-github"
install -m644 "$release_dir/share/herdr-corral/config.default.sh" "$stage_dir/config.default.sh"
# Stage on the destination filesystem so running binaries can be replaced by rename.
mv -f -- "$stage_dir/corral" "$prefix/bin/corral"
mv -f -- "$stage_dir/corral-github" "$prefix/bin/corral-github"
mv -f -- "$stage_dir/config.default.sh" "$prefix/share/herdr-corral/config.default.sh"

printf '\nInstalled Corral %s (%s) into %s/bin\n' "$tag" "$arch" "$prefix"
printf 'Add the binary directory to your shell PATH:\n'
printf '  export PATH="%s"\n' "\$HOME/.local/bin:\$PATH"
printf '\nRegister or update the matching Herdr plugin:\n'
printf '  herdr plugin install xifan2333/herdr-corral --ref %s\n' "$tag"
printf '  herdr plugin action invoke corral.open\n'
printf '\nReopen Corral to use the installed version.\n'
