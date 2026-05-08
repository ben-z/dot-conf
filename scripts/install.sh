#!/usr/bin/env bash
set -euo pipefail

REPO="ben-z/dot-conf"
BIN_NAME="dot-conf"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Linux) os_part="unknown-linux-gnu" ;;
  Darwin) os_part="apple-darwin" ;;
  *) echo "Unsupported OS: $os"; exit 1 ;;
esac

case "$arch" in
  x86_64|amd64) arch_part="x86_64" ;;
  arm64|aarch64) arch_part="aarch64" ;;
  *) echo "Unsupported architecture: $arch"; exit 1 ;;
esac

target="${arch_part}-${os_part}"
archive="${BIN_NAME}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${archive}"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

curl -fsSL "$url" -o "$tmp_dir/$archive"
tar -xzf "$tmp_dir/$archive" -C "$tmp_dir"

install_dir="${DOT_CONF_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
install -m 0755 "$tmp_dir/$BIN_NAME" "$install_dir/$BIN_NAME"

echo "Installed $BIN_NAME to $install_dir/$BIN_NAME"
echo "Make sure $install_dir is in your PATH."
