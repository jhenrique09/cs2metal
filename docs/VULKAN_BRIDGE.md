# The Vulkan bridge

## Status

Design and first implementation for roadmap step F2, 27 November 2026, extended the
same day with the recording of commands on the guest's side. Describes
`guest/shims/libvulkan.so.1.c`, `crates/cs2metal-linux/src/vulkan.rs`, and
`crates/cs2metal-linux/src/record.rs`.

## What the renderer needs

The renderer module, `librendersystemvulkan.so`, does not link a Vulkan loader. It
loads `libvulkan.so.1` with `dlopen`, looks up `vkGetInstanceProcAddr`, and
reaches every other function through it and through `vkGetDeviceProcAddr`.
(CONFIRMED from the module's dynamic section and strings, read locally,
27 November 2026.) On Linux, the runtime's loader then looks for Linux drivers. Under
cs2metal it finds none and the game stops with its message that Vulkan 1.2 could
not be initialized, which the first client boot showed.

## Design

1. cs2metal serves its shim wherever the guest opens `libvulkan.so.1` or
   `libvulkan.so`. Its exports are the loader's global entry points, public
   Vulkan names.
2. The shim's constructor makes the private system call `0x43530003`. The host
   loads the macOS Vulkan loader into its own domain, by default the newest Vulkan
   SDK's (`CS2METAL_VULKAN_LOADER` names another). It points the loader at the
   SDK's MoltenVK manifest unless `VK_DRIVER_FILES` is set, the driver the
   [Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md) chose.
3. Every function pointer the loader hands out, through either lookup function,
   reaches the guest as a host thunk. Handles, structures, and `pNext` chains cross
   unchanged, since both sides are x86-64 System V.

### Adjusted calls

| Call | Adjustment |
| --- | --- |
| `vkCreateInstance` | Enables `VK_KHR_portability_enumeration` with its creation flag, without which the macOS loader hides MoltenVK. Drops requested extensions the host lacks, naming them. Debug callbacks chained in `pNext` are swapped for guest entries during the call |
| `vkCreateDevice` | Enables `VK_KHR_portability_subset`, which the specification requires on a device that offers it. Names requested extensions the device lacks |
| `vkCreateDebugUtilsMessengerEXT`, `vkCreateDebugReportCallbackEXT` | The callback is guest code: it is called through the guest thunk |
| `vkGetInstanceProcAddr`, `vkGetDeviceProcAddr` | Answer with bridged entries, and with themselves |
| `vkCmdBindDescriptorSets` | The renderer binds ranges of sets with `VK_NULL_HANDLE` among them, which the Linux drivers skip. MoltenVK reads through the null pointer when it encodes the command at submission, and the submitting thread never returns. Each run of sets that are not null is bound by a call of its own. Dynamic offsets go to the first run; a binding that had both several runs and dynamic offsets would need the set layouts to split them, and is named once if it happens |

Allocation callbacks are guest code too. The adjusted calls do not pass them on,
so the loader uses its own allocator for its objects. The others pass them through
unchanged; MoltenVK does not call them.

`CS2METAL_VULKAN_TRACE=1` logs the instance and device extensions the renderer
requests, the measurement Q-7 asks for. It also counts the swapchain's calls, and
every ten seconds reports the pacing of presents: how many, the median interval,
the 99th percentile, the longest, and how many took more than twice the median.
A present more than 250 ms after the one before is named on its own, with the time
of day, to set beside the game's console log. With each pacing report it names the
Vulkan functions the guest called most in the period, as calls a present: every
entry it hands the guest is put behind one that counts. Counting costs a locked
increment a call, so `CS2METAL_VULKAN_TRACE=pacing` leaves it out and keeps only
the reports on presents, for measuring the pace itself.
`MVK_CONFIG_SHOULD_MAXIMIZE_CONCURRENT_COMPILATION` is set to 1 in the host
environment unless the owner sets it: MoltenVK then lets Metal compile several
shaders at once, where by default it compiles one at a time, while the renderer
compiles pipelines on several threads. Its effect on the first-use stutters is not
measured yet.
`CS2METAL_SHADER_TRACE=1` asks MoltenVK for its performance tracking: every
3000 frames it reports, for each step of making a pipeline (SPIR-V to Metal's
shading language, that to a Metal library, a function from the library, the
pipeline state), how many it did and how long they took, and the same for
encoding command buffers, the GPU's execution of them, and presents. The host
sets MoltenVK's log level to info for it, since the SDK's build says nothing
below a warning by default. On a map loaded for the first time (client boot 88
of 27 November 2026, two minutes on `de_nuke`), Metal's compiler turning its
shading language into libraries took 16.4 s of the 20.6 s spent making
pipelines, the pipeline states 3.8 s, and the translation of SPIR-V 0.7 s: the
game's own pipeline cache handed MoltenVK 1,286 shaders already translated, and
133 were translated anew. The game's own `vulkan_pipeline_compile_spew` was
tried beside it and writes nothing to the console log in this build.
`CS2METAL_VULKAN_HIDE`, a comma-separated list of device extension names, removes
them from what `vkEnumerateDeviceExtensionProperties` reports to the game, to test
whether the renderer works better without one, such as `VK_KHR_present_wait`.

