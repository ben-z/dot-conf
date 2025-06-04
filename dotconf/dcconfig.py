from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
import logging
import shutil
import time
from pathlib import Path
from typing import Dict, Sequence, Union

from strictyaml import EmptyDict, Map, MapPattern, Optional, Seq, Str, load as load_yaml

from .dcutils import absp


class Scope(Enum):
    ALL = 0
    USER = 1
    SYS = 2


logger = logging.getLogger('dot-conf')
# this is a redeclaration of the schema of DCConfig.
#   https://github.com/crdoconnor/strictyaml/issues/90
YAMLSchema = Map({"backup_directory": Str(),
                  Optional("symlinks", default={}): MapPattern(Str(), Str() | Seq(Str())) | EmptyDict(),
                  Optional("sys_symlinks", default={}): MapPattern(Str(), Str() | Seq(Str())) | EmptyDict(),
                })

@dataclass
class DCConfig:
    """Representation of a dot-conf configuration file."""

    config_path: Path
    backup_directory: Path
    symlinks: Dict[Path, Union[Path, Sequence[Path]]] = field(default_factory=dict)
    sys_symlinks: Dict[Path, Union[Path, Sequence[Path]]] = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.config_path = absp(self.config_path)
        self.backup_directory = absp(self.backup_directory)

        self.symlinks = {
            absp(self.config_path.parent, src): [Path(d).expanduser() for d in dest] if isinstance(dest, list)
            else Path(dest).expanduser()
            for src, dest in self.symlinks.items()
        }

        self.sys_symlinks = {
            absp(self.config_path.parent, src): [Path(d).expanduser() for d in dest] if isinstance(dest, list)
            else Path(dest).expanduser()
            for src, dest in self.sys_symlinks.items()
        }

    def requires_root(self):
        return len(self.sys_symlinks) > 0

    def apply(self, scope):
        logger.info("Applying config ({}): {}".format(scope.name, self.config_path))
        if not self.backup_directory.exists():
            logger.info("Creating backup directory: {}".format(
                self.backup_directory))
            self.backup_directory.mkdir(parents=True, exist_ok=True)

        if scope is Scope.USER:
            self.apply_user_symlinks()
        elif scope is Scope.SYS:
            self.apply_sys_symlinks()
        else:
            self.apply_user_symlinks()
            self.apply_sys_symlinks()


    def apply_user_symlinks(self):
        self._apply_symlinks(self.symlinks)

    def apply_sys_symlinks(self):
        self._apply_symlinks(self.sys_symlinks)

    def _apply_symlinks(self, symlinks):
        for src, dest_ in symlinks.items():
            dests = dest_ if isinstance(dest_, list) else [dest_]
            if not src.exists():
                logger.info("{} does not exist, skipping".format(src))
                continue

            for dest in dests:
                logger.debug(
                    "Preparing to link {} -> {}".format(src, dest))

                if dest.exists() or dest.is_symlink(): # exists() returns false for orphan symlinks
                    backup_path = Path(self.backup_directory,
                                       "{}.{}.bak".format(dest.name, time.time()))
                    logger.info("Backing up {} to {}".format(dest, backup_path))
                    shutil.copy(dest, backup_path, follow_symlinks=False)
                    dest.unlink()

                logger.info("Linking {} -> {}".format(src, dest))
                dest.symlink_to(src)

    @classmethod
    def from_yaml(cls, config_path: Path) -> "DCConfig":
        """Create an instance from a YAML configuration file."""
        with open(config_path) as config_file:
            config = load_yaml(config_file.read(), YAMLSchema)

        return cls(config_path=config_path, **config.data)
