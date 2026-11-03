#!/usr/bin/env bash
# Install Corral and dependencies according to the current Linux distribution.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: bash scripts/install.sh [OPTIONS] [VERSION]

Install Corral and dependencies according to the current Linux distribution.

Options:
  --binary       Install prebuilt GitHub Release binaries instead of using AUR (Arch Linux)
  --no-deps      Skip installing system dependencies via package manager
  -h, --help     Show this help message

Version:
  Specify a release tag (e.g. v0.1.10) or omit for the latest release.
EOF
}

install_deps=1
force_binary=0
tag=""

while (( $# > 0 )); do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --no-deps)
      install_deps=0
      shift
      ;;
    --binary)
      force_binary=1
      shift
      ;;
    -*)
      printf 'corral: unrecognized option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$tag" ]]; then
        usage >&2
        exit 2
      fi
      tag="$1"
      shift
      ;;
  esac
done

if [[ "$(uname -s)" != Linux ]]; then
  printf 'corral: this installer requires Linux\n' >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64) arch=x86_64 ;;
  aarch64) arch=aarch64 ;;
  *) printf 'corral: supported architectures are x86_64 and aarch64\n' >&2; exit 1 ;;
esac

detect_distro() {
  local os_release=""
  if [[ -f /etc/os-release ]]; then
    os_release=/etc/os-release
  elif [[ -f /usr/lib/os-release ]]; then
    os_release=/usr/lib/os-release
  fi

  distro_id=""
  distro_like=""
  if [[ -n "$os_release" ]]; then
    # shellcheck source=/dev/null
    . "$os_release"
    distro_id="${ID:-}"
    distro_like="${ID_LIKE:-}"
  fi
}

run_elevated() {
  if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  elif command -v doas >/dev/null 2>&1; then
    doas "$@"
  else
    printf 'corral: root privileges (sudo/doas) required for: %s\n' "$*" >&2
    return 1
  fi
}

print_plugin_instructions() {
  local ref_arg=""
  if [[ -n "${1:-}" ]]; then
    ref_arg=" --ref $1"
  fi
  printf '\nRegister or update the matching Herdr plugin:\n'
  printf '  herdr plugin install xifan2333/herdr-corral%s\n' "$ref_arg"
  printf '  herdr plugin action invoke corral.open\n'
  printf '\nReopen Corral to use the installed version.\n'
}

