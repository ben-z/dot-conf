# dot-conf

`dot-conf` is a small, elegant Rust CLI for managing dotfiles from a YAML config.

## Why this version

- **Simple mental model:** map `source -> destination` symlinks.
- **Safe updates:** existing destination files are backed up before replacement.
- **Flexible mapping:** one source can target many destinations.
- **Intuitive tooling:** install from prebuilt binaries or `cargo`.

## Install (non-dev)

### Option 1: one-line installer (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/ben-z/dot-conf/master/scripts/install.sh | bash
```

By default, this installs to `~/.local/bin/dot-conf`. Set `DOT_CONF_INSTALL_DIR` to override.

### Option 2: download prebuilt binary from GitHub Releases

1. Go to: <https://github.com/ben-z/dot-conf/releases/latest>
2. Download the archive for your platform:
   - `dot-conf-x86_64-unknown-linux-gnu.tar.gz`
   - `dot-conf-x86_64-apple-darwin.tar.gz`
   - `dot-conf-aarch64-apple-darwin.tar.gz`
   - `dot-conf-x86_64-pc-windows-msvc.zip`
3. Extract and place `dot-conf` (or `dot-conf.exe`) into a directory on your `PATH`.

### Option 3: install with Cargo

```bash
cargo install --git https://github.com/ben-z/dot-conf
```

## Quick start

```yaml
backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
  .tmux.conf:
    - ~/.tmux.conf
    - ~/.config/tmux/tmux.conf
sys_symlinks:
  .sysrc: /etc/sysrc
```

Then run:

```bash
dot-conf config.yaml
```

Options:

- `--user-only` only apply `symlinks`
- `--sys-only` only apply `sys_symlinks`

## Behavior notes

- Source paths are resolved relative to the YAML file.
- Destination paths support `~` expansion.
- Missing source files are skipped (no error).

## CI and release process

- Pull requests and pushes to `master` run formatting, clippy, and tests on Linux/macOS/Windows.
- Pull requests and pushes to `master` also build release binaries and upload them as workflow artifacts.
- Creating a tag like `v0.2.0` triggers release automation that builds per-platform archives and publishes them to GitHub Releases.

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
