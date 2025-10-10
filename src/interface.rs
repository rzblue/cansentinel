//! CAN interface info

use nix::Result;
/// Information about a CAN interface
#[derive(Debug, Clone)]
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
