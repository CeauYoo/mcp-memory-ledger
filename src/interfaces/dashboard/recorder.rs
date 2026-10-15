use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use tokio::sync::broadcast;

use super::{EventQuery, OperationEvent};

#[derive(Debug, Clone)]
pub struct OperationRecorder {
    capacity: usize,
    events: Arc<Mutex<VecDeque<OperationEvent>>>,
    broadcaster: broadcast::Sender<OperationEvent>,
}

impl OperationRecorder {
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "operation recorder capacity must be greater than 0"
        );
        Self {
            capacity,
            events: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            broadcaster: broadcast::channel(capacity.max(16)).0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn append(&self, event: OperationEvent) {
        let mut events = self
            .events
            .lock()
            .expect("operation recorder lock poisoned");
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(event.clone());
        let _ = self.broadcaster.send(event);
    }

    /// Assign and publish while holding one lock so concurrent core sessions
    /// cannot publish a higher sequence before a lower one.
    pub(super) fn append_ordered(
        &self,
        mut event: OperationEvent,
        sequence: &std::sync::atomic::AtomicU64,
    ) {
        let mut events = self
            .events
            .lock()
            .expect("operation recorder lock poisoned");
        event.sequence = sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(event.clone());
        let _ = self.broadcaster.send(event);
    }

    pub fn recent(&self, query: EventQuery) -> Vec<OperationEvent> {
        let limit = query.limit.unwrap_or(self.capacity);
        let events = self
            .events
            .lock()
            .expect("operation recorder lock poisoned");
        events
            .iter()
            .filter(|event| query.kind.is_none_or(|kind| event.kind == kind))
            .filter(|event| query.status.is_none_or(|status| event.status == status))
            .filter(|event| {
                query
                    .namespace
                    .as_ref()
                    .is_none_or(|namespace| event.namespace.as_ref() == Some(namespace))
            })
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OperationEvent> {
        self.broadcaster.subscribe()
    }
}
