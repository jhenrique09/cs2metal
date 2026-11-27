use cs2metal_vulkan::{DriverReport, is_instance_extension, markdown, scan_names, version_string};

/// Bytes as a renderer module carries them: NUL-terminated strings among code.
fn module_bytes() -> Vec<u8> {
    let mut b = vec![0x48, 0x89, 0xe5, 0];
    for s in [
        "VK_KHR_swapchain",
        "VK_EXT_descriptor_indexing",
        "VK_EXT_metal_surface",
        "VK_EXT_graphics_pipeline_library",
        "vkCreateDevice",
        "vkCmdDraw",
        "VK_SUCCESS",                        // an enum value, not an extension
        "VK_KHR_SWAPCHAIN_EXTENSION_NAME",   // a macro name, not an extension
        "vulkan",                            // not a command
        "Requires VK_KHR_swapchain, got %s", // a name inside a message
    ] {
        b.extend_from_slice(s.as_bytes());
        b.push(0);
    }
    b
}

const REPORT: &str = r#"{
  "capabilities": { "device": {
    "extensions": { "VK_KHR_swapchain": 70, "VK_EXT_descriptor_indexing": 2 },
    "features": {
      "VkPhysicalDeviceFeatures": { "textureCompressionBC": true },
      "VkPhysicalDeviceVulkan12Features": { "descriptorIndexing": true }
    },
    "properties": {
      "VkPhysicalDeviceProperties": { "deviceName": "Test GPU", "apiVersion": 4211045 },
      "VkPhysicalDeviceVulkan11Properties": {
        "subgroupSize": 32,
        "subgroupSupportedOperations": ["VK_SUBGROUP_FEATURE_BASIC_BIT", "VK_SUBGROUP_FEATURE_BALLOT_BIT"],
        "subgroupSupportedStages": ["VK_SHADER_STAGE_COMPUTE_BIT"]
      },
      "VkPhysicalDeviceVulkan12Properties": { "driverName": "TestVK", "driverInfo": "1.0" }
    }
  } }
}"#;

#[test]
fn finds_extension_and_command_names_as_whole_tokens() {
    let names = scan_names(&module_bytes());
    let extensions: Vec<_> = names.extensions.iter().map(String::as_str).collect();
    assert_eq!(
        extensions,
        [
            "VK_EXT_descriptor_indexing",
            "VK_EXT_graphics_pipeline_library",
            "VK_EXT_metal_surface",
            "VK_KHR_swapchain",
        ]
    );
    let commands: Vec<_> = names.commands.iter().map(String::as_str).collect();
    assert_eq!(commands, ["vkCmdDraw", "vkCreateDevice"]);
}

#[test]
fn reads_a_vulkaninfo_report() {
    let d = DriverReport::from_vulkaninfo("test", REPORT).expect("report");
    assert_eq!(d.device_name, "Test GPU");
    assert_eq!(d.driver_name, "TestVK");
    assert_eq!(d.api_version, "1.4.357");
    assert_eq!(d.subgroup_size, Some(32));
    assert_eq!(d.subgroup_operations.len(), 2);
    assert_eq!(d.texture_compression_bc, Some(true));
    assert_eq!(d.descriptor_indexing, Some(true));
    assert_eq!(d.offers("VK_KHR_swapchain"), Some(true));
    assert_eq!(d.offers("VK_EXT_graphics_pipeline_library"), Some(false));
    assert_eq!(d.offers("VK_EXT_metal_surface"), None, "instance level");
}

#[test]
fn rejects_files_that_are_not_vulkaninfo_reports() {
    assert!(DriverReport::from_vulkaninfo("x", "{}").is_err());
    assert!(DriverReport::from_vulkaninfo("x", "not json").is_err());
}

#[test]
fn decodes_packed_versions() {
    assert_eq!(version_string((1 << 22) | (4 << 12) | 363), "1.4.363");
    assert!(is_instance_extension("VK_KHR_surface"));
    assert!(!is_instance_extension("VK_KHR_swapchain"));
}

#[test]
fn crosses_names_with_drivers() {
    let names = scan_names(&module_bytes());
    let d = DriverReport::from_vulkaninfo("TestVK x86_64", REPORT).expect("report");
    let report = markdown("librender.so", &names, &[d]);
    assert!(report.contains("| Module's device extensions offered | 2 of 3 |"));
    assert!(report.contains("| `VK_EXT_graphics_pipeline_library` | device | no |"));
    assert!(report.contains("| `VK_EXT_metal_surface` | instance | not in device report |"));
    assert!(report.contains("| Subgroup operations | basic, ballot |"));
}

#[test]
fn drops_and_names_pieces_of_strings_built_in_chunks() {
    // The compiler stored one name whole and another only as a 16-byte piece.
    let bytes = b"\0VK_KHR_get_physical_device_properties2\0VK_KHR_get_physi\0VK_KHR_depth_ste\0";
    let names = scan_names(bytes);
    let fragments: Vec<_> = names.fragments.iter().map(String::as_str).collect();
    assert_eq!(fragments, ["VK_KHR_get_physi"]);

    // Real names that prefix longer real names are kept.
    let real = b"\0VK_KHR_swapchain\0VK_KHR_swapchain_mutable_format\0VK_KHR_shader_abort\0VK_KHR_shader_abort_message\0VK_EXT_extended_dynamic_state\0VK_EXT_extended_dynamic_state2\0";
    let kept = scan_names(real);
    assert!(kept.fragments.is_empty());
    assert_eq!(kept.extensions.len(), 6);
    let report_json = REPORT.replace(
        r#""VK_KHR_swapchain": 70"#,
        r#""VK_KHR_swapchain": 70, "VK_KHR_depth_stencil_resolve": 1"#,
    );
    let d = DriverReport::from_vulkaninfo("d", &report_json).expect("report");
    let report = markdown("m", &names, &[d]);
    assert!(
        report.contains("| `VK_KHR_depth_ste` | truncated: `VK_KHR_depth_stencil_resolve` | no |")
    );
    assert!(report.contains("| Module's device extensions offered | 0 of 0 |"));
}
