"""Tests for tools/steam_shortcuts.py, on made-up Steam user data."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import steam_shortcuts  # noqa: E402

# One shortcut, as Steam writes it: a map of maps, strings, and integers.
EXISTING = (
    b"\x00shortcuts\x00"
    b"\x000\x00"
    b"\x02appid\x00\x01\x02\x03\x84"
    b"\x01AppName\x00Other Game\x00"
    b'\x01Exe\x00"/Applications/Other.app"\x00'
    b'\x01StartDir\x00"/Applications/"\x00'
    b"\x02IsHidden\x00\x00\x00\x00\x00"
    b"\x00tags\x00\x010\x00favorite\x00\x08"
    b"\x08"
    b"\x08\x08"
)


class FormatTests(unittest.TestCase):
    def test_a_file_reads_and_writes_back_byte_for_byte(self) -> None:
        entries = steam_shortcuts.parse(EXISTING)
        game = entries["shortcuts"]["0"]
        self.assertEqual(game["AppName"], "Other Game")
        self.assertEqual(game["IsHidden"], 0)
        self.assertEqual(game["appid"], -2080177663)
        self.assertEqual(game["tags"], {"0": "favorite"})
        self.assertEqual(steam_shortcuts.serialize(entries), EXISTING)

    def test_a_damaged_file_is_refused(self) -> None:
        for damaged in (EXISTING[:-1], EXISTING[:40], EXISTING + b"\x01", b"\x05x\x00\x08"):
            with self.subTest(length=len(damaged)):
                with self.assertRaises(steam_shortcuts.FormatError):
                    steam_shortcuts.parse(damaged)

    def test_adding_keeps_what_was_there(self) -> None:
        entries = steam_shortcuts.parse(EXISTING)
        self.assertTrue(steam_shortcuts.add(entries, "Counter-Strike 2", "/apps/CS2.app", "/apps/icon.icns"))
        again = steam_shortcuts.parse(steam_shortcuts.serialize(entries))
        self.assertEqual(again["shortcuts"]["0"]["AppName"], "Other Game")
        added = again["shortcuts"]["1"]
        self.assertEqual(added["AppName"], "Counter-Strike 2")
        self.assertEqual(added["Exe"], '"/apps/CS2.app"')
        self.assertEqual(added["StartDir"], '"/apps/"')
        self.assertEqual(added["icon"], "/apps/icon.icns")
        self.assertEqual(added["AllowOverlay"], 1)
        self.assertLess(added["appid"], 0)

    def test_adding_the_same_program_twice_changes_nothing(self) -> None:
        entries: dict = {}
        self.assertTrue(steam_shortcuts.add(entries, "Counter-Strike 2", "/apps/CS2.app"))
        self.assertFalse(steam_shortcuts.add(entries, "Counter-Strike 2", "/apps/CS2.app"))
        self.assertEqual(list(entries["shortcuts"]), ["0"])
        self.assertTrue(steam_shortcuts.add(entries, "Counter-Strike 2", "/apps/CS2.app", "/new.icns"))
        self.assertEqual(entries["shortcuts"]["0"]["icon"], "/new.icns")


class AccountTests(unittest.TestCase):
    def test_every_account_with_a_config_gets_the_shortcut_and_a_copy_of_its_file(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            userdata = Path(root)
            (userdata / "111" / "config").mkdir(parents=True)
            (userdata / "111" / "config" / "shortcuts.vdf").write_bytes(EXISTING)
            (userdata / "222" / "config").mkdir(parents=True)
            (userdata / "333").mkdir()
            (userdata / "0" / "config").mkdir(parents=True)
            changed = steam_shortcuts.add_to_accounts(userdata, "Counter-Strike 2", "/apps/CS2.app", "")
            self.assertEqual(changed, ["111", "222"])
            first = steam_shortcuts.parse((userdata / "111/config/shortcuts.vdf").read_bytes())
            self.assertEqual(sorted(first["shortcuts"]), ["0", "1"])
            self.assertEqual(
                (userdata / "111/config/shortcuts.vdf.before-cs2metal").read_bytes(), EXISTING
            )
            second = steam_shortcuts.parse((userdata / "222/config/shortcuts.vdf").read_bytes())
            self.assertEqual(second["shortcuts"]["0"]["AppName"], "Counter-Strike 2")
            self.assertFalse((userdata / "222/config/shortcuts.vdf.before-cs2metal").exists())
            self.assertFalse((userdata / "0/config/shortcuts.vdf").exists())
            self.assertEqual(
                steam_shortcuts.add_to_accounts(userdata, "Counter-Strike 2", "/apps/CS2.app", ""), []
            )

    def test_a_damaged_file_is_left_alone(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            config = Path(root) / "111" / "config"
            config.mkdir(parents=True)
            (config / "shortcuts.vdf").write_bytes(EXISTING[:30])
            with self.assertRaises(steam_shortcuts.FormatError):
                steam_shortcuts.add_to_accounts(Path(root), "Counter-Strike 2", "/apps/CS2.app", "")
            self.assertEqual((config / "shortcuts.vdf").read_bytes(), EXISTING[:30])


if __name__ == "__main__":
    unittest.main()
