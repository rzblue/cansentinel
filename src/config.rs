//! Configuration types

use std::time::Duration;

/// Configuration for cansentinel
#[derive(Debug, Clone)]
pub struct Config {
    /// Timeout before restarting a bus-off interface
    pub bus_off_timeout: Duration,
    /// List of CAN interface names to monitor
    pub interface_names: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bus_off_timeout: Duration::from_millis(1000),
            interface_names: vec![],
        }
    }
}

impl Config {
    /// Create a new configuration with custom interface names
    pub fn with_interfaces(interface_names: Vec<String>) -> Self {
        Self {
            interface_names,
            ..Default::default()
        }
    }

    /// Set the bus-off timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.bus_off_timeout = timeout;
        self
    }
}
