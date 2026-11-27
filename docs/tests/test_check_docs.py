"""Tests for tools/check_docs.py. Samples are assembled from fragments so this
file never contains a literal the checker would reject in a document."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import check_docs  # noqa: E402

STATUS = "## Status\n\nAccepted.\n"


def check(text: str, path: str = "PLAN.md", files: set[str] | None = None) -> list[str]:
    present = files if files is not None else {"PLAN.md", "OTHER.md"}
    return [f.reason for f in check_docs.check_document(path, text, lambda p: p in present)]


class CheckDocumentTests(unittest.TestCase):
    def test_clean_document_passes(self) -> None:
        self.assertEqual(check("# Plan\n\n" + STATUS + "See [other](OTHER.md#part).\n"), [])

    def test_requires_a_status_section_except_in_readme(self) -> None:
        self.assertEqual(check("# Plan\n"), ['missing a "## Status" section'])
        self.assertEqual(check("# Index\n", path="README.md"), [])

    def test_rejects_portuguese(self) -> None:
        line = "O " + "código " + "não compila.\n"
        self.assertIn("documentation must be in English", check(STATUS + line))

    def test_rejects_broken_relative_links_only(self) -> None:
        text = STATUS + "[a](MISSING.md) [b](https://example.com) [c](#status) [d](mailto:x@example.com)\n"
        self.assertEqual(check(text), ["broken link to MISSING.md"])

    def test_ignores_links_inside_code_fences(self) -> None:
        self.assertEqual(check(STATUS + "```\n[a](MISSING.md)\n```\n"), [])

    def test_resolves_links_relative_to_the_document(self) -> None:
        files = {"research/NOTE.md", "PLAN.md"}
        self.assertEqual(check(STATUS + "[up](../PLAN.md)\n", path="research/NOTE.md", files=files), [])

    def test_rejects_links_that_leave_the_repository(self) -> None:
        self.assertEqual(check(STATUS + "[x](../README.md)\n"), ["broken link to ../README.md"])

    def test_rejects_home_paths_and_secrets(self) -> None:
        home = "/" + "Users/someone/Library\n"
        secret = "ghp" + "_" + "a" * 36 + "\n"
        reasons = check(STATUS + home + secret)
        self.assertIn("machine-specific home path; use ~ or a placeholder", reasons)
        self.assertIn("possible secret", reasons)

    def test_validates_dates_in_file_names(self) -> None:
        self.assertEqual(check(STATUS, path="NOTE_2026-11-26.md"), [])
        self.assertEqual(check(STATUS, path="NOTE_2026-13-01.md"), ["invalid date 2026-13-01 in file name"])


if __name__ == "__main__":
    unittest.main()
