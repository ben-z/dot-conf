# dot-conf

[![CI](https://github.com/ben-z/dot-conf/actions/workflows/python-package.yml/badge.svg)](https://github.com/ben-z/dot-conf/actions/workflows/python-package.yml)

Automatically configure modular dotfiles

## Getting started

Install the package for development:

```bash
pip install -e .
```

Run the program:

```bash
dot-conf .conf.yaml
```

Run the tests:

```bash
PYTHONPATH=. pytest
```

## Usage

Configuration files are YAML documents describing which files to link. Execute
the tool with one or more configuration files:

```bash
python -m dotconf path/to/config.yaml
```

Integration tests exercise the command-line interface in addition to unit
tests. They are run along with the regular test suite.

Container-based integration tests rely on [testcontainers](https://github.com/testcontainers/testcontainers-python)
and require Docker. If Docker is unavailable these tests will fail with an error
so make sure the Docker daemon is running.

