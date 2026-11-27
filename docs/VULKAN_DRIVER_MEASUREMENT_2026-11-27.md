# Vulkan drivers measured on the Mac, 27 November 2026

## Status

Measured on 27 November 2026 on an M1 Max with macOS 27.2.1 (26B131) and LunarG's
Vulkan SDK 1.4.363.1, for roadmap steps A6, B4, and F1. Replaces with measurements
the documentation-based driver columns and the UNKNOWN cells of the
[Vulkan requirements research](VULKAN_REQUIREMENTS_RESEARCH_2026-11-27.md), and
answers open question Q-6 of the
[feasibility research](FEASIBILITY_RESEARCH_2026-11-26.md). Not edited after the
fact.

## Questions

1. Can a Vulkan driver run inside an x86-64 process under Rosetta 2 and present to a
   window? (Q-6)
2. Which of the extensions the Linux renderer module names does each driver offer
   on this Mac, and does each driver meet the two requirements CS2's configuration
   sets: descriptor indexing and subgroup operations?

## Method

- **Architectures.** `lipo -archs` on each library of the SDK.
- **Driver reports.** `vulkaninfo --json`, run once per driver, selected with
  `VK_DRIVER_FILES`, and per architecture with `arch -x86_64` and `arch -arm64`.
  `arch` is a protected system binary and drops `DYLD_*` variables, so they are
  passed with `arch -e`.
- **The renderer's names.** `cs2metal vulkan scan` on the Linux
  `librendersystemvulkan.so`, and `cs2metal vulkan compare` against the reports.
- **Presentation.** The F1 probe, `spikes/vulkan-present/` in the repository: an SDL3
  window, `SDL_Vulkan_CreateSurface`, a device, one graphics pipeline from SPIR-V,
  a FIFO swapchain, and 300 cleared and presented frames, built for x86-64 and, as a
  native control, for arm64.

## Q-6: a driver in a Rosetta process

| Library in the SDK | Architectures |
| --- | --- |
| Vulkan loader (`libvulkan.1.dylib`) | x86_64, arm64 |
| MoltenVK (`libMoltenVK.dylib`) | x86_64, arm64 |
| KosmicKrisp (`libvulkan_kosmickrisp.dylib`) | **arm64 only** |
| `SDL3.framework` | x86_64, arm64 |

| Run | Result | Pipeline creation | First frame | Mean frame | Worst frame |
| --- | --- | --- | --- | --- | --- |
| MoltenVK 1.4.2, x86_64 under Rosetta 2 | **presented** | 26.95 ms | 38.81 ms | 8.37 ms | 19.85 ms |
| MoltenVK 1.4.2, arm64 | presented | 25.44 ms | 27.82 ms | 8.33 ms | 12.70 ms |
| KosmicKrisp (SDK 1.4.363), arm64 | presented | 29.10 ms | 224.68 ms | 8.33 ms | 13.05 ms |
| KosmicKrisp, x86_64 under Rosetta 2 | the loader refuses the driver: "incompatible architecture (have 'arm64', need 'x86_64')" | | | | |

**Q-6 is answered: MoltenVK.** It creates an instance and a device, translates
shaders, and presents to an SDL3 window from an x86-64 process under Rosetta 2.
(CONFIRMED) Its frame pacing there equals the native build's: the mean of 8.3 ms is
the display's 120 Hz refresh in FIFO mode, so it measures presentation, not
rendering throughput. In one run each, Rosetta cost about 11 ms on the first frame and 1.5 ms on
the one pipeline. KosmicKrisp as distributed cannot be used by the host. It stays
open only as a source build for x86-64 (see [Future work](FUTURE_WORK.md)).

## The renderer's names, from the Linux module

`cs2metal vulkan scan` finds 73 extension names and 774 command names in the Linux
renderer module. Four of the extensions are instance extensions, which a device
report does not list: `VK_EXT_debug_utils`, `VK_KHR_device_group_creation`,
`VK_KHR_get_physical_device_properties2`, and `VK_KHR_get_surface_capabilities2`.
One is stored only as a 16-byte piece and completes to `VK_EXT_surface_maintenance1`,
also an instance extension. Two more pieces of names stored in full were dropped.
That leaves 68 device extension names.

