# dot-conf

`dot-conf` creates symlinks from a YAML file and backs up anything it replaces.
It runs with your shell's permissions, stays quiet on success, and exits nonzero on errors.

## Install (non-dev)

### Option 1: one-line installer (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/ben-z/dot-conf/master/scripts/install.sh | bash
```

The installer supports x86_64 and ARM64 macOS/Linux machines. By default, this installs to `~/.local/bin/dot-conf`. Set `DOT_CONF_INSTALL_DIR` to override.

### Option 2: download prebuilt binary from GitHub Releases

1. Go to: <https://github.com/ben-z/dot-conf/releases/latest>
2. Download the archive for your platform:
   - `dot-conf-x86_64-unknown-linux-musl.tar.gz`
   - `dot-conf-aarch64-unknown-linux-musl.tar.gz`
   - `dot-conf-x86_64-apple-darwin.tar.gz`
   - `dot-conf-aarch64-apple-darwin.tar.gz`
   - `dot-conf-x86_64-pc-windows-msvc.zip`
3. Extract and place `dot-conf` (or `dot-conf.exe`) into a directory on your `PATH`.

The Linux archives are statically linked musl builds so they work across common glibc- and musl-based distributions, including older LTS releases.

### Option 3: install with Cargo

```bash
cargo install --locked --git https://github.com/ben-z/dot-conf
```

## Quick start

```yaml
backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
  .tmux.conf:
    - ~/.tmux.conf
    - ~/.config/tmux/tmux.conf
  jupyter_notebook_config.py:
    destinations: ~/.jupyter/jupyter_notebook_config.py
    hosts:
      - work-laptop
      - lab-workstation
sys_symlinks:
  .sysrc: /etc/sysrc
```

Preview and apply user links:

```bash
dot-conf --dry-run --scope user config.yaml
dot-conf --scope user config.yaml
```

Apply both sections with `dot-conf config.yaml` when both destinations are writable.
For privileged destinations, run the system section separately using your shell's
privilege tool. Use an absolute backup directory writable by that user:

```bash
sudo dot-conf --scope system /absolute/path/config.yaml
```

`~` refers to the invoking user's home directory. Use absolute destination and
backup paths in system configs so their meaning stays the same under `sudo`.

Options:

- `--version`, `-V` print the installed `dot-conf` version
- `--dry-run` preview resolved changes and path blockers without modifying files
- `--scope all|user|system` choose which config section(s) to apply
- `--user-only` only apply `symlinks` (alias for `--scope user`)
- `--sys-only` only apply `sys_symlinks` (alias for `--scope system`)
- `-v` report created links on stderr; `-vv` enable debug logs; `-q`, `-qq` reduce logs

## Host matching

Use your system hostname in a `host` or `hosts` filter when a link should only apply on some machines. On most systems, `hostname` prints the full name and `hostname -s` prints the short name.

```yaml
symlinks:
  jupyter_notebook_config.py:
    destinations: ~/.jupyter/jupyter_notebook_config.py
    hosts:
      - work-laptop
      - lab-workstation
```

`dot-conf` reads the current hostname from the operating system and compares configured hosts case-insensitively. A short configured host such as `work-laptop` matches either `work-laptop` or a full current hostname such as `work-laptop.example.com`. A configured fully-qualified host such as `work-laptop.example.com` requires the operating system to report that exact full hostname, which avoids guessing when two domains share the same short host name.

## Behavior notes

- Source paths and relative `backup_directory` paths are resolved relative to the YAML file.
- Relative destination paths are resolved relative to the current working directory.
- Destination and backup paths support `~` expansion for the current user's home directory.
- A symlink entry can use `destinations` plus `host` or `hosts` to apply only on matching hostnames.
- Missing sources, empty paths, empty destination lists, and blank hostnames are errors. Use host filters for machine-specific sources.
- On macOS and Windows, case and Unicode aliases are rejected conservatively, including on case-sensitive volumes.
- Windows paths must use long filenames without trailing dots or spaces.
- A directory cannot be linked inside itself.
- Re-running an already-correct link leaves it in place without creating a backup.
- Backup directories are created lazily only when an existing destination is backed up.
- Backup filenames include the destination name, a readable UTC timestamp, and a destination hash.
- All selected links in all input configs are checked before any changes are applied.
- Duplicate source keys, aliased or overlapping destinations, and destinations containing configured sources or backup directories are rejected.
- System links are applied before user links across all input configs. The caller supplies any elevated privileges.
- Dry runs use the same filesystem checks as apply and fail on blockers under the current credentials.
- Applying links is not transactional. Runtime failures or concurrent filesystem changes may leave earlier links applied or backed up. Backing up directories requires the backup directory to be on the same filesystem; regular files can be copied across filesystems.

## CI and release process

- Pull requests and pushes to `master` run dependency audit, formatting, clippy, and tests on Linux/macOS/Windows.
- Pull requests and pushes to `master` also build release binaries and upload them as workflow artifacts.
- Creating a version tag like `v0.1.1` triggers release automation that builds per-platform archives and publishes them to GitHub Releases.

## Development

```bash
cargo fmt
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo audit --deny warnings
```

The test suite runs the CLI against isolated home directories, checks that failed
plans leave every file unchanged, and exercises installation with local release
fixtures. CI runs these checks on Linux, macOS, and Windows; installer tests run
on Linux and macOS. Install `cargo-audit` with `cargo install cargo-audit --locked`
to run the dependency check locally.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
