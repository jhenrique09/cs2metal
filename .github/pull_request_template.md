## What and why

<!-- What this pull request changes, and the problem it solves. Link the issue: "Fixes #123". -->

## How it was tested

<!-- tools/check.sh passes. For a change in what the game does: the Mac, macOS
version, how the game was started (offline, please), and what was measured. Cite
only counts, system call numbers, and public interface names from a log. -->

## Checklist

- [ ] `tools/check.sh` passes on every commit.
- [ ] Each commit is one change, with an imperative subject under 72 characters and a body that says why.
- [ ] A change in behavior comes with tests in the same commit.
- [ ] `docs/` is updated where the change affects the architecture, a decision, or a milestone.
- [ ] Code, comments, documentation, and commit messages are in English.
- [ ] Nothing here comes from a game file: no copied files, symbol lists, disassembly, or quoted strings.
- [ ] No credentials, Steam session files, or real home paths.
- [ ] Nothing here studies, disables, intercepts, or works around Valve Anti-Cheat, or changes gameplay.
- [ ] Every new in-memory transform of guest code is registered, counted at load, and has an accepted decision record.
