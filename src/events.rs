//! Event types

use crate::interface::CanInterfaceInfo;

/// Types of CAN bus events we care about
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusEventType {
    /// Bus has gone into bus-off state and needs restart
    BusOff,
    /// Bus has been restarted and is active again
    Restart,
    /// Interface has gone down(?)
    Stopped,
}

#[derive(Debug, Clone)]
pub enum BusEventSource {
    /// CAN socket error frame
    ErrorFrame(socketcan::CanErrorFrame),
    /// CANState from netlink linkinfo attribute
    StateUpdate(socketcan::nl::CanState),
}

/// Unified event for CAN bus state changes
///
/// This represents any significant bus state change that occurred,
/// whether detected via netlink state changes or CAN error frames.
#[derive(Debug, Clone)]
pub struct BusEvent {
    /// Interface that experienced the state change
    pub interface: CanInterfaceInfo,
    /// Type of event that occurred
    pub event_type: BusEventType,
    /// Where the event originated from
    pub event_source: BusEventSource,
}

impl BusEvent {
    /// Create a new bus-off event
    pub fn bus_off(interface: CanInterfaceInfo, event_source: BusEventSource) -> Self {
        Self {
            interface,
            event_type: BusEventType::BusOff,
            event_source,
        }
    }

    /// Create a new restart event
    pub fn restart(interface: CanInterfaceInfo, event_source: BusEventSource) -> Self {
        Self {
            interface,
            event_type: BusEventType::Restart,
            event_source,
        }
    }

    /// Create a new stopped event
    pub fn stopped(interface: CanInterfaceInfo, event_source: BusEventSource) -> Self {
        Self {
            interface,
            event_type: BusEventType::Stopped,
            event_source,
        }
    }

    /// Check if this is a bus-off event
    pub fn is_bus_off(&self) -> bool {
        matches!(self.event_type, BusEventType::BusOff)
    }

    /// Check if this is a restart event  
    pub fn is_restart(&self) -> bool {
        matches!(self.event_type, BusEventType::Restart)
    }

    /// Check if this is a stopped event
    pub fn is_stopped(&self) -> bool {
        matches!(self.event_type, BusEventType::Stopped)
    }
}
