from pathlib import Path


def absp(*args, **kwargs) -> Path:
    return Path(*args, **kwargs).expanduser().resolve()
