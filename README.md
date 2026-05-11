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

- `--dry-run` preview resolved changes and path blockers without modifying files or invoking `sudo`
- `--scope all|user|system` choose which config section(s) to apply
- `--user-only` only apply `symlinks` (alias for `--scope user`)
- `--sys-only` only apply `sys_symlinks` (alias for `--scope system`)
- `-v`, `-vv` increase log verbosity; `-q`, `-qq` reduce it

## Behavior notes

- Source paths and relative `backup_directory` paths are resolved relative to the YAML file.
- Relative destination paths are resolved relative to the current working directory.
- Destination and backup paths support `~` expansion for the current user's home directory.
- Missing source files are skipped with a warning.
- Backup directories are created lazily only when an existing destination is backed up.
- Config files are all parsed before any changes are applied.
- When applying both user and system links from a non-root shell, system links are applied with `sudo` first; user links are applied only after that succeeds.
- Dry runs validate obvious destination and backup-directory blockers. System links that need `sudo` may be reported as needing elevated validation instead of hard-failing from a non-root preview.
- Applying links is not transactional; if a later link in a scope fails, earlier links in that scope may already have been applied or backed up.

## CI and release process

- Pull requests and pushes to `master` run dependency audit, formatting, clippy, and tests on Linux/macOS/Windows.
- Pull requests and pushes to `master` also build release binaries and upload them as workflow artifacts.
- Creating a tag like `v0.2.0` triggers release automation that builds per-platform archives and publishes them to GitHub Releases.

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## License

GPL-3.0-only. See [LICENSE](LICENSE).