Against the Windows-derived list of the research, the Linux module adds
`VK_AMD_buffer_marker`, `VK_AMD_device_coherent_memory`, and
`VK_KHR_dedicated_allocation`. It lacks `VK_EXT_full_screen_exclusive` (Windows only)
and `VK_NV_device_diagnostics_config`. The rest matches, which confirms the
research's INFERRED premise that both builds share one renderer.

## Device extensions offered

| Measure | MoltenVK (x86_64 and arm64 alike) | KosmicKrisp (arm64) |
| --- | --- | --- |
| Device extensions the driver reports in total | 129 | 145 |
| Of the module's 68 device extension names | 33 | 35 |
| In common | 31 | 31 |
| Only this driver | `VK_GOOGLE_display_timing`, `VK_KHR_fragment_shader_barycentric` | `VK_AMD_buffer_marker`, `VK_EXT_conditional_rendering`, `VK_EXT_multi_draw`, `VK_EXT_mutable_descriptor_type` |

MoltenVK reports exactly the same extensions in an x86-64 process as in a native
one.

The 31 names that neither driver offers, by the roles the research assigned:

| Role | Count | Names |
| --- | --- | --- |
| Vendor-specific | 9 | the AMD, NV, and NVX names, including `VK_AMD_device_coherent_memory`, new in the Linux list |
| Ray tracing and other options the configuration does not request | 6 | `VK_KHR_acceleration_structure`, `VK_KHR_ray_query`, `VK_KHR_ray_tracing_pipeline`, `VK_KHR_ray_tracing_maintenance1`, `VK_KHR_ray_tracing_position_fetch`, `VK_KHR_compute_shader_derivatives` |
| Optional by the renderer's diagnostics | 6 | `VK_EXT_depth_bias_control`, `VK_EXT_device_generated_commands`, `VK_EXT_opacity_micromap`, `VK_KHR_shader_abort`, `VK_KHR_shader_abort_message`, `VK_KHR_shader_clock` |
| Pipeline libraries | 2 | `VK_EXT_graphics_pipeline_library`, `VK_KHR_pipeline_library` |
| Role not visible statically | 7 | `VK_EXT_memory_priority`, `VK_EXT_mesh_shader`, `VK_EXT_pageable_device_local_memory`, `VK_EXT_validation_cache`, `VK_KHR_device_fault`, `VK_KHR_fragment_shading_rate`, `VK_KHR_pipeline_executable_properties` |
| Not an extension | 1 | `VK_EXT_extended_dynamic_state_2`, a misspelling inside a diagnostic |

The measurement agrees with the documentation-based columns of the research except
for one name: MoltenVK's documentation lists `VK_EXT_multi_draw`, but the driver
does not report it on this Mac.

## The two hard requirements

| | MoltenVK | KosmicKrisp |
| --- | --- | --- |
| `VK_EXT_descriptor_indexing`, and the `descriptorIndexing` feature | yes, yes | yes, yes |
| Subgroup size | 32 | 32 |
| Subgroup operations | basic, vote, arithmetic, ballot, shuffle, shuffle relative, clustered, quad, rotate, rotate clustered | the same |
| Stages with subgroup operations | tessellation control, fragment, compute | fragment, compute |
| `textureCompressionBC` | yes | yes |

Both drivers report every subgroup operation bit and BC texture compression, so the
UNKNOWN cells of the research close in their favour. (CONFIRMED) Whether the
renderer's `VulkanRequireSubgroupWaveOpSupport` check also wants subgroup
operations in the vertex stage, which neither driver offers, is not visible
statically. The host will see the answer when the renderer creates its device in
step F2, and it is the one requirement that could still stop MoltenVK.
(UNKNOWN)

## What it means

- **The renderer path of the plan stands, on MoltenVK.** F1 passes for MoltenVK,
  and Gate 0 now waits only on the Steam probe H-2.
- **The architecture's substitution table names the driver:** the macOS Vulkan
  loader, x86-64 slice, with MoltenVK.
- **Shader compilation stutter is expected.** Neither driver offers graphics
  pipeline libraries, and MoltenVK adds SPIR-V to Metal translation. One trivial
  pipeline took about 27 ms under Rosetta 2. The game's pipeline count and cache
  behaviour decide how much that matters; it is measured in M6.
