# dot-conf

`dot-conf` is a small, elegant Rust CLI for managing dotfiles from a YAML config.

## Why this version

- **Simple mental model:** map `source -> destination` symlinks.
- **Safe updates:** existing destination files are backed up before replacement.
- **Flexible mapping:** one source can target many destinations.
- **Intuitive tooling:** `cargo build`, `cargo run`, `cargo test`.

## Install

```bash
cargo install --path .
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
# or
cargo run -- config.yaml
```

Options:

- `--user-only` only apply `symlinks`
- `--sys-only` only apply `sys_symlinks`

## Behavior notes

- Source paths are resolved relative to the YAML file.
- Destination paths support `~` expansion.
- Missing source files are skipped (no error).

## Development

```bash
cargo fmt
cargo test
```
