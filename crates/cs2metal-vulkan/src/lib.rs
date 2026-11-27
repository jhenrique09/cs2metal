//! What the CS2 renderer knows about Vulkan, against what each driver offers.
//!
//! [`scan_names`] lists the Vulkan extension and command names a module carries as
//! strings: the names it can ask for, a superset of what it enables on one machine.
//! [`DriverReport::from_vulkaninfo`] reads the JSON that `vulkaninfo --json` writes
//! for one driver, and [`markdown`] crosses the two. Everything here reads files;
//! nothing loads a driver.
//!
//! Names of Vulkan extensions and commands are public API names, so reports may cite
//! them; they are still derived from a local file and stay in `reports/`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::Value;

/// The Vulkan names a module carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Names {
    /// Extension names: `VK_` + an upper-case vendor tag + `_` + a lower-case name.
    pub extensions: BTreeSet<String>,
    /// Command names: `vk` + an upper-case letter + letters and digits.
    pub commands: BTreeSet<String>,
    /// Extension-like tokens that are pieces of a longer name in the module: the
    /// compiler built the string in 8- or 16-byte chunks, so the token's length is
    /// a multiple of 8 and it ends inside a word of the longer name.
    pub fragments: BTreeSet<String>,
}

fn is_extension(token: &str) -> bool {
    let Some(rest) = token.strip_prefix("VK_") else {
        return false;
    };
    let Some((vendor, name)) = rest.split_once('_') else {
        return false;
    };
    !vendor.is_empty()
        && vendor
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_command(token: &str) -> bool {
    let Some(rest) = token.strip_prefix("vk") else {
        return false;
    };
    rest.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
        && rest.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Every Vulkan extension and command name in `bytes`, found as whole tokens of
/// ASCII letters, digits, and underscores.
pub fn scan_names(bytes: &[u8]) -> Names {
    let mut names = Names::default();
    let word = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_';
    for token in bytes.split(|b| !word(b)).filter(|t| t.len() > 3) {
        // Tokens are ASCII by construction.
        let token = String::from_utf8_lossy(token);
        if is_extension(&token) {
            names.extensions.insert(token.into_owned());
        } else if is_command(&token) {
            names.commands.insert(token.into_owned());
        }
    }
    let fragments: BTreeSet<String> = names
        .extensions
        .iter()
        .filter(|e| names.extensions.iter().any(|o| is_piece_of(e, o)))
        .cloned()
        .collect();
    names.extensions.retain(|e| !fragments.contains(e));
    names.fragments = fragments;
    names
}

/// Whether `token` is a chunk-sized piece cut from inside a word of `full`. A real
/// name may also prefix a longer one (`VK_KHR_swapchain` and
/// `VK_KHR_swapchain_mutable_format`), but then the longer one continues with `_`.
fn is_piece_of(token: &str, full: &str) -> bool {
    token.len() % 8 == 0
        && full.len() > token.len()
        && full.starts_with(token)
        && full.as_bytes()[token.len()] != b'_'
}

/// The known extension a truncated token begins, when exactly one does. `known`
/// is every name the drivers report; instance extensions are added here.
fn completion<'k>(token: &str, known: &BTreeSet<&'k str>) -> Option<&'k str> {
    let mut matches = known.iter().filter(|k| is_piece_of(token, k));
    let first = matches.next().copied();
    first.filter(|_| matches.next().is_none())
}

/// Instance extensions: the instance, not the device, reports them, so a device
/// report cannot say whether a driver offers them. From the Vulkan registry.
pub fn is_instance_extension(name: &str) -> bool {
    INSTANCE_EXTENSIONS.contains(&name)
}

