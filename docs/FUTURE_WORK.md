# Future work

## Status

Backlog, reconciled on 26 November 2026. Nothing here is approved for
implementation; an item moves into the [implementation plan](IMPLEMENTATION_PLAN.md)
only with a decision record.

## Ideas

| Idea | Why it might matter | Prerequisite |
| --- | --- | --- |
| Own x86-64 to arm64 translation | The exit from Rosetta after macOS 27, and the end of the `fs` problem and the GS swap cost | M9 decision |
| KosmicKrisp built from Mesa source for x86-64 | The SDK ships it for arm64 only; an x86-64 build would offer `VK_EXT_mutable_descriptor_type`, `VK_EXT_multi_draw`, and `VK_EXT_conditional_rendering`, which MoltenVK lacks | M6 measurements showing MoltenVK limits the renderer |
| Own Vulkan-to-Metal translation for the renderer's subset | Control over performance and features if neither KosmicKrisp nor MoltenVK serves the renderer | M6 measurements |
| App bundle and launcher | A double-click experience with the integrity guard built in | M8 |
| Real game controllers through the Game Controller framework | Beyond what SDL3 already covers on macOS | M5 |
| Continuous integration on macOS runners | Run the check suite on every push | A hosted remote for the code repository |
| Shared components with Native VIctory for macOS | Its bridge, signal handling, and Metal findings overlap with this project | Attribution and notices as the GPL-3.0 requires (see the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md)) |
| Syscall-level mode: load the runtime's real glibc and emulate Linux system calls | Fewer layout translations, at the price of rewriting `syscall` instructions in glibc | Evidence from M3 that the glibc layer is the bottleneck |
