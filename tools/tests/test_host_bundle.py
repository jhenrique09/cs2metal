"""Tests for tools/host-bundle.sh, with a stand-in for the host program."""

from __future__ import annotations

import os
import plistlib
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "host-bundle.sh"


class HostBundleTests(unittest.TestCase):
    def bundle(self, root: Path, *extra: str) -> tuple[Path, subprocess.CompletedProcess[str]]:
        program = root / "built"
        program.write_text("#!/bin/sh\necho host \"$@\"\n")
        app = root / "bin" / "cs2metal.app"
        done = subprocess.run(
            [str(SCRIPT), str(app), str(program), *extra],
            capture_output=True,
            text=True,
            check=False,
        )
        return app, done

    def test_the_bundle_declares_a_game_that_supports_game_mode(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            app, done = self.bundle(Path(root), "abc1234")
            self.assertEqual(done.returncode, 0, done.stderr)
            with (app / "Contents" / "Info.plist").open("rb") as file:
                info = plistlib.load(file)
            self.assertEqual(info["CFBundleExecutable"], "cs2metal")
            self.assertEqual(info["CFBundlePackageType"], "APPL")
            self.assertEqual(info["CFBundleIdentifier"], "local.cs2metal.host")
            self.assertEqual(info["CFBundleShortVersionString"], "abc1234")
            self.assertTrue(info["LSApplicationCategoryType"].endswith("-games"))
            self.assertIs(info["LSSupportsGameMode"], True)
            self.assertIs(info["GCSupportsGameMode"], True)
            self.assertIn("voice chat", info["NSMicrophoneUsageDescription"])

    def test_the_program_runs_from_inside_the_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            app, _ = self.bundle(Path(root))
            program = app / "Contents" / "MacOS" / "cs2metal"
            self.assertTrue(os.access(program, os.X_OK))
            out = subprocess.run([str(program), "run"], capture_output=True, text=True, check=True)
            self.assertEqual(out.stdout, "host run\n")
            self.assertEqual(sorted(p.name for p in program.parent.iterdir()), ["cs2metal"])

    def test_a_second_run_replaces_the_program_and_keeps_an_icon(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            app, _ = self.bundle(Path(root))
            icon = app / "Contents" / "Resources" / "icon.icns"
            icon.write_bytes(b"icon")
            (Path(root) / "built").write_text("#!/bin/sh\necho newer\n")
            done = subprocess.run(
                [str(SCRIPT), str(app), str(Path(root) / "built")],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(done.returncode, 0, done.stderr)
            program = app / "Contents" / "MacOS" / "cs2metal"
            out = subprocess.run([str(program)], capture_output=True, text=True, check=True)
            self.assertEqual(out.stdout, "newer\n")
            self.assertEqual(icon.read_bytes(), b"icon")

    def test_a_missing_program_is_an_error_and_makes_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            app = Path(root) / "cs2metal.app"
            done = subprocess.run(
                [str(SCRIPT), str(app), str(Path(root) / "absent")],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(done.returncode, 1)
            self.assertFalse(app.exists())


if __name__ == "__main__":
    unittest.main()