install_binary_release() {
  local repo="https://github.com/xifan2333/herdr-corral"
  local release_tag="$tag"
  if [[ -z "$release_tag" ]]; then
    local latest_url
    latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "$repo/releases/latest")"
    if [[ "$latest_url" != "$repo/releases/tag/"* ]]; then
      printf 'corral: could not resolve the latest release tag\n' >&2
      exit 1
    fi
    release_tag="${latest_url##*/}"
  fi
  release_tag="v${release_tag#v}"
  if [[ ! "$release_tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
    printf 'corral: invalid release version: %s\n' "$release_tag" >&2
    exit 2
  fi

  local version="${release_tag#v}"
  local archive="herdr-corral-${version}-${arch}.tar.xz"
  local download_dir
  download_dir="$(mktemp -d)"
  local stage_dir=""
  cleanup() {
    rm -rf -- "$download_dir"
    if [[ -n "$stage_dir" ]]; then
      rm -rf -- "$stage_dir"
    fi
  }
  trap cleanup EXIT

  local url="$repo/releases/download/$release_tag"
  printf 'Downloading Corral %s (%s)...\n' "$release_tag" "$arch"
  curl -fsSL -o "$download_dir/$archive" "$url/$archive"
  curl -fsSL -o "$download_dir/$archive.sha256" "$url/$archive.sha256"
  (
    cd "$download_dir"
    sha256sum --check "$archive.sha256"
  )
  tar -xJf "$download_dir/$archive" -C "$download_dir"
  local release_dir="$download_dir/herdr-corral-${version}-${arch}"

  local prefix="$HOME/.local"
  mkdir -p "$prefix/bin" "$prefix/share/herdr-corral"
  stage_dir="$(mktemp -d "$prefix/bin/.corral-install.XXXXXX")"
  install -m755 "$release_dir/bin/corral" "$stage_dir/corral"
  install -m755 "$release_dir/bin/corral-github" "$stage_dir/corral-github"
  install -m644 "$release_dir/share/herdr-corral/config.default.sh" "$stage_dir/config.default.sh"
  # Stage on destination filesystem so running binaries can be replaced by rename.
  mv -f -- "$stage_dir/corral" "$prefix/bin/corral"
  mv -f -- "$stage_dir/corral-github" "$prefix/bin/corral-github"
  mv -f -- "$stage_dir/config.default.sh" "$prefix/share/herdr-corral/config.default.sh"

  printf '\nInstalled Corral %s (%s) into %s/bin\n' "$release_tag" "$arch" "$prefix"
  printf 'Add the binary directory to your shell PATH:\n'
  printf '  export PATH="%s"\n' "\$HOME/.local/bin:\$PATH"
  print_plugin_instructions "$release_tag"
}

detect_distro

# 1. Arch Linux (and derivatives)
if [[ "$distro_id" == "arch" || "$distro_like" =~ (^|[[:space:]])arch([[:space:]]|$) || "$distro_id" =~ ^(manjaro|endeavouros|cachyos|garuda|artix)$ ]]; then
  if (( ! force_binary )) && [[ -z "$tag" ]]; then
    if command -v yay >/dev/null 2>&1; then
      printf 'Detected Arch Linux. Installing herdr-corral-bin via yay...\n'
      yay -S --needed herdr-corral-bin
      print_plugin_instructions ""
      exit 0
    elif command -v paru >/dev/null 2>&1; then
      printf 'Detected Arch Linux. Installing herdr-corral-bin via paru...\n'
      paru -S --needed herdr-corral-bin
      print_plugin_instructions ""
      exit 0
    fi
  fi

  printf 'Installing Corral on Arch Linux via release binaries...\n'
  if (( install_deps )); then
    if command -v pacman >/dev/null 2>&1; then
      printf 'Checking and installing dependencies via pacman...\n'
      run_elevated pacman -S --needed --noconfirm glib2 git-delta github-cli neovim imv || true
    fi
  fi
  install_binary_release
  exit 0
fi

# 2. Fedora / RHEL family
if [[ "$distro_id" == "fedora" || "$distro_like" =~ (^|[[:space:]])(fedora|rhel)([[:space:]]|$) || "$distro_id" =~ ^(rhel|centos|rocky|almalinux)$ ]]; then
  printf 'Detected Fedora / RHEL family.\n'
  if (( install_deps )); then
    pm="dnf"
    if command -v dnf5 >/dev/null 2>&1; then
      pm="dnf5"
    fi
    if command -v "$pm" >/dev/null 2>&1; then
      printf 'Installing dependencies via %s...\n' "$pm"
      run_elevated "$pm" install -y curl tar xz coreutils git bash jq gh neovim git-delta glib2 imv || true
    fi
  fi
  install_binary_release
  exit 0
fi

# 3. Debian / Ubuntu family
if [[ "$distro_id" =~ ^(debian|ubuntu|linuxmint|pop|elementary|kali)$ || "$distro_like" =~ (^|[[:space:]])(debian|ubuntu)([[:space:]]|$) ]]; then
  printf 'Detected Debian / Ubuntu family.\n'
  if (( install_deps )); then
    if command -v apt-get >/dev/null 2>&1; then
      printf 'Installing dependencies via apt-get...\n'
      run_elevated apt-get update -y || true
      run_elevated apt-get install -y curl tar xz-utils coreutils git bash jq gh neovim git-delta libglib2.0-bin imv || {
        printf 'corral: some optional packages failed to install, proceeding with core utilities...\n' >&2
        run_elevated apt-get install -y curl tar xz-utils coreutils git bash jq libglib2.0-bin || true
      }
    fi
  fi
  install_binary_release
  exit 0
fi

# 4. openSUSE
if [[ "$distro_id" =~ ^(opensuse.*|sles)$ || "$distro_like" =~ (^|[[:space:]])suse([[:space:]]|$) ]]; then
  printf 'Detected openSUSE family.\n'
  if (( install_deps )); then
    if command -v zypper >/dev/null 2>&1; then
      printf 'Installing dependencies via zypper...\n'
      run_elevated zypper install -y curl tar xz coreutils git bash jq gh neovim git-delta glib2 imv || true
    fi
  fi
  install_binary_release
  exit 0
fi

# 5. Generic Linux
printf 'Detected Linux (%s). Installing prebuilt binaries...\n' "${distro_id:-generic}"
install_binary_release
