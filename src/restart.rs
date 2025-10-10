//! Restart management for CAN interfaces

use crate::interface::CanInterfaceInfo;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::RwLock, task::JoinHandle};

/// Manages pending restart tasks for CAN interfaces
#[derive(Debug)]
pub struct RestartManager {
    /// Map of interface index to pending restart task
    pending_tasks: Arc<RwLock<HashMap<u32, JoinHandle<()>>>>,
}

impl RestartManager {
    /// Create a new restart manager
    pub fn new() -> Self {
        Self {
            pending_tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Schedule a delayed restart for a bus-off interface
    pub async fn schedule_restart(&self, interface: CanInterfaceInfo, delay: Duration) {
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

        // Store the interface index before moving interface into the task
        let interface_idx = interface.idx;

        // Spawn a new delayed restart task
        let task = tokio::spawn(async move {
            tokio::time::sleep(delay).await;

            // Remove this task from pending tasks BEFORE executing restart
            // This prevents race condition with netlink events caused by the restart
            pending_tasks.write().await.remove(&interface.idx);

            do_restart(interface).await;
        });

        // Store the task handle
        self.pending_tasks.write().await.insert(interface_idx, task);
    }

    /// Cancel any pending restart for an interface
    pub async fn cancel_restart(&self, interface: &CanInterfaceInfo) {
        if let Some(task) = self.pending_tasks.write().await.remove(&interface.idx) {
            task.abort();
            println!("{}: cancelled pending restart", interface.name);
        }
    }

    /// Get the number of pending restart tasks
    pub async fn pending_count(&self) -> usize {
        self.pending_tasks.read().await.len()
    }
}

impl Default for RestartManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Performs the actual restart for a CAN interface
async fn do_restart(interface: CanInterfaceInfo) {
    use socketcan::CanInterface;

    println!("{}: restarting interface", interface.name);

    let iface = CanInterface::open_iface(interface.idx);
    match iface.restart() {
        Ok(_) => println!("{}: restart successful", interface.name),
        Err(e) => println!("{}: restart failed: {}", interface.name, e),
    }
}
