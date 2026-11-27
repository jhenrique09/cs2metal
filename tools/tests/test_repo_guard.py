"""Tests for tools/repo_guard.py.

Sample secrets are assembled from fragments so that this file never contains a
literal that the guard itself would reject.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import repo_guard  # noqa: E402


def reasons(findings: list[repo_guard.Finding]) -> list[str]:
    return [finding.reason for finding in findings]


class NameTests(unittest.TestCase):
    def test_rejects_steam_session_files(self) -> None:
        for path in ("ssfn123456", "steam/config/loginusers.vdf", "config/config.vdf"):
            with self.subTest(path=path):
                self.assertTrue(repo_guard.scan_name(path))

    def test_rejects_game_content_and_local_output(self) -> None:
        for path in ("game/pak01_dir.vpk", "appmanifest_730.acf", "reports/inventory.md", "baselines/a.json"):
            with self.subTest(path=path):
                self.assertTrue(repo_guard.scan_name(path))

    def test_rejects_environment_files_but_allows_the_example(self) -> None:
        self.assertTrue(repo_guard.scan_name(".env"))
        self.assertTrue(repo_guard.scan_name("tools/.env.local"))
        self.assertFalse(repo_guard.scan_name(".env.example"))

    def test_allows_ordinary_sources(self) -> None:
        for path in ("crates/cs2metal-elf/src/lib.rs", "README.md", "tools/check.sh"):
            with self.subTest(path=path):
                self.assertFalse(repo_guard.scan_name(path))


class ContentTests(unittest.TestCase):
    def test_rejects_executable_formats(self) -> None:
        for data in (b"\x7fELF\x02\x01\x01", b"\xcf\xfa\xed\xfe\x07", b"MZ" + b"\0" * 62 + b"PE\0\0"):
            with self.subTest(data=data[:4]):
                self.assertTrue(repo_guard.scan_bytes("blob", data))

    def test_rejects_other_binary_content(self) -> None:
        self.assertEqual(reasons(repo_guard.scan_bytes("x.bin", b"abc\x00def")),
                         ["binary content must not be tracked"])

    def test_text_starting_with_mz_is_text(self) -> None:
        self.assertFalse(repo_guard.scan_bytes("notes.txt", b"MZ is a letter pair."))

    def test_rejects_secrets(self) -> None:
        samples = [
            "-----BEGIN " + "RSA PRIVATE KEY-----",
            "key = AKIA" + "ABCDEFGHIJKLMNOP",
            "ghp" + "_" + "a" * 36,
            "sk-ant-" + "x" * 24,
            "eyJ" + "a" * 12 + ".eyJ" + "b" * 12 + ".sig",
            "pass" + 'word = "' + "hunter2hunter2" + '"',
        ]
        for sample in samples:
            with self.subTest(sample=sample[:12]):
                self.assertTrue(repo_guard.scan_text("config.toml", sample))

    def test_rejects_home_paths_but_allows_placeholders(self) -> None:
        self.assertTrue(repo_guard.scan_text("a.md", "/" + "Users/someone/Library"))
        self.assertTrue(repo_guard.scan_text("a.sh", "/" + "home/someone/.steam"))
        for line in ("~/Library/Caches", "/Users/<user>/Library", "$HOME/x", "/home/$USER/.steam"):
            with self.subTest(line=line):
                self.assertFalse(repo_guard.scan_text("a.md", line))

    def test_rejects_portuguese_documentation_only(self) -> None:
        line = "Esta " + "configuração " + "não funciona."
        self.assertTrue(repo_guard.scan_text("docs.md", line))
        self.assertFalse(repo_guard.scan_text("fixture.rs", line))

    def test_accepts_english_documentation(self) -> None:
        self.assertFalse(repo_guard.scan_text("README.md", "The loader maps each module read-only."))


if __name__ == "__main__":
    unittest.main()
