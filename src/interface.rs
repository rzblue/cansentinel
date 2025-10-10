//! CAN interface info

use glob::Pattern;
use nix::Result;
use std::collections::HashSet;
/// Information about a CAN interface
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanInterfaceInfo {
    /// Interface index (from kernel)
    pub idx: u32,
    /// Interface name (e.g., "can0")
    pub name: String,
}

impl CanInterfaceInfo {
    /// Create a new CanInterfaceInfo by looking up the interface index
    pub fn new(name: &str) -> Result<Self> {
        let idx = nix::net::if_::if_nametoindex(name)?;
        Ok(Self {
            idx,
            name: name.to_string(),
        })
    }
}

/// Enumerate all available network interfaces and return their info
pub fn get_available_interfaces() -> Vec<CanInterfaceInfo> {
    use nix::net::if_::if_nameindex;

    match if_nameindex() {
        Ok(interfaces) => interfaces
            .into_iter()
            .filter_map(|iface| {
                iface.name().to_str().ok().map(|name| CanInterfaceInfo {
                    idx: iface.index(),
                    name: name.to_string(),
                })
            })
            .collect(),
        Err(e) => {
            println!("Warning: Failed to enumerate interfaces: {}", e);
            Vec::new()
        }
    }
}

/// Match interface patterns against available interfaces and return matching interface info
pub fn find_matching_interfaces(patterns: Vec<String>) -> Vec<CanInterfaceInfo> {
    if patterns.is_empty() {
        return Vec::new();
    }

    let available_interfaces = get_available_interfaces();
    let mut matched_interfaces = HashSet::new();

    for pattern_str in &patterns {
        // First try exact match
        if let Some(interface) = available_interfaces
            .iter()
            .find(|iface| iface.name == *pattern_str)
        {
            matched_interfaces.insert(interface.clone());
            continue;
        }

        // Then try glob pattern matching
        if let Ok(pattern) = Pattern::new(pattern_str) {
            for interface in &available_interfaces {
                if pattern.matches(&interface.name) {
                    matched_interfaces.insert(interface.clone());
                }
            }
        } else {
            println!("Warning: Invalid glob pattern '{}'", pattern_str);
        }
    }

    let mut result: Vec<CanInterfaceInfo> = matched_interfaces.into_iter().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}
