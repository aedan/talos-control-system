use std::sync::Arc;
use tokio::sync::mpsc::{self, Sender, Receiver};
use dashmap::DashMap;
use tracing::debug;

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum EventType {
    ClusterChanged,
    ClusterRemoved,
    MachineChanged,
    MachineRemoved,
    MachineSetChanged,
    ConfigChanged,
    BrandingChanged,
    UserChanged,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventType::ClusterChanged => write!(f, "cluster.changed"),
            EventType::ClusterRemoved => write!(f, "cluster.removed"),
            EventType::MachineChanged => write!(f, "machine.changed"),
            EventType::MachineRemoved => write!(f, "machine.removed"),
            EventType::MachineSetChanged => write!(f, "machineset.changed"),
            EventType::ConfigChanged => write!(f, "config.changed"),
            EventType::BrandingChanged => write!(f, "branding.changed"),
            EventType::UserChanged => write!(f, "user.changed"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: EventType,
    pub resource_id: String,
    pub payload: serde_json::Value,
}

pub struct EventBus {
    channels: Arc<DashMap<EventType, Vec<Sender<Event>>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
        }
    }

    pub async fn subscribe(&self, event_type: EventType) -> Receiver<Event> {
        let (tx, rx) = mpsc::channel(100);
        self.channels.entry(event_type).or_default().push(tx);
        debug!(event_type = %event_type, "New subscriber registered");
        rx
    }

    pub async fn publish(&self, event: Event) {
        let et = event.event_type;
        debug!(event_type = %et, resource_id = %event.resource_id, "Event published");

        if let Some(entry) = self.channels.get(&et) {
            let senders = entry.value().clone();
            let mut broken_indices = Vec::new();
            for (i, tx) in senders.iter().enumerate() {
                if tx.send(event.clone()).await.is_err() {
                    broken_indices.push(i);
                }
            }
            if !broken_indices.is_empty() {
                drop(entry);
                let remove_entry = self.channels.remove(&et);
                if let Some((_, mut senders)) = remove_entry {
                    for &i in broken_indices.iter().rev() {
                        if i < senders.len() {
                            senders.remove(i);
                        }
                    }
                    self.channels.insert(et, senders);
                }
            }
        }
    }

    pub async fn broadcast(&self, event: Event) {
        let event_types = [
            EventType::ClusterChanged,
            EventType::ClusterRemoved,
            EventType::MachineChanged,
            EventType::MachineRemoved,
            EventType::MachineSetChanged,
            EventType::ConfigChanged,
            EventType::BrandingChanged,
            EventType::UserChanged,
        ];

        for et in event_types {
            if let Some(entry) = self.channels.get(&et) {
                for tx in entry.value().clone() {
                    let _ = tx.send(event.clone()).await;
                }
            }
        }
    }
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        Self {
            channels: Arc::clone(&self.channels),
        }
    }
}
