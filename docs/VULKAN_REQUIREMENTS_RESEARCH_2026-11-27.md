# Vulkan requirements research, 27 November 2026

## Status

Research snapshot of 27 November 2026, built from public sources only: no game file
was read. It answers part of open question Q-7 of the
[feasibility research](FEASIBILITY_RESEARCH_2026-11-26.md) and names the three
measurements that close the rest. Not edited after the fact.

## Question

Which Vulkan extensions and features does the CS2 renderer need, and do the two
Vulkan-on-Metal drivers, MoltenVK and KosmicKrisp, offer them?

## Sources

| Side | Source | Tag |
| --- | --- | --- |
| CS2 renderer | The strings of the Vulkan renderer module and the game's renderer configuration (`gameinfo.gi`), as published by SteamDB's [GameTracking-CS2](https://github.com/SteamDatabase/GameTracking-CS2) repository, last updated 23 November 2026. | REPORTED |
| MoltenVK | The extension list in the [MoltenVK runtime user guide](https://github.com/KhronosGroup/MoltenVK/blob/main/Docs/MoltenVK_Runtime_UserGuide.md) on its main branch, which includes extensions of the unreleased 1.4.3. | REPORTED |
| KosmicKrisp | Mesa's [features list](https://gitlab.freedesktop.org/mesa/mesa/-/blob/main/docs/features.txt) on its main branch (driver `kk`) and the [driver page](https://docs.mesa3d.org/drivers/kosmickrisp.html). | REPORTED |

Three limits follow from the sources:

- The published strings come from the **Windows** build of the renderer. The Linux
  module is built from the same renderer, so the names should match, but that is
  INFERRED until a static scan of the Linux file is run locally.
- A name inside the module says the renderer *knows* an extension, not that it
  *requires* it. The list is a superset of what any one machine enables.
- Driver lists say what a driver can expose, not what it exposes on a given chip and
  macOS version. `vulkaninfo` on the target machine is the authority.

## What the game configuration demands

The renderer reads switches from the game's configuration that turn optional
capabilities into hard requirements. CS2's configuration sets two of them. (REPORTED)

| Switch | Set by CS2 | Needs | MoltenVK | KosmicKrisp |
| --- | --- | --- | --- | --- |
| `VulkanRequireDescriptorIndexing` | **yes** | `VK_EXT_descriptor_indexing` | yes | yes |
| `VulkanRequireSubgroupWaveOpSupport` | **yes** | Shader subgroup operations beyond the basic set | Subgroup extensions listed; exact operation bits UNKNOWN | Core 1.1 subgroups; exact operation bits UNKNOWN |
| `VulkanRequireSM6` | no | `VK_KHR_shader_float16_int8`, `VK_KHR_16bit_storage` | yes | yes |
| `VulkanRequireBarycentrics` | no | `VK_KHR_fragment_shader_barycentric` | yes | no |
| `VulkanRequireComputeShaderDerivatives` | no | `VK_KHR_compute_shader_derivatives` | no | no |
| `VulkanRequireFullGPURayTracing` | no | The ray tracing extension family | no | no |
| `VulkanMinAPIVersion` | not set | The default minimum is UNKNOWN | reports Vulkan 1.4 | conformant to Vulkan 1.4 |

**Finding 1.** Of the switches CS2 sets, descriptor indexing is covered by both
drivers, and subgroup operations are the one hard requirement whose coverage is not
yet known. The subgroup operation bits each driver reports on this Mac decide it.

## The renderer's extension names against both drivers

The module names 71 distinct Vulkan extensions (and one misspelling inside a
diagnostic) and 787 Vulkan commands. Role is assigned from the configuration
switches above and from the renderer's own diagnostics, paraphrased: "Optional" means
the renderer reports disabling the capability when the driver lacks it.

| Extension | Role for CS2 | MoltenVK | KosmicKrisp | Note |
| --- | --- | --- | --- | --- |
| `VK_EXT_descriptor_indexing` | Required | yes | yes | Demanded by the game configuration |
| `VK_KHR_swapchain` | Required | yes | yes | Presentation |
| `VK_EXT_graphics_pipeline_library` | Optional, matters | no | no | Valve recommends it; without it expect shader compilation stutter |
| `VK_KHR_pipeline_library` | Optional, matters | no | no | Dependency of graphics_pipeline_library |
| `VK_EXT_buffer_device_address` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_conditional_rendering` | Unknown | no | yes | Named by the renderer; role not visible statically |
| `VK_EXT_debug_utils` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_extended_dynamic_state` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_load_store_op_none` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_memory_budget` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_memory_priority` | Unknown | no | no | Named by the renderer; role not visible statically |
| `VK_EXT_mesh_shader` | Unknown | no | no | Named by the renderer; role not visible statically |
| `VK_EXT_multi_draw` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_mutable_descriptor_type` | Unknown | no | yes | Named by the renderer; role not visible statically |
| `VK_EXT_pageable_device_local_memory` | Unknown | no | no | Named by the renderer; role not visible statically |
| `VK_EXT_pipeline_creation_cache_control` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_pipeline_robustness` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_robustness2` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_surface_maintenance1` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_validation_cache` | Unknown | no | not listed | Named by the renderer; role not visible statically |
| `VK_GOOGLE_display_timing` | Unknown | yes | no | Named by the renderer; role not visible statically |
| `VK_KHR_buffer_device_address` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_create_renderpass2` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_depth_stencil_resolve` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_device_fault` | Unknown | no | no | Named by the renderer; role not visible statically |
| `VK_KHR_device_group_creation` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_dynamic_rendering` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_fragment_shading_rate` | Unknown | no | no | Named by the renderer; role not visible statically |
| `VK_KHR_get_memory_requirements2` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_get_physical_device_properties2` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_get_surface_capabilities2` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_maintenance3` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_pipeline_executable_properties` | Unknown | no | no | Named by the renderer; role not visible statically |
| `VK_KHR_push_descriptor` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_separate_depth_stencil_layouts` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_shader_non_semantic_info` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_swapchain_mutable_format` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_KHR_synchronization2` | Unknown | yes | yes | Named by the renderer; role not visible statically |
| `VK_EXT_depth_bias_control` | Optional | no | no | Disabled when unsupported |
| `VK_EXT_device_generated_commands` | Optional | no | no | Disabled when unsupported |
| `VK_EXT_extended_dynamic_state2` | Optional | yes | yes | Disabled when its features are missing |
| `VK_EXT_extended_dynamic_state3` | Optional | yes | yes | Disabled when its features are missing |
| `VK_EXT_opacity_micromap` | Optional | no | not listed | Ray tracing; disabled when unsupported |
| `VK_EXT_subgroup_size_control` | Optional | yes | yes | Disabled when unsupported; a warning about subgroup sizes |
| `VK_EXT_swapchain_maintenance1` | Optional | yes | yes | Disabled when unsupported |
| `VK_KHR_present_id` | Optional | yes | yes | Paired with present_wait |
| `VK_KHR_present_wait` | Optional | yes | yes | Disabled when unsupported |
| `VK_KHR_shader_abort` | Optional | no | no | Disabled when unsupported |
| `VK_KHR_shader_abort_message` | Optional | no | not listed | Paired with shader_abort |
| `VK_KHR_shader_clock` | Optional | no | no | Disabled when unsupported |
| `VK_KHR_shader_integer_dot_product` | Optional | yes | yes | Disabled when unsupported |
| `VK_KHR_16bit_storage` | Optional here | yes | yes | Required only with the SM6 switch; CS2 does not set it |
| `VK_KHR_acceleration_structure` | Optional here | no | no | Ray tracing; required only with the full ray tracing switch, which CS2 does not set |
| `VK_KHR_compute_shader_derivatives` | Optional here | no | no | Required only when the game configuration asks; CS2 does not |
| `VK_KHR_deferred_host_operations` | Optional here | yes | yes | Ray tracing; required only with the full ray tracing switch, which CS2 does not set |
| `VK_KHR_fragment_shader_barycentric` | Optional here | yes | no | Required only when the game configuration asks; CS2 does not |
| `VK_KHR_ray_query` | Optional here | no | no | Ray tracing; required only with the full ray tracing switch, which CS2 does not set |
| `VK_KHR_ray_tracing_maintenance1` | Optional here | no | no | Ray tracing; required only with the full ray tracing switch, which CS2 does not set |
| `VK_KHR_ray_tracing_pipeline` | Optional here | no | no | Ray tracing; required only with the full ray tracing switch, which CS2 does not set |
| `VK_KHR_ray_tracing_position_fetch` | Optional here | no | no | Ray tracing; required only with the full ray tracing switch, which CS2 does not set |
| `VK_KHR_shader_float16_int8` | Optional here | yes | yes | Required only with the SM6 switch; CS2 does not set it |
| `VK_AMD_anti_lag` | Vendor | no | no | Vendor-specific (NVIDIA or AMD) |
| `VK_AMD_memory_overallocation_behavior` | Vendor | no | no | Vendor-specific (NVIDIA or AMD) |
| `VK_NVX_binary_import` | Vendor | no | not listed | Vendor-specific (NVIDIA or AMD) |
| `VK_NVX_image_view_handle` | Vendor | no | no | Vendor-specific (NVIDIA or AMD) |
| `VK_NV_device_diagnostic_checkpoints` | Vendor | no | not listed | Vendor-specific (NVIDIA or AMD) |
| `VK_NV_device_diagnostics_config` | Vendor | no | not listed | Vendor-specific (NVIDIA or AMD) |
| `VK_NV_low_latency2` | Vendor | no | not listed | Vendor-specific (NVIDIA or AMD) |
| `VK_NV_ray_tracing_invocation_reorder` | Vendor | no | not listed | Vendor-specific (NVIDIA or AMD) |
| `VK_NV_ray_tracing_validation` | Vendor | no | not listed | Vendor-specific (NVIDIA or AMD) |
| `VK_EXT_full_screen_exclusive` | Not applicable | no | not listed | Windows only |
| `VK_EXT_extended_dynamic_state_2` | Not an extension | no | not listed | A misspelling inside a diagnostic |

Totals: MoltenVK lists 38 of the 72 names and KosmicKrisp 38, with 36 in common.
MoltenVK alone lists `VK_KHR_fragment_shader_barycentric` and
`VK_GOOGLE_display_timing`; KosmicKrisp alone lists `VK_EXT_conditional_rendering`
and `VK_EXT_mutable_descriptor_type`.

**Finding 2.** Nothing the game configuration requires is missing from either driver
on paper. Neither driver lists 31 of the extensions. Of those, 22 are optional by the
renderer's own diagnostics or configuration switches, vendor-specific, or
Windows-only; two are the pipeline library pair of Finding 3; and seven have a role
that is not visible statically: `VK_EXT_memory_priority`, `VK_EXT_mesh_shader`,
`VK_EXT_pageable_device_local_memory`, `VK_EXT_validation_cache`,
`VK_KHR_device_fault`, `VK_KHR_fragment_shading_rate`, and
`VK_KHR_pipeline_executable_properties`. Renderers normally treat all seven as
optional (memory hints, diagnostics, and capabilities Apple GPUs lack), but for this
renderer that is INFERRED.

**Finding 3.** Neither driver offers `VK_EXT_graphics_pipeline_library`, which Valve
recommended for the Linux build at launch
([Phoronix](https://www.phoronix.com/news/Counter-Strike-2-Linux)). The game runs
without it on Linux drivers that lack it, at the cost of shader compilation stutter.
(REPORTED) On Metal that cost comes on top of the driver's own shader translation.

**Finding 4.** The renderer creates its surface through SDL
(`SDL_Vulkan_CreateSurface`). With a macOS SDL3 answering that call, the surface is a
Metal layer and the instance extension is `VK_EXT_metal_surface`, which both drivers
provide. This matches the substitution table in [ARCHITECTURE.md](ARCHITECTURE.md).
(INFERRED from REPORTED strings.)

## Features

The diagnostics name few core features, all with fallbacks: non-solid fill modes
(falls back to solid), variable pointers (a warning), and shader clip distances
(logged per device). The features the renderer enables without comment are not
visible statically. Two matter for a Mac and stay **UNKNOWN** until measured:

- BC texture compression, which the game's textures are expected to use. Apple
  silicon Macs support it in Metal; what each driver reports is in `vulkaninfo`.
- The subgroup operation bits of Finding 1.

## The two drivers as hosts

| | MoltenVK | KosmicKrisp |
| --- | --- | --- |
| Design | Vulkan on Metal through SPIRV-Cross | Mesa driver on Metal 4 |
| Vulkan version | 1.4 (1.4.2 released 20 July 2026) | Conformant to 1.4 |
| Requirements | macOS and iOS, Intel and Apple silicon | Apple silicon, macOS 26 or newer |
| x86-64 build for a Rosetta process | Distributed as a universal library | **UNKNOWN**: documented for Apple silicon only |
| Known caveat | Portability subset: not a conformant implementation | The driver page says M1 and M2 chips see more issues than newer chips |

The last row of facts matters here: the development machine is an M1 Max, and the
host process is x86-64. Whether KosmicKrisp can be built for and loaded into an
x86-64 process at all is open question Q-6, which must be measured before any
renderer work. MoltenVK is the fallback with a known x86-64 build.

## What is still open, and which step closes it

| Open | Closed by |
| --- | --- |
| The extension list of the Linux module, against this Windows-derived one | A local static scan of the Linux renderer module |
| Subgroup operation bits, BC compression, and every limit, per driver, on this Mac | `vulkaninfo --json` for each driver, from the Vulkan SDK for macOS |
| The extensions and features the renderer actually enables, and the ones it refuses to start without | The host logging the instance and device creation calls it forwards |
| An x86-64 Vulkan driver inside a Rosetta process (Q-6) | A probe under `spikes/` |

## Reproduce

```bash
# CS2 side: extension names known to the renderer module (public mirror)
curl -sSfL https://raw.githubusercontent.com/SteamDatabase/GameTracking-CS2/master/game/bin/win64/rendersystemvulkan_strings.txt \
  | grep -oE '\bVK_[A-Z0-9]+_[a-z0-9_]+\b' | sort -u

# Driver side
curl -sSfL https://raw.githubusercontent.com/KhronosGroup/MoltenVK/main/Docs/MoltenVK_Runtime_UserGuide.md \
  | grep -oE '\bVK_[A-Z0-9]+_[a-z0-9_]+\b' | sort -u
curl -sSfL https://gitlab.freedesktop.org/mesa/mesa/-/raw/main/docs/features.txt | grep -E 'VK_.*kk|all DONE.*kk'
```