const INSTANCE_EXTENSIONS: [&str; 24] = [
    "VK_KHR_surface",
    "VK_KHR_display",
    "VK_KHR_xlib_surface",
    "VK_KHR_xcb_surface",
    "VK_KHR_wayland_surface",
    "VK_KHR_win32_surface",
    "VK_KHR_get_physical_device_properties2",
    "VK_KHR_get_surface_capabilities2",
    "VK_KHR_get_display_properties2",
    "VK_KHR_device_group_creation",
    "VK_KHR_external_memory_capabilities",
    "VK_KHR_external_semaphore_capabilities",
    "VK_KHR_external_fence_capabilities",
    "VK_KHR_surface_protected_capabilities",
    "VK_KHR_portability_enumeration",
    "VK_EXT_debug_utils",
    "VK_EXT_debug_report",
    "VK_EXT_metal_surface",
    "VK_EXT_headless_surface",
    "VK_EXT_surface_maintenance1",
    "VK_EXT_swapchain_colorspace",
    "VK_EXT_validation_features",
    "VK_EXT_validation_flags",
    "VK_EXT_layer_settings",
];

/// What one driver reports on one machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DriverReport {
    /// A label chosen by the caller, such as `MoltenVK x86_64`.
    pub label: String,
    pub device_name: String,
    pub driver_name: String,
    pub driver_info: String,
    /// `major.minor.patch` of the device's Vulkan version.
    pub api_version: String,
    pub device_extensions: BTreeSet<String>,
    pub subgroup_size: Option<u64>,
    pub subgroup_operations: Vec<String>,
    pub subgroup_stages: Vec<String>,
    pub texture_compression_bc: Option<bool>,
    pub descriptor_indexing: Option<bool>,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// `major.minor.patch` of a packed Vulkan version number.
pub fn version_string(packed: u64) -> String {
    format!(
        "{}.{}.{}",
        (packed >> 22) & 0x7f,
        (packed >> 12) & 0x3ff,
        packed & 0xfff
    )
}

impl DriverReport {
    /// Reads the JSON that `vulkaninfo --json` writes for one device.
    pub fn from_vulkaninfo(label: &str, json: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let device = &root["capabilities"]["device"];
        if device.is_null() {
            return Err("no capabilities.device object: not a vulkaninfo --json report".into());
        }
        let props = &device["properties"];
        let core = &props["VkPhysicalDeviceProperties"];
        let v11 = &props["VkPhysicalDeviceVulkan11Properties"];
        let v12 = &props["VkPhysicalDeviceVulkan12Properties"];
        let features = &device["features"];
        let extensions = device["extensions"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        Ok(Self {
            label: label.to_owned(),
            device_name: text(&core["deviceName"]),
            driver_name: text(&v12["driverName"]),
            driver_info: text(&v12["driverInfo"]),
            api_version: core["apiVersion"]
                .as_u64()
                .map(version_string)
                .unwrap_or_default(),
            device_extensions: extensions,
            subgroup_size: v11["subgroupSize"].as_u64(),
            subgroup_operations: strings(&v11["subgroupSupportedOperations"]),
            subgroup_stages: strings(&v11["subgroupSupportedStages"]),
            texture_compression_bc: features["VkPhysicalDeviceFeatures"]["textureCompressionBC"]
                .as_bool(),
            descriptor_indexing: features["VkPhysicalDeviceVulkan12Features"]["descriptorIndexing"]
                .as_bool(),
        })
    }

    /// Whether the driver offers a device extension; `None` for instance extensions,
    /// which a device report does not list.
    pub fn offers(&self, extension: &str) -> Option<bool> {
        (!is_instance_extension(extension)).then(|| self.device_extensions.contains(extension))
    }
}

/// Short names: `VK_SUBGROUP_FEATURE_BALLOT_BIT` becomes `ballot`.
fn short(flag: &str, prefix: &str) -> String {
    flag.trim_start_matches(prefix)
        .trim_end_matches("_BIT")
        .to_ascii_lowercase()
}

fn yes_no(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        Some(false) => "**no**",
        None => "unknown",
    }
}

