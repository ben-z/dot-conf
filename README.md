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

## License

MIT

