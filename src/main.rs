//! cansentinel
//!
//! A daemon that monitors CAN interface state changes via netlink and automatically
//! restarts interfaces that enter the bus-off state.

use std::{collections::HashMap, sync::Arc, time::Duration};

use socketcan::async_io::CanSocket;
use socketcan::{CanError, CanErrorFrame, SocketOptions};
use socketcan::{CanFrame, CanInterface, EmbeddedFrame, Frame, nl::CanState};
use tokio::{
    sync::{RwLock, mpsc},
    task::JoinHandle,
    time::sleep,
};

/// Hardware type for CAN interfaces in netlink
const ARPHRD_CAN: u16 = 280;

/// Default netlink group for link state changes
const RTNLGRP_LINK: u32 = 1;

/// Configuration for the daemon
#[derive(Debug, Clone)]
struct Config {
    /// Timeout before restarting a bus-off interface (in seconds)
    bus_off_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bus_off_timeout: Duration::from_millis(500),
        }
    }
}

/// Manages pending restart tasks for CAN interfaces
#[derive(Debug)]
struct RestartManager {
    /// Map of interface index to pending restart task
    pending_tasks: Arc<RwLock<HashMap<u32, JoinHandle<()>>>>,
}

impl RestartManager {
    fn new() -> Self {
        Self {
            pending_tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Schedule a delayed restart for a bus-off interface
    async fn schedule_restart(&self, interface: CanInterfaceInfo, delay: Duration) {
        // Only schedule if there isn't already a pending restart for this interface
        {
            let pending_tasks = self.pending_tasks.read().await;
            if pending_tasks.contains_key(&interface.idx) {
                println!(
                    "{}: restart already scheduled, ignoring duplicate bus-off",
                    interface.name
                );
                return;
            }
        }

        let pending_tasks = Arc::clone(&self.pending_tasks);

        // Spawn a new delayed restart task
        let task = tokio::spawn(async move {
            sleep(delay).await;

            println!("{}: restarting interface", interface.name);

            // Remove this task from pending tasks BEFORE executing restart
            // This prevents race condition with netlink events caused by the restart
            pending_tasks.write().await.remove(&interface.idx);

            let iface = CanInterface::open_iface(interface.idx);
            match iface.restart() {
                Ok(_) => (),
                Err(e) => {
                    println!("{}: restart failed: {}", interface.name, e);
                }
            }
        });

        // Store the task handle
        self.pending_tasks.write().await.insert(interface.idx, task);
    }
}

#[derive(Debug, Clone)]
struct CanInterfaceInfo {
    idx: u32,
    name: String,
}

impl CanInterfaceInfo {
    fn new(name: &str) -> Result<Self, nix::Error> {
        let idx = nix::net::if_::if_nametoindex(name)?;
        println!("Interface {} has index {}", name, idx);
        Ok(Self {
            idx,
            name: name.to_string(),
        })
    }
}

#[derive(Debug)]
struct StateChangeEvent {
    interface: CanInterfaceInfo,
    state: Option<CanState>,
}

#[derive(Debug)]
struct CanErrorEvent {
    interface: CanInterfaceInfo,
    error_frame: CanErrorFrame,
}

#[tokio::main]
async fn main() {
    let config = Config::default();
    let restart_manager = Arc::new(RestartManager::new());

    // Hardcoded interface list for now
    let interface_names = vec!["can_s0", "can_s1"]; // TODO: Make this configurable

    println!("Starting CAN interface monitor daemon");
    println!("Bus-off timeout: {:?}", config.bus_off_timeout);
    println!("Monitoring interfaces: {:?}", interface_names);

    // Look up interface indices early - only proceed with interfaces that exist
    let mut interfaces = Vec::new();
    for name in &interface_names {
        match CanInterfaceInfo::new(name) {
            Ok(interface) => {
                interfaces.push(interface);
            }
            Err(e) => {
                println!("Error: Interface {} does not exist: {}", name, e);
                println!("Skipping monitoring for {}", name);
            }
        }
    }

    if interfaces.is_empty() {
        println!("No valid CAN interfaces found. Exiting.");
        return;
    }

    // Create a unified channel for bus-off detection from both sources
    let (bus_off_tx, mut bus_off_rx) = mpsc::unbounded_channel::<StateChangeEvent>();

    // Start netlink monitoring
    let netlink_tx = bus_off_tx.clone();
    let netlink_handle = tokio::task::spawn_blocking(move || {
        netlink_monitoring_loop(netlink_tx);
    });

    // Start CAN error frame monitoring for each interface
    let mut error_handles = Vec::new();
    for interface in &interfaces {
        let interface = interface.clone();
        let error_tx = bus_off_tx.clone();
        let handle = tokio::spawn(async move {
            monitor_interface_errors(error_tx, interface).await;
        });
        error_handles.push(handle);
    }

    // Main event loop - handle bus-off events from both sources
    while let Some(event) = bus_off_rx.recv().await {
        if let Some(CanState::BusOff) = event.state {
            println!(
                "{}: bus_off, scheduling restart in {:?}",
                event.interface.name, config.bus_off_timeout
            );
            let restart_mgr = Arc::clone(&restart_manager);
            restart_mgr
                .schedule_restart(event.interface, config.bus_off_timeout)
                .await;
        }
    }

    // Clean up monitoring tasks
    println!("Shutting down monitoring tasks...");

    // Abort error monitoring tasks
    for handle in error_handles {
        handle.abort();
    }

    // Abort netlink monitoring
    netlink_handle.abort();

    println!("CAN interface monitor daemon shutting down");
}

/// Monitor error frames on a specific CAN interface
async fn monitor_interface_errors(
    tx: mpsc::UnboundedSender<StateChangeEvent>,
    interface: CanInterfaceInfo,
) {
    loop {
        match CanSocket::open(&interface.name) {
            Ok(socket) => {
                // Configure socket to receive only error frames and drop all regular data frames
                if let Err(e) = socket
                    .set_error_filter_accept_all()
                    .and_then(|_| socket.set_filter_drop_all())
                {
                    println!(
                        "Failed to configure socket filters for {}: {}",
                        interface.name, e
                    );
                    continue;
                }

                println!("Started error monitoring for interface: {}", interface.name);
                // Monitor for error frames using async read
                loop {
                    match socket.read_frame().await {
                        Ok(CanFrame::Error(frame)) => {
                            let event = CanErrorEvent {
                                interface: interface.clone(),
                                error_frame: frame,
                            };
                            log_can_error(&event);

                            if let CanError::BusOff = frame.into_error() {
                                let event = StateChangeEvent {
                                    interface: interface.clone(),
                                    state: Some(CanState::BusOff),
                                };
                                if tx.send(event).is_err() {
                                    println!("Channel closed, stopping monitoring");
                                    return;
                                }
                            }
                        }
                        Ok(_) => (),
                        Err(e) => {
                            println!("Error reading from {}: {}", interface.name, e);
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                println!("Failed to open error socket for {}: {}", interface.name, e);
            }
        }

        // Wait before retrying if the socket failed
        println!(
            "Retrying error monitoring for {} in 5 seconds...",
            interface.name
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Log CAN error events
fn log_can_error(event: &CanErrorEvent) {
    let frame = &event.error_frame;

    println!(
        "CAN ERROR on {}: ID=0x{:03X}, DLC={}, Data={:02X?}",
        event.interface.name,
        frame.raw_id(),
        frame.len(),
        frame.data()
    );

    // Additional error frame analysis based on CAN error frame format
    use socketcan::errors::CanError::*;
    match frame.into_error() {
        TransmitTimeout => println!("  -> TX timeout (bus-off recovery in progress)"),
        LostArbitration(_) => println!("  -> Lost arbitration"),
        ControllerProblem(_) => println!("  -> Controller problems"),
        ProtocolViolation {
            vtype: _,
            location: _,
        } => println!("  -> Protocol violations"),
        TransceiverError => println!("  -> Transceiver status"),
        NoAck => println!("  -> No acknowledgment on transmission"),
        BusOff => println!("  -> Bus off"),
        BusError => println!("  -> Bus error"),
        Restarted => println!("  -> Bus restarted"),
        Unknown(0x204) => println!("  -> Error counters"),
        _ => println!("  -> Other error condition"),
    }
}

/// Runs the blocking netlink monitoring loop
fn netlink_monitoring_loop(tx: mpsc::UnboundedSender<StateChangeEvent>) {
    use neli::{
        consts::{
            rtnl::{Ifla, Rtm},
            socket::NlFamily,
        },
        rtnl::Ifinfomsg,
        socket,
    };
    use socketcan::InterfaceCanParams;
    use std::process;

    let mut s =
        socket::NlSocketHandle::connect(NlFamily::Route, Some(process::id() + 1), &[RTNLGRP_LINK])
            .expect("Failed to create netlink socket");

    for next in s.iter::<Rtm, Ifinfomsg>(true) {
        match next {
            Ok(msg) => {
                if let Ok(msg_payload) = msg.get_payload() {
                    // Only process CAN interfaces
                    if u16::from(msg_payload.ifi_type) == ARPHRD_CAN {
                        let handle = msg_payload.rtattrs.get_attr_handle();
                        let idx = msg_payload.ifi_index as u32;
                        let name = handle
                            .get_attr_payload_as_with_len::<String>(Ifla::Ifname)
                            .unwrap_or_else(|_| "Unknown".to_string());

                        let state = handle
                            .get_attribute(Ifla::Linkinfo)
                            .and_then(|attr| InterfaceCanParams::try_from(attr).ok()?.state);

                        // Only send bus-off events
                        if let Some(CanState::BusOff) = state {
                            let interface = CanInterfaceInfo { idx, name };
                            let event = StateChangeEvent {
                                interface,
                                state: Some(CanState::BusOff),
                            };
                            if tx.send(event).is_err() {
                                println!("Channel closed, stopping netlink monitoring");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                println!("\nNetlink error: {:?}\n", e);
                break;
            }
        }
    }
    println!("Netlink monitoring thread finished");
}
