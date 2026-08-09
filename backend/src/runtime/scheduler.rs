use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{self, Interval};
use tokio::sync::{mpsc, broadcast};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::runtime::dag::ControllerId;
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReconciliationMode {
    Critical,
    Active,
    Idle,
}

impl ReconciliationMode {
    pub fn interval(&self) -> Duration {
        match self {
            ReconciliationMode::Critical => Duration::from_secs(1),
            ReconciliationMode::Active => Duration::from_secs(5),
            ReconciliationMode::Idle => Duration::from_secs(60),
        }
    }
}

pub struct ControllerScheduler {
    modes: HashMap<ControllerId, ReconciliationMode>,
    task_handles: HashMap<ControllerId, JoinHandle<()>>,
    shutdown_tx: broadcast::Sender<ShutdownMessage>,
}

#[derive(Clone)]
enum ShutdownMessage {
    Stop(ControllerId),
    UpdateMode(ControllerId, ReconciliationMode),
}

impl ControllerScheduler {
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(32);

        Self {
            modes: HashMap::new(),
            task_handles: HashMap::new(),
            shutdown_tx,
        }
    }

    pub async fn register<F, Fut>(&mut self, id: ControllerId, mode: ReconciliationMode, reconcile_fn: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.modes.insert(id, mode);

        let handle = self.start_reconciliation_loop(id, mode, reconcile_fn);
        self.task_handles.insert(id, handle);

        info!(controller = %id, ?mode, "Controller registered with scheduler");
    }

    fn start_reconciliation_loop<F, Fut>(&self, id: ControllerId, mode: ReconciliationMode, reconcile_fn: F) -> JoinHandle<()>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut rx = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval: Interval = time::interval(mode.interval());
            // Skip initial immediate tick
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        reconcile_fn().await;
                    }
                    result = rx.recv() => {
                        match result {
                            Ok(msg) => {
                                match msg {
                                    ShutdownMessage::Stop(ctrl_id) if ctrl_id == id => {
                                        info!(controller = %id, "Controller stopped via shutdown signal");
                                        break;
                                    }
                                    ShutdownMessage::UpdateMode(ctrl_id, new_mode) if ctrl_id == id => {
                                        info!(controller = %id, ?new_mode, "Controller mode updated");
                                        interval = time::interval(new_mode.interval());
                                        interval.tick().await;
                                    }
                                    _ => {}
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(count = n, "Shutdown channel lagged for controller {}", id);
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                info!(controller = %id, "Shutdown channel closed, stopping");
                                break;
                            }
                        }
                    }
                }
            }
        })
    }

    pub async fn update_mode(&self, id: &ControllerId, mode: ReconciliationMode) -> Result<(), String> {
        self.shutdown_tx.send(ShutdownMessage::UpdateMode(*id, mode))
            .map_err(|e| format!("Failed to update mode: {}", e))?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        for id in self.task_handles.keys() {
            let _ = self.shutdown_tx.send(ShutdownMessage::Stop(*id));
        }
    }
}
