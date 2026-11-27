#!/usr/bin/env python3
"""Reject commits that carry secrets, game files, binaries, or non-English docs.

The guard runs from ``tools/check.sh``. In ``--staged`` mode it reads file contents
from the Git index, so it checks exactly what would be committed; ``--all`` checks
the working tree.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import PurePosixPath

FORBIDDEN_NAMES = (
    (re.compile(r"(^|/)ssfn[^/]*$"), "Steam sentry file"),
    (re.compile(r"(^|/)loginusers\.vdf$"), "Steam login cache"),
    (re.compile(r"(^|/)config\.vdf$"), "Steam client configuration"),
    (re.compile(r"\.vpk$"), "game archive"),
    (re.compile(r"\.acf$"), "Steam app manifest"),
    (re.compile(r"(^|/)\.env(\.(?!example$)[^/]*)?$"), "environment file"),
    (re.compile(r"\.(pem|p12|key)$"), "private key or certificate bundle"),
    (re.compile(r"(^|/)id_(rsa|ed25519|ecdsa)[^/]*$"), "SSH key"),
    (re.compile(r"(^|/)cs2metal\.local\.toml$"), "local configuration"),
    (re.compile(r"^(reports|runs|baselines)/"), "generated local output"),
)

BINARY_MAGICS = (
    (b"\x7fELF", "ELF binary"),
    (b"\xcf\xfa\xed\xfe", "Mach-O binary"),
    (b"\xce\xfa\xed\xfe", "Mach-O binary"),
    (b"\xca\xfe\xba\xbe", "universal binary"),
    (b"MZ", "PE binary"),
)

SECRET_PATTERNS = (
    (re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"), "private key"),
    (re.compile(r"\bAKIA[0-9A-Z]{16}\b"), "AWS access key"),
    (re.compile(r"\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}\b"), "GitHub token"),
    (re.compile(r"\bgithub_pat_[A-Za-z0-9_]{40,}\b"), "GitHub token"),
    (re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b"), "Slack token"),
    (re.compile(r"\bsk-ant-[A-Za-z0-9_-]{20,}\b"), "Anthropic API key"),
    (re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+"), "JWT"),
    (
        re.compile(
            r"(?i)\b(password|passwd|secret|token|api[_-]?key)\b\s*[:=]\s*['\"][^'\"\s]{8,}['\"]"
        ),
        "credential assignment",
    ),
)

HOME_PATH = re.compile(r"/(Users|home)/(?!<|\$|\{|user/|username/|example/)[A-Za-z0-9._-]+/")

PORTUGUESE_PROSE = re.compile(
    r"\b(?:não|também|você|então|está|são|código|configuração|aplicação|"
    r"documentação|execução|instalação|versão|conexão|usuário|diretório|"
    r"arquivos?|segurança|requisitos|objetivo|portanto|através)\b",
    re.IGNORECASE,
)

DOCUMENT_SUFFIXES = {".md", ".markdown", ".txt", ".rst"}


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    reason: str

    def __str__(self) -> str:
        where = f"{self.path}:{self.line}" if self.line else self.path
        return f"{where}: {self.reason}"


def scan_name(path: str) -> list[Finding]:
    return [Finding(path, 0, f"forbidden file ({why})") for rx, why in FORBIDDEN_NAMES if rx.search(path)]


def scan_bytes(path: str, data: bytes) -> list[Finding]:
    for magic, what in BINARY_MAGICS:
        if data.startswith(magic) and (magic != b"MZ" or b"PE\x00\x00" in data[:1024]):
            return [Finding(path, 0, f"{what} must not be tracked")]
    if b"\x00" in data[:8192]:
        return [Finding(path, 0, "binary content must not be tracked")]
    return scan_text(path, data.decode("utf-8", errors="replace"))


def scan_text(path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    is_document = PurePosixPath(path).suffix.lower() in DOCUMENT_SUFFIXES
    for number, line in enumerate(text.splitlines(), start=1):
        for rx, why in SECRET_PATTERNS:
            if rx.search(line):
                findings.append(Finding(path, number, f"possible secret ({why})"))
        if HOME_PATH.search(line):
            findings.append(Finding(path, number, "machine-specific home path; use ~ or a placeholder"))
        if is_document and PORTUGUESE_PROSE.search(line):
            findings.append(Finding(path, number, "tracked documentation must be in English"))
    return findings


def git(*args: str) -> bytes:
    return subprocess.run(["git", *args], check=True, stdout=subprocess.PIPE).stdout


def staged_files() -> list[str]:
    out = git("diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z")
    return [p for p in out.decode().split("\0") if p]


def tracked_files() -> list[str]:
    out = git("ls-files", "-z", "--cached", "--others", "--exclude-standard")
    return [p for p in out.decode().split("\0") if p]


def read_file(path: str, staged: bool) -> bytes:
    if staged:
        return git("cat-file", "blob", f":{path}")
    with open(path, "rb") as handle:
        return handle.read()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--staged", action="store_true", help="check the Git index")
    mode.add_argument("--all", action="store_true", help="check tracked and untracked files (default)")
    args = parser.parse_args(argv)

    paths = staged_files() if args.staged else tracked_files()
    findings: list[Finding] = []
    for path in paths:
        findings.extend(scan_name(path))
        try:
            findings.extend(scan_bytes(path, read_file(path, args.staged)))
        except FileNotFoundError:
            continue
    for finding in findings:
        print(f"repo-guard: {finding}", file=sys.stderr)
    if findings:
        print(f"repo-guard: {len(findings)} problem(s); nothing was committed.", file=sys.stderr)
        return 1
    print(f"repo-guard: {len(paths)} file(s) clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
