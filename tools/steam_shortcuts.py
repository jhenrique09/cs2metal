#!/usr/bin/env python3
"""Adds a program to Steam's list of non-Steam games.

Steam keeps that list per account in `userdata/<account>/config/shortcuts.vdf`, in
its binary key-value format: a nested map of strings and 32-bit integers. This
reads the file, adds one entry or brings the one for the same program up to date,
and writes it back, keeping a copy of the file as it was. Steam reads the file
when it starts and writes it when it quits, so Steam must not be running.

    tools/steam_shortcuts.py add --name NAME --exe PATH [--icon PATH] [--userdata DIR]

Exit status 3: Steam is running, and nothing was changed.
"""

from __future__ import annotations

import argparse
import binascii
import os
import struct
import subprocess
import sys
from pathlib import Path

MAP, STRING, INT, END = 0x00, 0x01, 0x02, 0x08
USERDATA = Path.home() / "Library/Application Support/Steam/userdata"


class FormatError(ValueError):
    """The file is not a binary key-value file this tool understands."""


def _text(data: bytes, at: int) -> tuple[str, int]:
    end = data.find(b"\x00", at)
    if end < 0:
        raise FormatError("a string runs past the end of the file")
    return data[at:end].decode("utf-8", errors="surrogateescape"), end + 1


def _read_map(data: bytes, at: int) -> tuple[dict, int]:
    entries: dict = {}
    while True:
        if at >= len(data):
            raise FormatError("a map runs past the end of the file")
        kind = data[at]
        at += 1
        if kind == END:
            return entries, at
        key, at = _text(data, at)
        if kind == MAP:
            entries[key], at = _read_map(data, at)
        elif kind == STRING:
            entries[key], at = _text(data, at)
        elif kind == INT:
            if at + 4 > len(data):
                raise FormatError("an integer runs past the end of the file")
            entries[key] = struct.unpack_from("<i", data, at)[0]
            at += 4
        else:
            raise FormatError(f"unknown value type {kind:#x}")


def parse(data: bytes) -> dict:
    """The file's top-level map: normally one key, `shortcuts`."""
    entries, at = _read_map(data, 0)
    if at != len(data):
        raise FormatError("data after the end of the top-level map")
    return entries


def _write_map(entries: dict) -> bytes:
    out = bytearray()
    for key, value in entries.items():
        name = key.encode("utf-8", errors="surrogateescape") + b"\x00"
        if isinstance(value, dict):
            out += bytes([MAP]) + name + _write_map(value)
        elif isinstance(value, str):
            out += bytes([STRING]) + name
            out += value.encode("utf-8", errors="surrogateescape") + b"\x00"
        elif isinstance(value, int):
            out += bytes([INT]) + name + struct.pack("<i", value)
        else:
            raise FormatError(f"cannot write a value of type {type(value).__name__}")
    out.append(END)
    return bytes(out)


def serialize(entries: dict) -> bytes:
    return _write_map(entries)


def shortcut_id(name: str, exe: str) -> int:
    """The id Steam's own clients have derived for a shortcut: a CRC with the top bit set."""
    crc = binascii.crc32((exe + name).encode("utf-8")) | 0x80000000
    return crc - (1 << 32) if crc & 0x80000000 else crc


def _field(entry: dict, name: str):
    for key, value in entry.items():
        if key.lower() == name.lower():
            return value
    return None


def add(entries: dict, name: str, exe: str, icon: str = "") -> bool:
    """Adds the program, or updates the entry that already names it. True if anything changed."""
    key = next((k for k in entries if k.lower() == "shortcuts"), "shortcuts")
    shortcuts = entries.setdefault(key, {})
    quoted = f'"{exe}"'
    wanted = {
        "appid": shortcut_id(name, quoted),
        "AppName": name,
        "Exe": quoted,
        "StartDir": f'"{os.path.dirname(exe)}/"',
        "icon": icon,
        "ShortcutPath": "",
        "LaunchOptions": "",
        "IsHidden": 0,
        "AllowDesktopConfig": 1,
        "AllowOverlay": 1,
        "OpenVR": 0,
        "Devkit": 0,
        "DevkitGameID": "",
        "DevkitOverrideAppID": 0,
        "LastPlayTime": 0,
        "FlatpakAppID": "",
        "tags": {},
    }
    for entry in shortcuts.values():
        if isinstance(entry, dict) and _field(entry, "Exe") == quoted:
            changed = False
            for field in ("AppName", "icon"):
                current = next((k for k in entry if k.lower() == field.lower()), field)
                if entry.get(current) != wanted[field]:
                    entry[current] = wanted[field]
                    changed = True
            return changed
    index = 0
    while str(index) in shortcuts:
        index += 1
    shortcuts[str(index)] = wanted
    return True


def steam_is_running() -> bool:
    found = subprocess.run(
        ["pgrep", "-f", "Steam.AppBundle/Steam/Contents/MacOS/steam_osx"],
        capture_output=True,
        check=False,
    )
    return found.returncode == 0


def add_to_accounts(userdata: Path, name: str, exe: str, icon: str) -> list[str]:
    """Adds the program for every account directory. Returns the accounts changed."""
    changed = []
    for account in sorted(userdata.iterdir()) if userdata.is_dir() else []:
        if not account.name.isdigit() or account.name == "0":
            continue
        config = account / "config"
        if not config.is_dir():
            continue
        file = config / "shortcuts.vdf"
        entries = parse(file.read_bytes()) if file.exists() else {}
        if not add(entries, name, exe, icon):
            continue
        if file.exists():
            backup = file.with_name("shortcuts.vdf.before-cs2metal")
            if not backup.exists():
                backup.write_bytes(file.read_bytes())
        new = file.with_name("shortcuts.vdf.cs2metal-new")
        new.write_bytes(serialize(entries))
        new.replace(file)
        changed.append(account.name)
    return changed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    adding = commands.add_parser("add", help="add a program to every account's list")
    adding.add_argument("--name", required=True)
    adding.add_argument("--exe", required=True)
    adding.add_argument("--icon", default="")
    adding.add_argument("--userdata", type=Path, default=USERDATA)
    adding.add_argument("--even-if-running", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if steam_is_running() and not args.even_if_running:
        print("steam-shortcuts: Steam is running; quit it first, nothing was changed", file=sys.stderr)
        return 3
    try:
        changed = add_to_accounts(args.userdata, args.name, args.exe, args.icon)
    except FormatError as error:
        print(f"steam-shortcuts: {error}; nothing was changed", file=sys.stderr)
        return 1
    if changed:
        print(f"steam-shortcuts: added {args.name!r} for account directories {', '.join(changed)}")
    else:
        print(f"steam-shortcuts: {args.name!r} is already in Steam's list, or no account was found")
    return 0


if __name__ == "__main__":
    sys.exit(main())
