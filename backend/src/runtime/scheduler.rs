use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{self, Interval};
use tokio::sync::mpsc;
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
    _shutdown_tx: mpsc::Sender<ShutdownMessage>,
}

enum ShutdownMessage {
    Stop(ControllerId),
    UpdateMode(ControllerId, ReconciliationMode),
}

impl ControllerScheduler {
    pub fn new() -> Self {
        let (shutdown_tx, _shutdown_rx) = mpsc::channel(32);
        let _ = _shutdown_rx;

        Self {
            modes: HashMap::new(),
            task_handles: HashMap::new(),
            _shutdown_tx: shutdown_tx,
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
        tokio::spawn(async move {
            let mut interval: Interval = time::interval(mode.interval());

            loop {
                interval.tick().await;
                reconcile_fn().await;
            }
        })
    }

    pub async fn update_mode(&self, id: &ControllerId, mode: ReconciliationMode) -> Result<(), String> {
        self._shutdown_tx.send(ShutdownMessage::UpdateMode(*id, mode)).await
            .map_err(|e| format!("Failed to update mode: {}", e))?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        for id in self.task_handles.keys() {
            let _ = self._shutdown_tx.send(ShutdownMessage::Stop(*id)).await;
        }
    }
}