### Commands recorded on the guest's side

A Vulkan call through a host thunk costs two GS switches, half a microsecond for
the pair. The renderer makes 8,200 to 9,000 calls a present in a match (client
boot 87 of 27 November 2026, counted by the trace), and three quarters of them
are `vkCmd*`: commands recorded into a command buffer, which return nothing and
take effect only when the buffer is executed.

For those the guest is given a function that does not cross the bridge. It is
neutral code (`crates/cs2metal-linux/src/record.rs`): it runs in the guest's
domain and writes the command into a stream kept for the command buffer, with a
copy of everything its arguments point at. The stream is *flushed*, replayed in
order into the loader's functions, in one crossing:

- when the guest ends the command buffer (`vkEndCommandBuffer` is wrapped);
- when the stream is full, at 64 KiB, about two thousand commands;
- before any `vkCmd*` on the same command buffer that is not recorded: such a
  function is handed out behind an entry that flushes first.

`vkBeginCommandBuffer` gives the command buffer its stream, empty, and
`vkResetCommandBuffer` empties it. The driver therefore sees the same commands in
the same order, only later, and Vulkan asks nothing of a command buffer between
its begin and its end that could show the difference.

| Recorded | How |
| --- | --- |
| Commands whose arguments are all integers and handles: binding a pipeline or an index buffer, the draws and dispatches, the stencil state, the dynamic state of `VK_EXT_extended_dynamic_state` under its core and `EXT` names, queries, `vkCmdFillBuffer`, `vkCmdEndRendering` | The argument registers, as they came |
| `vkCmdBindVertexBuffers`, `vkCmdBindDescriptorSets`, `vkCmdPushConstants`, `vkCmdCopyBuffer`, viewports and scissors, `vkCmdSetBlendConstants`, `vkCmdSetDepthBias` | The arguments and a copy of each array. `vkCmdBindDescriptorSets` is replayed through the wrapper that leaves null sets out |
| `vkCmdBeginRendering`, `vkCmdPipelineBarrier2` | The structure and the arrays it names, copied behind it, with the pointers kept as offsets until the replay. A structure with a `pNext` chain is not recorded: the chain is not copied |

A command that cannot be recorded as it is goes to the driver directly, after a
flush: one with a `pNext` chain, one for a command buffer that has no stream (the
table holds 4,096), one larger than a stream. That is slower and never out of
order.

What still crosses, call by call, is everything that is not a command: above all
the allocation of descriptor sets and their update from a template, a thousand
of each a present, which return or change objects at once.

`CS2METAL_NO_VULKAN_RECORD=1` gives the guest the bridged functions, as before.
With the trace on, each report says how many commands were recorded a present,
in how many flushes, and how many were not recordable.

## Validation

A test creates an instance and a device through the wrappers on the owner's
macOS loader and MoltenVK, in an x86-64 process under Rosetta 2, as the renderer
does. It is skipped where no Vulkan SDK is installed.

## What remains

- A quarter of the renderer's Vulkan calls still cross the bridge one by one,
  with two GS switches each: mostly `vkAllocateDescriptorSets` and
  `vkUpdateDescriptorSetWithTemplate`, about a thousand of each a present.
  Deferring the update would need the layout of each update template; the
  allocation returns a handle and cannot be deferred at all.
- `vkGetPhysicalDeviceFeatures2` and related queries answer with MoltenVK's
  features. Whether the renderer accepts them is what F2's gate shows.
