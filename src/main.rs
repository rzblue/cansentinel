//! cansentinel
//!
//! cansentinel monitors CAN interface state changes and automatically restarts interfaces that enter the bus-off state.

use cansentinel::{
    BusEvent, BusEventType, Config, RestartManager, find_matching_interfaces,
    monitoring::{monitor_interface_errors, monitor_netlink},
};
use clap::Parser;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "cansentinel")]
#[command(
    about = "cansentinel monitors CAN interface state changes and automatically restarts interfaces that enter the bus-off state"
)]
struct Args {
    /// CAN interface names to monitor (can be specified multiple times)
    #[arg(short = 'i', long = "interface", action = clap::ArgAction::Append, default_values = ["can*"])]
    interfaces: Vec<String>,

    /// Delay in milliseconds to wait before restarting interface
    #[arg(short = 'd', long = "delay", default_value = "1000")]
    delay_ms: u64,

    /// Enable more verbose output
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::SetTrue)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Check if user specified their own interfaces or we're using defaults
    let using_defaults = args.interfaces.len() == 1 && args.interfaces[0] == "can*";

    // Resolve interface patterns to actual interface info
    let interfaces = find_matching_interfaces(args.interfaces);

    if interfaces.is_empty() {
        if using_defaults {
            println!("Error: No matching interfaces found for default pattern 'can*'");
        } else {
            println!("Error: No matching interfaces found for the specified patterns.");
        }
        return;
    }

    // Configure interfaces to monitor
    let config = Config::new(Duration::from_millis(args.delay_ms), interfaces);
    let restart_manager = RestartManager::new();

    println!("Starting CAN interface monitor daemon");
    println!("Bus-off delay: {:?}", config.bus_off_delay);
    println!(
        "Monitoring interfaces: {:?}",
        config
            .interfaces
            .iter()
            .map(|i| &i.name)
            .collect::<Vec<_>>()
    );

    let interfaces = &config.interfaces;

    // Create a unified channel for bus-off detection from both sources
    let (tx, mut rx) = mpsc::unbounded_channel::<BusEvent>();

    // Start netlink monitoring
    let netlink_tx = tx.clone();
    let netlink_handle = tokio::task::spawn_blocking(move || {
        monitor_netlink(netlink_tx, args.verbose);
    });

    // Start CAN error frame monitoring for each interface
    let mut error_handles = Vec::new();
    for interface in interfaces {
        let interface = interface.clone();
        let error_tx = tx.clone();
        let handle = tokio::spawn(async move {
            monitor_interface_errors(error_tx, interface, args.verbose).await;
        });
        error_handles.push(handle);
    }

    // Main event loop - handle bus-off events from both sources
    while let Some(event) = rx.recv().await {
        match event.event_type {
            BusEventType::BusOff => {
                restart_manager
                    .schedule_restart(event.interface, config.bus_off_delay)
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
