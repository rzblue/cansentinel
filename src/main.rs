//! cansentinel daemon
//!
//! cansentinel monitors CAN interface state changes and automatically restarts interfaces that enter the bus-off state.

use cansentinel::{
    BusEvent, BusEventType, CanInterfaceInfo, Config, RestartManager,
    monitoring::{monitor_interface_errors, monitor_netlink},
};
use clap::Parser;
use std::time::Duration;
use tokio::sync::mpsc;

/// CAN interface monitor daemon that automatically restarts bus-off interfaces
#[derive(Parser)]
#[command(name = "cansentinel")]
#[command(
    about = "A daemon that monitors CAN interface state changes and automatically restarts interfaces that enter the bus-off state"
)]
struct Args {
    /// CAN interface names to monitor (can be specified multiple times)
    #[arg(short = 'i', long = "interface", action = clap::ArgAction::Append)]
    interfaces: Vec<String>,

    /// Bus-off timeout in milliseconds before restarting interface
    #[arg(short = 't', long = "timeout", default_value = "1000")]
    timeout_ms: u64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Configure interfaces to monitor
    let config = Config::new(Duration::from_millis(args.timeout_ms), args.interfaces);
    let restart_manager = RestartManager::new();

    println!("Starting CAN interface monitor daemon");
    println!("Bus-off timeout: {:?}", config.bus_off_timeout);
    println!("Monitoring interfaces: {:?}", config.interface_names);

    // Look up interface indices early - only proceed with interfaces that exist
    let interfaces: Vec<CanInterfaceInfo> = config
        .interface_names
        .iter()
        .filter_map(|name| match CanInterfaceInfo::new(name) {
            Ok(interface) => Some(interface),
            Err(e) => {
                println!("Could not get index for interface '{}': {}", name, e);
                None
            }
        })
        .collect();

    // Create a unified channel for bus-off detection from both sources
    let (tx, mut rx) = mpsc::unbounded_channel::<BusEvent>();

    // Start netlink monitoring
    let netlink_tx = tx.clone();
    let netlink_handle = tokio::task::spawn_blocking(move || {
        monitor_netlink(netlink_tx);
    });

    // Start CAN error frame monitoring for each interface
    let mut error_handles = Vec::new();
    for interface in &interfaces {
        let interface = interface.clone();
        let error_tx = tx.clone();
        let handle = tokio::spawn(async move {
            monitor_interface_errors(error_tx, interface).await;
        });
        error_handles.push(handle);
    }

    // Main event loop - handle bus-off events from both sources
    while let Some(event) = rx.recv().await {
        match event.event_type {
            BusEventType::BusOff => {
                println!(
                    "{}: bus_off, scheduling restart in {:?}",
                    event.interface.name, config.bus_off_timeout
                );
                restart_manager
                    .schedule_restart(event.interface, config.bus_off_timeout)
                    .await;
            }
            BusEventType::Restart | BusEventType::Stopped => {
                // It may be better to just ignore these and let it fail.
                // Todo: Log these
                restart_manager.cancel_restart(&event.interface).await;
            }
        }
    }

    for handle in error_handles {
        handle.abort();
    }
    netlink_handle.abort();
}
