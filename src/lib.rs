pub mod config;
pub mod events;
pub mod interface;
pub mod monitoring;
pub mod restart;

pub use config::Config;
pub use events::{BusEvent, BusEventType};
pub use interface::{CanInterfaceInfo, find_matching_interfaces, get_available_interfaces};
pub use monitoring::{monitor_interface_errors, monitor_netlink};
pub use restart::RestartManager;

pub mod consts {
    /// Hardware type for CAN interfaces in netlink
    pub const ARPHRD_CAN: u16 = 280;

    /// Default netlink group for link state changes
    pub const RTNLGRP_LINK: u32 = 1;
}
