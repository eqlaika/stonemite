#!/usr/bin/env python3
"""Manage Stonemite's canonical YYYY.MM.DD[.N] release version."""

from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VERSION_FILE = ROOT / "VERSION"
CARGO_MANIFEST = ROOT / "crates" / "stonemite" / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
TAURI_CONFIG = ROOT / "crates" / "stonemite" / "tauri.conf.json"
MAX_DAILY_REVISION = 99
PUBLIC_PATTERN = re.compile(
    rf"^(?P<year>\d{{4}})\.(?P<month>\d{{2}})\.(?P<day>\d{{2}})"
    rf"(?:\.(?P<revision>[1-9]\d?))?$"
)


@dataclass(frozen=True, order=True)
class CalVer:
    year: int
    month: int
    day: int
    revision: int = 0

    @classmethod
    def parse(cls, value: str) -> "CalVer":
        match = PUBLIC_PATTERN.fullmatch(value)
        if not match:
            raise ValueError(
                f"invalid Stonemite version {value!r}; expected YYYY.MM.DD or "
                f"YYYY.MM.DD.N (N=1-{MAX_DAILY_REVISION})"
            )
        result = cls(
            int(match["year"]),
            int(match["month"]),
            int(match["day"]),
            int(match["revision"] or 0),
        )
        try:
            dt.date(result.year, result.month, result.day)
        except ValueError as error:
            raise ValueError(f"invalid Stonemite version {value!r}: {error}") from error
        return result

    @property
    def public(self) -> str:
        base = f"{self.year:04d}.{self.month:02d}.{self.day:02d}"
        return f"{base}.{self.revision}" if self.revision else base

    @property
    def cargo(self) -> str:
        # Pack DD and the daily revision as DDNN. This preserves CalVer order
        # while satisfying Cargo/Tauri's three-component SemVer requirement.
        return f"{self.year}.{self.month}.{self.day * 100 + self.revision}"


def read_public_version() -> CalVer:
    return CalVer.parse(VERSION_FILE.read_text(encoding="utf-8").strip())


def replace_once(path: Path, pattern: re.Pattern[str], replacement: str) -> None:
    contents = path.read_text(encoding="utf-8")
    updated, count = pattern.subn(replacement, contents, count=1)
    if count != 1:
        raise RuntimeError(f"could not find exactly one version field in {path.relative_to(ROOT)}")
    path.write_text(updated, encoding="utf-8", newline="")


def manifest_version() -> str:
    contents = CARGO_MANIFEST.read_text(encoding="utf-8")
    match = re.search(
        r'(?ms)^\[package\]\s*$.*?^name = "stonemite"\s*$.*?^version = "([^"]+)"\s*$',
        contents,
    )
    if not match:
        raise RuntimeError("could not find the Stonemite package version")
    return match.group(1)


def lock_version() -> str:
    contents = CARGO_LOCK.read_text(encoding="utf-8")
    match = re.search(
        r'(?ms)^\[\[package\]\]\s*$\s*^name = "stonemite"\s*$\s*^version = "([^"]+)"\s*$',
        contents,
    )
    if not match:
        raise RuntimeError("could not find the Stonemite Cargo.lock version")
    return match.group(1)


def tauri_version() -> str:
    contents = TAURI_CONFIG.read_text(encoding="utf-8")
    match = re.search(r'^  "version": "([^"]+)",$', contents, flags=re.MULTILINE)
    if not match:
        raise RuntimeError("could not find the Tauri version")
    return match.group(1)


def check() -> CalVer:
    version = read_public_version()
    expected = version.cargo
    actual = {
        str(CARGO_MANIFEST.relative_to(ROOT)): manifest_version(),
        str(CARGO_LOCK.relative_to(ROOT)): lock_version(),
        str(TAURI_CONFIG.relative_to(ROOT)): tauri_version(),
    }
    mismatches = [
        f"{path}: expected {expected}, found {found}"
        for path, found in actual.items()
        if found != expected
    ]
    if mismatches:
        raise RuntimeError("version files are out of sync:\n  " + "\n  ".join(mismatches))
    return version


def set_version(raw_version: str) -> CalVer:
    version = CalVer.parse(raw_version)
    internal = version.cargo

    VERSION_FILE.write_text(version.public + "\n", encoding="utf-8", newline="")
    replace_once(
        CARGO_MANIFEST,
        re.compile(
            r'(?m)(?<=^\[package\]\nname = "stonemite"\n)version = "[^"]+"$'
        ),
        f'version = "{internal}"',
    )
    replace_once(
        CARGO_LOCK,
        re.compile(
            r'(?m)(?<=^\[\[package\]\]\nname = "stonemite"\n)version = "[^"]+"$'
        ),
        f'version = "{internal}"',
    )
    replace_once(
        TAURI_CONFIG,
        re.compile(r'(?m)^  "version": "[^"]+",$'),
        f'  "version": "{internal}",',
    )
    check()
    return version


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("get", help="print the canonical public version")
    subparsers.add_parser("cargo", help="print the internal Cargo/Tauri version")
    subparsers.add_parser("check", help="verify every version source is synchronized")
    set_parser = subparsers.add_parser("set", help="set and synchronize a public version")
    set_parser.add_argument("version")
    args = parser.parse_args()

    try:
        if args.command == "set":
            version = set_version(args.version)
        else:
            version = check()
        print(version.cargo if args.command == "cargo" else version.public)
        return 0
    except (OSError, RuntimeError, ValueError) as error:
        print(f"version error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
