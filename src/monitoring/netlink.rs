//! Netlink-based CAN interface monitoring

use crate::{
    events::{BusEvent, BusEventSource},
    interface::CanInterfaceInfo,
};
use nix::libc::{ARPHRD_CAN, RTNLGRP_LINK};
use socketcan::{InterfaceCanParams, nl::CanState};
use tokio::sync::mpsc;

/// Runs the blocking netlink monitoring loop
pub fn monitor_netlink(
    tx: mpsc::UnboundedSender<BusEvent>,
    interfaces: Vec<CanInterfaceInfo>,
    verbose: bool,
) {
    use neli::{
        consts::{
            rtnl::{Ifla, Rtm},
            socket::NlFamily,
        },
        rtnl::Ifinfomsg,
        socket,
    };

    let interfaces = {
        let mut interfaces: Vec<u32> = interfaces.into_iter().map(|i| i.idx).collect();
        interfaces.sort();
        interfaces
    };

    let mut s = match socket::NlSocketHandle::connect(NlFamily::Route, Some(0), &[RTNLGRP_LINK]) {
        Ok(socket) => socket,
        Err(e) => {
            println!("Failed to create netlink socket: {:?}", e);
            return;
        }
    };

    println!("Started netlink monitoring for CAN interfaces");

    for next in s.iter::<Rtm, Ifinfomsg>(true) {
        match next {
            Ok(msg) => {
                if let Ok(msg_payload) = msg.get_payload() {
                    // Only process CAN interfaces
                    if u16::from(msg_payload.ifi_type) == ARPHRD_CAN
                        && interfaces
                            .binary_search(&(msg_payload.ifi_index as u32))
                            .is_ok()
                    {
                        let handle = msg_payload.rtattrs.get_attr_handle();
                        let idx = msg_payload.ifi_index as u32;
                        let name = handle
                            .get_attr_payload_as_with_len::<String>(Ifla::Ifname)
                            .unwrap_or_else(|_| "Unknown".to_string());

                        let state = handle
                            .get_attribute(Ifla::Linkinfo)
                            .and_then(|attr| InterfaceCanParams::try_from(attr).ok()?.state);

                        let interface = CanInterfaceInfo { idx, name };

                        if verbose {
                            println!(
                                "Netlink: Interface {} (idx={}) state: {:?}",
                                interface.name, interface.idx, state
                            );
                        }

                        let event = match state {
                            Some(CanState::BusOff) => Some(BusEvent::bus_off(
                                interface,
                                BusEventSource::StateUpdate(CanState::BusOff),
                            )),
                            Some(CanState::Stopped) => Some(BusEvent::stopped(
                                interface,
                                BusEventSource::StateUpdate(CanState::Stopped),
                            )),
                            // We don't trust netlink to deliver restarted messages correctly
                            _ => None,
                        };

                        if let Some(event) = event {
                            if tx.send(event).is_err() {
                                println!("Channel closed, stopping netlink monitoring");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                println!("Netlink error: {:?}", e);
                break;
            }
        }
    }
    println!("Netlink monitoring thread finished");
}
