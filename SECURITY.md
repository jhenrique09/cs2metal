# Security

## Scope

cs2metal loads third-party x86-64 code into its own process. Treat every game file
as untrusted input: the ELF parser is bounds-checked and must never panic, and the
loader will validate every address it derives from a file before using it.

## Secrets

The project never asks for, stores, or forwards Steam credentials. Steam sign-in
happens only in Valve's own client. If a secret is committed by mistake, rotate it
first, then remove it from history.

## Reporting

Report a vulnerability privately to the repository owner instead of opening a public
issue. Include the affected commit, the input that triggers the problem, and the
observed behavior.
