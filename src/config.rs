//! Configuration types

use crate::CanInterfaceInfo;
use std::time::Duration;

/// Configuration for cansentinel
#[derive(Debug, Clone)]
pub struct Config {
    /// Delay before restarting a bus-off interface
    pub bus_off_delay: Duration,
    /// List of CAN interfaces to monitor
    pub interfaces: Vec<CanInterfaceInfo>,
}

impl Config {
    pub fn new(bus_off_delay: Duration, interfaces: Vec<CanInterfaceInfo>) -> Self {
        Self {
            bus_off_delay,
            interfaces,
        }
    }
}
