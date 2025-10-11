//! Configuration types

use std::time::Duration;

/// Configuration for cansentinel
#[derive(Debug, Clone)]
pub struct Config {
    /// Delay before restarting a bus-off interface
    pub bus_off_delay: Duration,
    /// List of CAN interface names to monitor
    pub interface_names: Vec<String>,
}

impl Config {
    pub fn new(bus_off_delay: Duration, interface_names: Vec<String>) -> Self {
        Self {
            bus_off_delay,
            interface_names,
        }
    }
}
