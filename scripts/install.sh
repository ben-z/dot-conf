#!/usr/bin/env bash
set -euo pipefail

REPO="ben-z/dot-conf"
BIN_NAME="dot-conf"

for tool in uname curl tar mktemp install mkdir mv rm; do
  command -v "$tool" >/dev/null || { echo "Required command not found: $tool" >&2; exit 1; }
done
if command -v sha256sum >/dev/null; then
  checksum=(sha256sum --check)
elif command -v shasum >/dev/null; then
  checksum=(shasum -a 256 -c)
else
  echo "Required command not found: sha256sum or shasum" >&2
  exit 1
fi

os="$(uname -s)"
arch="$(uname -m)"

case "$os:$arch" in
  Linux:x86_64|Linux:amd64)
    target="x86_64-unknown-linux-musl"
    ;;
  Linux:aarch64|Linux:arm64)
    target="aarch64-unknown-linux-musl"
    ;;
  Darwin:x86_64|Darwin:amd64)
    target="x86_64-apple-darwin"
    ;;
  Darwin:arm64|Darwin:aarch64)
    target="aarch64-apple-darwin"
    ;;
  *)
    echo "Unsupported OS/architecture: $os/$arch" >&2
    exit 1
    ;;
esac

tmp_dir="$(mktemp -d)"
staged_binary=""
trap 'rm -rf -- "${tmp_dir:?}"; if [[ -n "$staged_binary" ]]; then rm -f -- "${staged_binary:?}"; fi' EXIT

archive="${BIN_NAME}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${archive}"
curl -fsSL "$url" -o "$tmp_dir/$archive"
curl -fsSL "$url.sha256" -o "$tmp_dir/$archive.sha256"
(
  cd "$tmp_dir"
  "${checksum[@]}" "$archive.sha256"
)
tar -xzf "$tmp_dir/$archive" -C "$tmp_dir" "$BIN_NAME"
if [[ ! -f "$tmp_dir/$BIN_NAME" || ! -x "$tmp_dir/$BIN_NAME" || -L "$tmp_dir/$BIN_NAME" ]]; then
  echo "Archive must contain an executable file named $BIN_NAME" >&2
  exit 1
fi

install_dir="${DOT_CONF_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
if [[ -d "$install_dir/$BIN_NAME" ]]; then
  echo "Install destination is a directory: $install_dir/$BIN_NAME" >&2
  exit 1
fi
staged_binary="$(mktemp "$install_dir/.dot-conf.XXXXXXXX")"
install -m 0755 "$tmp_dir/$BIN_NAME" "$staged_binary"
mv -f -- "$staged_binary" "$install_dir/$BIN_NAME"

echo "Installed $BIN_NAME to $install_dir/$BIN_NAME"
echo "Make sure $install_dir is in your PATH."
