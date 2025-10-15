//! cansentinel
//!
//! cansentinel monitors CAN interface state changes and automatically restarts interfaces that enter the bus-off state.

use cansentinel::{
    BusEvent, BusEventType, CanInterfaceInfo, Config, RestartManager,
    monitoring::{monitor_interface_errors, monitor_netlink},
};
use clap::Parser;
use git_version::git_version;
use socketcan::{CanInterface, nl::CanState};
use std::time::Duration;
use tokio::sync::mpsc;

const VERSION: &str = git_version!(prefix = concat!(env!("CARGO_PKG_VERSION"), "-"), fallback = "unknown");

#[derive(Parser)]
#[command(name = "cansentinel")]
#[command(version = VERSION)]
#[command(
    about = "cansentinel monitors CAN interface state changes and automatically restarts interfaces that enter the bus-off state"
)]
struct Args {
    /// CAN interface names to monitor (can be specified multiple times)
    #[arg(short = 'i', long = "interface", action = clap::ArgAction::Append)]
    interfaces: Vec<String>,

    /// Ignore invalid interface names instead of failing
    #[arg(long = "ignore-invalid")]
    ignore_invalid: bool,

    /// Delay in milliseconds to wait before restarting interface
    #[arg(short = 'd', long = "delay-ms", default_value = "1000")]
    delay_ms: u64,

    /// Enable more verbose output
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::SetTrue)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Configure interfaces to monitor
    let config = Config::new(Duration::from_millis(args.delay_ms), args.interfaces);

    if config.interface_names.is_empty() {
        println!("No interfaces specified. Use -i/--interface to specify interfaces to monitor.");
        std::process::exit(1);
    }

    let mut interfaces: Vec<CanInterfaceInfo> = Vec::with_capacity(config.interface_names.len());
    let mut got_error = false;
    for name in &config.interface_names {
        match CanInterfaceInfo::new(name) {
            Ok(interface) => interfaces.push(interface),
            Err(e) => {
                if args.ignore_invalid {
                    println!("Could not find interface '{}': {}. Ignoring.", name, e);
                } else {
                    println!("Could not find interface '{}': {}", name, e);
                    got_error = true;
                }
            }
        }
    }

    if got_error {
        std::process::exit(1);
    }

    if interfaces.is_empty() {
        println!("No valid interfaces found to monitor.");
        std::process::exit(1);
    }

    println!("Starting cansentinel {VERSION}");
    println!("Restart delay: {:?}", config.restart_delay);
    println!("Monitoring interfaces: {:?}", config.interface_names);

    let restart_manager = RestartManager::new();

    for interface in &interfaces {
        // Check initial interface status and restart if already in bus-off state
        if let Ok(Some(CanState::BusOff)) = CanInterface::open_iface(interface.idx).state() {
            println!(
                "{}: already in bus-off state, restarting immediately",
                interface.name
            );
            restart_manager
                .schedule_restart(interface.clone(), Duration::from_millis(0))
                .await;
        }
    }

    #[cfg(feature = "systemd")]
    {
        // This is where we are considered ready, for the purposes of service ordering
        use libsystemd::daemon::{NotifyState, notify};
        match notify(
            false,
            &[
                NotifyState::Ready,
                NotifyState::Status(format!("Monitoring {} CAN interfaces...", interfaces.len(),)),
            ],
        ) {
            Ok(_) => (), // If this returns false, systemd isn't available and we don't care.
            Err(e) => println!("Failed to notify systemd: {}", e),
        }
    }

    // Create a unified channel for bus-off detection from both sources
    let (tx, mut rx) = mpsc::unbounded_channel::<BusEvent>();

    let netlink_handle = {
        // Start netlink monitoring
        let netlink_tx = tx.clone();
        let netlink_interfaces = interfaces.clone();
        tokio::task::spawn_blocking(move || {
            monitor_netlink(netlink_tx, netlink_interfaces, args.verbose);
        })
    };

    // Start CAN error frame monitoring for each interface
    let error_handles = {
        let mut handles = Vec::with_capacity(interfaces.capacity());
        for interface in &interfaces {
            let interface = interface.clone();
            let error_tx = tx.clone();
            let handle = tokio::spawn(async move {
                monitor_interface_errors(error_tx, interface, args.verbose).await;
            });
            handles.push(handle);
        }
        handles
    };

    // Main event loop - handle bus-off events from both sources
    while let Some(event) = rx.recv().await {
        match event.event_type {
            BusEventType::BusOff => {
                restart_manager
                    .schedule_restart(event.interface, config.restart_delay)
                    .await;
            }
            BusEventType::Restart | BusEventType::Stopped => {
                // Just let pending restarts ride out.
                // These can arrive in a weird order during a continuous bus short condition causing this to race
            }
        }
    }

    for handle in error_handles {
        handle.abort();
    }
    netlink_handle.abort();
}
