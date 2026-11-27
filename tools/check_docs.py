#!/usr/bin/env python3
"""Check the cs2metal documentation repository before every commit.

Rules:
- English only in tracked documents.
- Relative Markdown links must resolve inside the repository.
- No machine-specific home paths, no secrets.
- Every document other than README.md has a "## Status" section.
- Dated documents (NAME_YYYY-MM-DD.md) carry a real calendar date.
"""

from __future__ import annotations

import argparse
import datetime
import posixpath
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

PORTUGUESE_PROSE = re.compile(
    r"\b(?:não|também|você|então|está|são|código|configuração|aplicação|"
    r"documentação|execução|instalação|versão|conexão|usuário|diretório|"
    r"arquivos?|segurança|requisitos|objetivo|portanto|através)\b",
    re.IGNORECASE,
)
HOME_PATH = re.compile(r"/(Users|home)/(?!<|\$|\{|user/|username/|example/)[A-Za-z0-9._-]+/")
SECRET_PATTERNS = (
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}\b"),
    re.compile(r"\bsk-ant-[A-Za-z0-9_-]{20,}\b"),
    re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
    re.compile(r"(?i)\b(password|passwd|secret|token|api[_-]?key)\b\s*[:=]\s*['\"][^'\"\s]{8,}['\"]"),
)
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
DATED_NAME = re.compile(r"_(\d{4}-\d{2}-\d{2})\.md$")
FENCE = re.compile(r"^\s*(```|~~~)")


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    reason: str

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: {self.reason}" if self.line else f"{self.path}: {self.reason}"


def check_document(path: str, text: str, exists) -> list[Finding]:
    findings: list[Finding] = []
    in_fence = False
    has_status = False
    for number, line in enumerate(text.splitlines(), start=1):
        if FENCE.match(line):
            in_fence = not in_fence
        if line.startswith("## Status"):
            has_status = True
        if PORTUGUESE_PROSE.search(line):
            findings.append(Finding(path, number, "documentation must be in English"))
        if HOME_PATH.search(line):
            findings.append(Finding(path, number, "machine-specific home path; use ~ or a placeholder"))
        if any(rx.search(line) for rx in SECRET_PATTERNS):
            findings.append(Finding(path, number, "possible secret"))
        if in_fence:
            continue
        for target in LINK.findall(line):
            if re.match(r"^[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
                continue
            relative = target.split("#", 1)[0]
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(path), relative))
            if resolved.startswith("../") or not exists(resolved):
                findings.append(Finding(path, number, f"broken link to {target}"))
    name = PurePosixPath(path).name
    if name != "README.md" and not has_status:
        findings.append(Finding(path, 0, 'missing a "## Status" section'))
    dated = DATED_NAME.search(name)
    if dated:
        try:
            datetime.date.fromisoformat(dated.group(1))
        except ValueError:
            findings.append(Finding(path, 0, f"invalid date {dated.group(1)} in file name"))
    return findings


def git_files(root: Path) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    return [p for p in out.decode().split("\0") if p]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args(argv)
    root: Path = args.root

    files = git_files(root)
    present = set(files)

    def exists(relative: str) -> bool:
        return relative in present or (root / relative).exists()

    findings: list[Finding] = []
    documents = [f for f in files if f.endswith(".md")]
    for path in documents:
        text = (root / path).read_text(encoding="utf-8")
        findings.extend(check_document(path, text, exists))
    for finding in findings:
        print(f"check-docs: {finding}", file=sys.stderr)
    if findings:
        print(f"check-docs: {len(findings)} problem(s)", file=sys.stderr)
        return 1
    print(f"check-docs: {len(documents)} document(s) clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
