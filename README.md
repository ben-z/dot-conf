# dot-conf

[![Build Status](https://travis-ci.com/ben-z/dot-conf.svg?token=XoxzU5ytmnXGRFUMpScC&branch=master)](https://travis-ci.com/ben-z/dot-conf)

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

