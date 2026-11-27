# The Vulkan bridge

## Status

Design and first implementation for roadmap step F2, 27 November 2026. Describes
`guest/shims/libvulkan.so.1.c` and `crates/cs2metal-linux/src/vulkan.rs`.

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
`CS2METAL_VULKAN_HIDE`, a comma-separated list of device extension names, removes
them from what `vkEnumerateDeviceExtensionProperties` reports to the game, to test
whether the renderer works better without one, such as `VK_KHR_present_wait`.

## Validation

A test creates an instance and a device through the wrappers on the owner's
macOS loader and MoltenVK, in an x86-64 process under Rosetta 2, as the renderer
does. It is skipped where no Vulkan SDK is installed.

## What remains

- Every Vulkan call crosses the bridge, with two GS switches. That is cheap
  enough for the first frame; recording command buffers at the game's rate may
  need cheaper crossings later.
- `vkGetPhysicalDeviceFeatures2` and related queries answer with MoltenVK's
  features. Whether the renderer accepts them is what F2's gate shows.
