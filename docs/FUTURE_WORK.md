# Future work

## Status

Backlog, reconciled on 27 November 2026. Nothing here is approved for
implementation; an item moves into the [implementation plan](IMPLEMENTATION_PLAN.md)
only with a decision record.

## Ideas

| Idea | Why it might matter | Prerequisite |
| --- | --- | --- |
| Own x86-64 to arm64 translation | The exit from Rosetta after macOS 27, and the end of the `fs` problem and the GS swap cost | M9 decision |
| KosmicKrisp built from Mesa source for x86-64 | The SDK ships it for arm64 only; an x86-64 build would offer `VK_EXT_mutable_descriptor_type`, `VK_EXT_multi_draw`, and `VK_EXT_conditional_rendering`, which MoltenVK lacks | M6 measurements showing MoltenVK limits the renderer |
| Own Vulkan-to-Metal translation for the renderer's subset | Control over performance and features if neither KosmicKrisp nor MoltenVK serves the renderer | M6 measurements |
| A signed application bundle with the host inside | `create-shortcut.sh` makes an application that runs `run.sh` from `~/cs2-metal/bin`; a self-contained bundle would need no checkout and no terminal | A decision on distributing built binaries |
| The guest's C library as a release download | Building glibc from source is the one step that needs Docker. The build is reproducible and its six libraries have tracked checksums, so a release could carry them and `build.sh` could download and check them (it already accepts a directory, `CS2METAL_GLIBC_FROM`). It would make the project a distributor of an LGPL library, with the duty to offer its source and patch | A hosted remote with releases; a decision record on distributing the library, which the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) does not allow today |
| Real game controllers through the Game Controller framework | Beyond what SDL3 already covers on macOS | M5 |
| Continuous integration on macOS runners | Run the check suite on every push | A hosted remote for the code repository |
| Shared components with Native VIctory for macOS | Its bridge, signal handling, and Metal findings overlap with this project | Attribution and notices as the GPL-3.0 requires (see the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md)) |
| Shader pre-compilation from Fossilize data | The stutters that remain are the first use of each pipeline: 16 spikes of 0.1 to 1.9 s in the owner's first ten minutes of a casual match on 27 November 2026, almost all in the first two. On Linux the Steam client delivers a game's pipeline descriptions as Fossilize databases and replays them through the GPU driver before the game runs (REPORTED, community sources; no source found that `steamcmd` can fetch them, and the macOS client does not offer them). Replaying such a database through MoltenVK, inside the host process so that Metal's caches are the game's, would pay that cost before a match | A Fossilize database for the build, which today only a Linux Steam client downloads; a decision record on reading one |
| Syscall-level mode: load the runtime's real glibc and emulate Linux system calls | Fewer layout translations, at the price of rewriting `syscall` instructions in glibc | Evidence from M3 that the glibc layer is the bottleneck |
