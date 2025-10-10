//! CAN error frame monitoring

use crate::events::BusEventSource;
use crate::{events::BusEvent, interface::CanInterfaceInfo};
use socketcan::async_io::CanSocket;
use socketcan::{CanError, CanErrorFrame, SocketOptions};
use socketcan::{CanFrame, EmbeddedFrame, Frame};
use std::time::Duration;
use tokio::sync::mpsc;

/// Monitor error frames on a specific CAN interface
pub async fn monitor_interface_errors(
    tx: mpsc::UnboundedSender<BusEvent>,
    interface: CanInterfaceInfo,
    verbose: bool,
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

                loop {
                    match socket.read_frame().await {
                        Ok(CanFrame::Error(frame)) => {
                            if verbose {
                                log_can_error(&interface, &frame);
                            }

                            let event = match frame.into_error() {
                                CanError::BusOff => Some(BusEvent::bus_off(
                                    interface.clone(),
                                    BusEventSource::ErrorFrame(frame),
                                )),
                                CanError::Restarted => Some(BusEvent::restart(
                                    interface.clone(),
                                    BusEventSource::ErrorFrame(frame),
                                )),
                                _ => None,
                            };

                            if let Some(event) = event {
                                if tx.send(event).is_err() {
                                    println!("Channel closed, stopping monitoring");
                                    return;
                                }
                            }
                        }
                        Ok(_) => (), // Ignore non-error frames
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
            "{}: failed to open socket for monitoring. retrying in 5 seconds...",
            interface.name
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Log CAN error events with detailed analysis
fn log_can_error(interface: &CanInterfaceInfo, frame: &CanErrorFrame) {
    println!(
        "CAN ERROR on {}: ID=0x{:03X}, DLC={}, Data={:02X?}",
        interface.name,
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
