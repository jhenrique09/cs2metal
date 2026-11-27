"""Tests for tools/low-preset.sh, on made-up Steam user data directories."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "low-preset.sh"

EXISTING = (
    '"video.cfg"\n'
    "{\n"
    '\t"Version"\t\t"17"\n'
    '\t"setting.defaultres"\t\t"2560"\n'
    '\t"setting.shaderquality"\t\t"1"\n'
    '\t"setting.msaa_samples"\t\t"4"\n'
    '\t"setting.r_csgo_cmaa_enable"\t\t"0"\n'
    '\t"setting.monitor_index"\t\t"0"\n'
    "}\n"
)


def settings(text: str) -> dict[str, str]:
    found = {}
    for line in text.splitlines():
        parts = [part for part in line.split('"') if part.strip()]
        if len(parts) == 2:
            found[parts[0]] = parts[1]
    return found


class LowPresetTests(unittest.TestCase):
    def run_script(self, userdata: Path, *ids: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(SCRIPT), str(userdata), *ids], capture_output=True, text=True, check=False
        )

    def test_an_existing_file_keeps_everything_but_the_quality(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            cfg = Path(root) / "1234" / "730" / "local" / "cfg"
            cfg.mkdir(parents=True)
            (cfg / "cs2_video.txt").write_text(EXISTING)
            self.assertEqual(self.run_script(Path(root)).returncode, 0)
            text = (cfg / "cs2_video.txt").read_text()
            got = settings(text)
            self.assertEqual(got["Version"], "17")
            self.assertEqual(got["setting.defaultres"], "2560")
            self.assertEqual(got["setting.monitor_index"], "0")
            self.assertEqual(got["setting.shaderquality"], "0")
            # Multisampling off, the cheap CMAA2 pass on.
            self.assertEqual(got["setting.msaa_samples"], "0")
            self.assertEqual(got["setting.r_csgo_cmaa_enable"], "1")
            self.assertEqual(got["setting.videocfg_shadow_quality"], "0")
            self.assertEqual(got["setting.videocfg_fsr_detail"], "3")
            self.assertTrue(text.startswith('"video.cfg"\n{\n'))
            self.assertTrue(text.endswith("}\n"))
            self.assertEqual((cfg / "cs2_video.txt.before-cs2metal").read_text(), EXISTING)

    def test_the_first_copy_is_kept_when_run_again(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            cfg = Path(root) / "1234" / "730" / "local" / "cfg"
            cfg.mkdir(parents=True)
            (cfg / "cs2_video.txt").write_text(EXISTING)
            self.run_script(Path(root))
            once = (cfg / "cs2_video.txt").read_text()
            self.run_script(Path(root))
            self.assertEqual((cfg / "cs2_video.txt").read_text(), once)
            self.assertEqual((cfg / "cs2_video.txt.before-cs2metal").read_text(), EXISTING)

    def test_an_account_without_the_file_gets_one_that_names_the_device(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "5678").mkdir()
            (Path(root) / "0").mkdir()
            (Path(root) / "anonymous").mkdir()
            done = self.run_script(Path(root), "4203", "99")
            self.assertEqual(done.returncode, 0)
            file = Path(root) / "5678" / "730" / "local" / "cfg" / "cs2_video.txt"
            got = settings(file.read_text())
            self.assertEqual(got["Version"], "17")
            self.assertEqual(got["VendorID"], "4203")
            self.assertEqual(got["DeviceID"], "99")
            self.assertEqual(got["setting.shaderquality"], "0")
            self.assertEqual(got["setting.r_csgo_cmaa_enable"], "1")
            self.assertEqual(got["setting.videocfg_hdr_detail"], "3")
            self.assertEqual(got["setting.coop_fullscreen"], "1")
            self.assertEqual(got["setting.nowindowborder"], "1")
            self.assertEqual(got["setting.fullscreen"], "0")
            self.assertEqual(len(got), 18)
            self.assertNotIn("setting.defaultres", got)
            self.assertFalse((file.parent / "cs2_video.txt.before-cs2metal").exists())
            self.assertFalse((Path(root) / "0" / "730").exists())
            self.assertFalse((Path(root) / "anonymous" / "730").exists())
            self.assertIn("5678", done.stdout)

    def test_without_device_ids_no_file_is_made(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "5678").mkdir()
            done = self.run_script(Path(root))
            self.assertEqual(done.returncode, 3)
            self.assertFalse((Path(root) / "5678" / "730").exists())

    def test_one_account_with_the_file_is_enough_without_ids(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            cfg = Path(root) / "1234" / "730" / "local" / "cfg"
            cfg.mkdir(parents=True)
            (cfg / "cs2_video.txt").write_text(EXISTING)
            (Path(root) / "5678").mkdir()
            done = self.run_script(Path(root))
            self.assertEqual(done.returncode, 0)
            self.assertFalse((Path(root) / "5678" / "730").exists())

    def test_no_user_data_is_no_error(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            done = self.run_script(Path(root) / "missing")
            self.assertEqual((done.returncode, done.stdout), (0, ""))


if __name__ == "__main__":
    unittest.main()
