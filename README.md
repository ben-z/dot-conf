# dot-conf

[![Tests](https://github.com/ben-z/dot-conf/actions/workflows/tests.yml/badge.svg)](https://github.com/ben-z/dot-conf/actions/workflows/tests.yml)

Automatically configure modular dotfiles

## Features

- Simple YAML-based configuration
- Support for symlinks and file copying
- Backup of existing dotfiles
- Cross-platform support

## Installation

### Install using pip

```bash
pip install git+https://github.com/ben-z/dot-conf.git
```

## Development

1. Clone the repository:
   ```bash
   git clone https://github.com/ben-z/dot-conf.git
   cd dot-conf
   ```

2. Install in development mode with test dependencies:
   ```bash
   pip install -e '.[test]'
   ```

## Usage

Create a configuration file (e.g., `.conf.yaml`) and run:

```bash
dot-conf .conf.yaml
```

## Configuration

Example configuration:

```yaml
backup_directory: ~/.dotfiles/backup
symlinks:
  ~/.vimrc: ~/dotfiles/vimrc
  ~/.gitconfig: ~/dotfiles/gitconfig
```

## Development

Run tests:

```bash
python -m unittest discover -s tests -p '*_test.py'
```

## Troubleshooting

If installation produces a package named `UNKNOWN` or the `dot-conf` command is missing, this usually means your version of pip or setuptools is too old and does not respect `pyproject.toml` builds.

To check your setuptools version, run:
```bash
python3 -m pip show setuptools
```
Setuptools must be at least version 61.0, and pip should be modern (version 23 or later).

To upgrade pip and setuptools to appropriate versions, run:
```bash
python3 -m pip install "setuptools>=61" "pip>=23" wheel
```

## License

MIT

