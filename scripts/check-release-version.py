#!/usr/bin/env python3
"""Check release identity against workspace package metadata."""
import pathlib
import re
import sys
import tomllib


def check(branch: str, root: pathlib.Path) -> None:
    match = re.fullmatch(r"release-(\d+)-(\d+)-(\d+)", branch)
    if match is None:
        raise ValueError(f"invalid release branch: {branch}")
    expected = ".".join(match.groups())
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    actual = manifest["workspace"]["package"]["version"]
    if actual != expected:
        raise ValueError(f"workspace version {actual} does not match {branch} ({expected})")
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    members = set(manifest["workspace"]["members"])
    names = {
        tomllib.loads((root / member / "Cargo.toml").read_text())["package"]["name"]
        for member in members
    }
    for package in lock["package"]:
        if package["name"] in names and "source" not in package and package["version"] != expected:
            raise ValueError(f"lock version for {package['name']} is {package['version']}, expected {expected}")
    print(f"release version: {expected}")


if __name__ == "__main__":
    try:
        check(sys.argv[1], pathlib.Path(__file__).resolve().parent.parent)
    except (ValueError, KeyError, IndexError) as error:
        sys.exit(str(error))
