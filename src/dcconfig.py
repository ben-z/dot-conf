from __future__ import annotations

import os
import shutil
import time
from collections import namedtuple
from logging import info, warning, error, critical
from pathlib import Path
from strictyaml import load as load_yaml, Map, MapPattern, Str, Int, Seq, YAMLError
from typing import NamedTuple, Dict, Union, Sequence
from .dcutils import absp


# this is a redeclaration of the schema of DCConfig.
#   https://github.com/crdoconnor/strictyaml/issues/90
YAMLSchema = Map({"backup_directory": Str(),
                  "symlinks": MapPattern(Str(), Str() | Seq(Str()))})


class DCConfig():
    config_path: Path
    backup_directory: Path
    symlinks: Dict[Path, Union[Path, Sequence[Path]]]

    def __init__(self, config_path, backup_directory, symlinks, **kwargs):
        self.config_path = absp(config_path)
        self.backup_directory = absp(backup_directory)
        self.symlinks = dict([[absp(self.config_path.parent, src), [absp(d) for d in dest] if isinstance(
            dest, list) else absp(dest)] for src, dest in symlinks.items()])

    def apply(self):
        info("Config path: {}".format(self.config_path))
        info("Creating backup directory (if it does not exist): {}".format(self.backup_directory))
        self.backup_directory.mkdir(parents=True, exist_ok=True)

        for src, dest_ in self.symlinks.items():
            dests = dest_ if isinstance(dest_, list) else [dest_]
            if not src.exists():
                info("{} does not exist, skipping".format(src))
                continue

            for dest in dests:
                info(
                    "Preparing to link {} -> {}".format(src, dest))

                if dest.exists():
                    backup_path = Path(self.backup_directory, dest.parent,
                                       "{}.{}.bak".format(dest.name, time.time()))
                    info("Backing up {} to {}".format(dest, backup_path))
                    shutil.copy(dest, backup_path)
                    dest.unlink()

                info("Linking {} -> {}".format(src, dest))
                dest.symlink_to(src)

    @staticmethod
    def from_yaml(config_path) -> DCConfig:
        with open(config_path) as config_file:
            config = load_yaml(config_file.read(), YAMLSchema)

            return DCConfig(config_path=config_path, **config.data)