/// The renderer's names against each driver, as Markdown.
pub fn markdown(module: &str, names: &Names, drivers: &[DriverReport]) -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(w, "# Vulkan: the renderer's names against the drivers\n");
    let _ = writeln!(
        w,
        "Generated by `cs2metal vulkan compare` from `{module}` and `vulkaninfo` reports. \
         Keep it in `reports/`.\n"
    );
    let _ = writeln!(w, "## The module\n");
    let _ = writeln!(w, "| Measure | Count |\n| --- | --- |");
    let instance = names
        .extensions
        .iter()
        .filter(|e| is_instance_extension(e))
        .count();
    let _ = writeln!(w, "| Extension names | {} |", names.extensions.len());
    let _ = writeln!(w, "| Of which instance extensions | {instance} |");
    let _ = writeln!(w, "| Command names | {} |", names.commands.len());
    let _ = writeln!(
        w,
        "| Fragments of longer names, dropped | {} |",
        names.fragments.len()
    );

    if drivers.is_empty() {
        let _ = writeln!(w, "\n## Extension names\n");
        for e in &names.extensions {
            let _ = writeln!(w, "- `{e}`");
        }
        return out;
    }

    // Names no driver offers that begin a known name are truncated pieces of it.
    let mut known: BTreeSet<&str> = drivers
        .iter()
        .flat_map(|d| d.device_extensions.iter().map(String::as_str))
        .collect();
    known.extend(INSTANCE_EXTENSIONS);
    let truncated: BTreeMap<&str, &str> = names
        .extensions
        .iter()
        .filter(|e| !is_instance_extension(e))
        .filter(|e| !drivers.iter().any(|d| d.offers(e) == Some(true)))
        .filter_map(|e| completion(e, &known).map(|full| (e.as_str(), full)))
        .collect();
    let device_names = names.extensions.len() - instance - truncated.len();

    let _ = writeln!(w, "\n## The drivers\n");
    let header: Vec<&str> = drivers.iter().map(|d| d.label.as_str()).collect();
    let _ = writeln!(w, "| | {} |", header.join(" | "));
    let _ = writeln!(w, "| --- |{}", " --- |".repeat(drivers.len()));
    let row = |w: &mut String, name: &str, cell: &dyn Fn(&DriverReport) -> String| {
        let cells: Vec<String> = drivers.iter().map(cell).collect();
        let _ = writeln!(w, "| {name} | {} |", cells.join(" | "));
    };
    row(w, "Device", &|d| d.device_name.clone());
    row(w, "Driver", &|d| {
        format!("{} {}", d.driver_name, d.driver_info)
    });
    row(w, "Vulkan version", &|d| d.api_version.clone());
    row(w, "Device extensions", &|d| {
        d.device_extensions.len().to_string()
    });
    row(w, "Subgroup size", &|d| {
        d.subgroup_size.map_or("unknown".into(), |s| s.to_string())
    });
    row(w, "Subgroup operations", &|d| {
        let ops: Vec<String> = d
            .subgroup_operations
            .iter()
            .map(|o| short(o, "VK_SUBGROUP_FEATURE_"))
            .collect();
        ops.join(", ")
    });
    row(w, "Subgroup stages", &|d| {
        let stages: Vec<String> = d
            .subgroup_stages
            .iter()
            .map(|s| short(s, "VK_SHADER_STAGE_"))
            .collect();
        stages.join(", ")
    });
    row(w, "BC texture compression", &|d| {
        yes_no(d.texture_compression_bc).into()
    });
    row(w, "Descriptor indexing", &|d| {
        yes_no(d.descriptor_indexing).into()
    });
    let offered = |d: &DriverReport| {
        names
            .extensions
            .iter()
            .filter(|e| d.offers(e) == Some(true))
            .count()
    };
    row(w, "Module's device extensions offered", &|d| {
        format!("{} of {device_names}", offered(d))
    });

    let _ = writeln!(w, "\n## Extension names\n");
    let _ = writeln!(w, "| Extension | Level | {} |", header.join(" | "));
    let _ = writeln!(w, "| --- | --- |{}", " --- |".repeat(drivers.len()));
    for e in &names.extensions {
        let level = match truncated.get(e.as_str()) {
            Some(full) => format!("truncated: `{full}`"),
            None if is_instance_extension(e) => "instance".to_owned(),
            None => "device".to_owned(),
        };
        let cells: Vec<&str> = drivers
            .iter()
            .map(|d| match d.offers(e) {
                Some(true) => "yes",
                Some(false) => "no",
                None => "not in device report",
            })
            .collect();
        let _ = writeln!(w, "| `{e}` | {level} | {} |", cells.join(" | "));
    }
    out
}
