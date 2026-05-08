#!/usr/bin/env bash
set -euo pipefail

REPO="ben-z/dot-conf"
BIN_NAME="dot-conf"

os="$(uname -s)"
arch="$(uname -m)"

case "$os:$arch" in
  Linux:x86_64|Linux:amd64)
    target="x86_64-unknown-linux-gnu"
    ;;
  Darwin:x86_64|Darwin:amd64)
    target="x86_64-apple-darwin"
    ;;
  Darwin:arm64|Darwin:aarch64)
    target="aarch64-apple-darwin"
    ;;
  *)
    echo "Unsupported OS/architecture: $os/$arch"
    echo "See https://github.com/${REPO}/releases/latest for published artifacts."
    exit 1
    ;;
esac

archive="${BIN_NAME}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${archive}"
checksum_url="${url}.sha256"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

curl -fsSL "$url" -o "$tmp_dir/$archive"
curl -fsSL "$checksum_url" -o "$tmp_dir/$archive.sha256"
(
  cd "$tmp_dir"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --check "$archive.sha256"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$archive.sha256"
  else
    echo "Unable to verify archive: sha256sum or shasum is required" >&2
    exit 1
  fi
)
tar -xzf "$tmp_dir/$archive" -C "$tmp_dir"

install_dir="${DOT_CONF_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
install -m 0755 "$tmp_dir/$BIN_NAME" "$install_dir/$BIN_NAME"

echo "Installed $BIN_NAME to $install_dir/$BIN_NAME"
echo "Make sure $install_dir is in your PATH."
