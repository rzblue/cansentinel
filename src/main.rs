use std::{collections::HashMap, sync::Arc, time::Duration};

use socketcan::{CanInterface, nl::CanState};
use tokio::{
    sync::{RwLock, mpsc},
    task::JoinHandle,
    time::sleep,
};

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
    async fn schedule_restart(&self, interface_idx: u32, interface_name: String, delay: Duration) {
        // Cancel any existing task for this interface
        self.cancel_restart(interface_idx).await;

        let pending_tasks = Arc::clone(&self.pending_tasks);

        // Spawn a new delayed restart task
        let task = tokio::spawn(async move {
            println!(
                "Scheduling restart for interface {} ({}) in {:?}",
                interface_name, interface_idx, delay
            );

            sleep(delay).await;

            println!(
                "Executing restart for interface {} ({})",
                interface_name, interface_idx
            );

            match CanInterface::open(&interface_name) {
                Ok(iface) => match iface.restart() {
                    Ok(()) => {
                        println!(
                            "Successfully restarted interface {} ({})",
                            interface_name, interface_idx
                        );
                    }
                    Err(e) => {
                        println!(
                            "Interface {} ({}) restart failed: {}",
                            interface_name, interface_idx, e
                        );
                    }
                },
                Err(e) => {
                    println!(
                        "Failed to open interface {} ({}) for restart: {}",
                        interface_name, interface_idx, e
                    );
                }
            }

            // Remove this task from pending tasks when completed
            pending_tasks.write().await.remove(&interface_idx);
        });

        // Store the task handle
        self.pending_tasks.write().await.insert(interface_idx, task);
    }

    /// Cancel any pending restart for an interface
    async fn cancel_restart(&self, interface_idx: u32) {
        if let Some(task) = self.pending_tasks.write().await.remove(&interface_idx) {
            task.abort();
            println!("Cancelled pending restart for interface {}", interface_idx);
        }
    }

    /// Cancel any pending restart for an interface with detailed logging
    async fn cancel_restart_with_reason(
        &self,
        interface_idx: u32,
        interface_name: &str,
        new_state: CanState,
    ) {
        if let Some(task) = self.pending_tasks.write().await.remove(&interface_idx) {
            task.abort();
            println!(
                "Cancelled pending restart for interface {} ({}): state changed to {:?} (likely handled by another process)",
                interface_name, interface_idx, new_state
            );
        }
    }
}

#[derive(Debug)]
struct NetlinkEvent {
    interface_idx: u32,
    interface_name: String,
    state: Option<CanState>,
}

#[tokio::main]
async fn main() {
    let config = Config::default();
    let restart_manager = RestartManager::new();

    println!("Starting CAN interface monitor daemon");
    println!("Bus-off timeout: {:?}", config.bus_off_timeout);

    // Create a channel for communication between the blocking netlink thread and async main loop
    let (tx, mut rx) = mpsc::unbounded_channel::<NetlinkEvent>();

    // Spawn the blocking netlink iteration in a separate thread
    let netlink_handle = tokio::task::spawn_blocking(move || {
        netlink_monitoring_loop(tx);
    });

    // Main async loop processes events from the netlink thread
    while let Some(event) = rx.recv().await {
        println!(
            "Interface {} ({}): {:?}",
            event.interface_name, event.interface_idx, event.state
        );

        if let Some(state) = event.state {
            match state {
                CanState::BusOff => {
                    restart_manager
                        .schedule_restart(
                            event.interface_idx,
                            event.interface_name,
                            config.bus_off_timeout,
                        )
                        .await;
                }
                _ => {
                    // Cancel any pending restart if the interface comes back online
                    restart_manager
                        .cancel_restart_with_reason(
                            event.interface_idx,
                            &event.interface_name,
                            state,
                        )
                        .await;
                }
            }
        }
    }

    // Wait for the netlink thread to finish (though it should run indefinitely)
    if let Err(e) = netlink_handle.await {
        println!("Netlink thread error: {:?}", e);
    }

    println!("CAN interface monitor daemon shutting down");
}

/// Runs the blocking netlink monitoring loop
fn netlink_monitoring_loop(tx: mpsc::UnboundedSender<NetlinkEvent>) {
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

    let mut s = socket::NlSocketHandle::connect(
        NlFamily::Route,
        Some(process::id() + 1),
        /* RTNLGRP_LINK */ &[1],
    )
    .expect("Creating netlink socket failed");

    for next in s.iter::<Rtm, Ifinfomsg>(true) {
        match next {
            Ok(msg) => {
                if let Ok(msg_payload) = msg.get_payload() {
                    // Only process CAN interfaces (ARPHRD_CAN = 280)
                    if u16::from(msg_payload.ifi_type) == 280 {
                        let handle = msg_payload.rtattrs.get_attr_handle();
                        let idx = msg_payload.ifi_index as u32;
                        let name = handle
                            .get_attr_payload_as_with_len::<String>(Ifla::Ifname)
                            .unwrap_or_else(|_| "Unknown".to_string());

                        let state = handle
                            .get_attribute(Ifla::Linkinfo)
                            .and_then(|attr| InterfaceCanParams::try_from(attr).ok()?.state);

                        let event = NetlinkEvent {
                            interface_idx: idx,
                            interface_name: name,
                            state,
                        };

                        // Send the event to the main async loop
                        if tx.send(event).is_err() {
                            println!("Channel closed, stopping netlink monitoring");
                            break;
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
