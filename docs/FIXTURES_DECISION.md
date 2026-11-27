# Decision: Linux fixtures are rebuilt, not tracked

## Status

Proposed on 27 November 2026 and in effect for roadmap step C1. The owner asked
for C1 to proceed. This record settles the question the implementation plan left
open for milestone M2: whether generated fixtures are tracked or rebuilt.

## Context

The loader's test suite is a set of small Linux programs written in this project
(`fixtures/`). They must be real x86-64 ELF objects, built by the toolchain the
CS2 build targets. The repository guard rejects ELF, Mach-O, and PE files in
tracked content, so that no game binary or derived object is ever committed.

## Options

| Option | Summary | Assessment |
| --- | --- | --- |
| 1. **Rebuild, track sources and checksums** | Track the sources, the expected outputs, and the SHA-256 of every built file. Build in an SDK image pinned by digest, with reproducible flags. | The guard stays absolute. A checksum mismatch shows any drift in the toolchain or the flags. Building needs Docker and an x86-64 Linux host or emulation. |
| 2. Track the binaries | Commit the built fixtures with an allowlist in the guard | Anyone can test without Docker, but the guard gains an exception, and binaries enter a history meant to hold none. |
| 3. Download prebuilt fixtures | Publish them elsewhere | A supply chain and a place to host, for files that take seconds to build. |

## Decision

Option 1.

- Sources, `build.sh`, `run.sh`, `expected/`, and `SHA256SUMS` are tracked under
  `fixtures/`. Binaries go to the ignored `target/fixtures/`.
- The image is the Steam Runtime 3 ("sniper") SDK, pinned by digest in
  `tools/fixtures.sh`. Builds and runs have no network.
- `expected/` is produced by running each fixture natively on Linux. That is the
  ground truth the cs2metal loader must reproduce on macOS.
- `tools/check.sh` does not build fixtures, because validation must not need a
  Linux host. The loader's fixture tests run when `target/fixtures/` exists and say
  so when it does not.

## Consequences

- Changing a fixture means running `tools/fixtures.sh --update` and committing the
  new `expected/` and `SHA256SUMS` with the source change.
- A new SDK image changes the checksums. Moving the pin is its own commit, with the
  reason.
