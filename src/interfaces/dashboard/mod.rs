use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

pub mod admin;
pub use admin::AdminGate;
pub mod assets;
pub mod event;
pub mod http;
mod lifecycle;
pub mod projection;
pub mod recorder;

pub use event::{EventQuery, OperationEvent, OperationKind, OperationStatus};
pub use http::{
    DashboardHandle, start_admin_dashboard_service, start_dashboard_service,
    start_dashboard_service_with_operation_log,
};
pub(crate) use lifecycle::{DashboardSessionHandle, start_dashboard_session};
pub use projection::{
    DashboardRuntimeInfo, DashboardSummary, OperationDetail, build_summary, project_event_detail,
    project_from_log_entry,
};
pub use recorder::OperationRecorder;

#[derive(Debug, Clone)]
pub enum DashboardObserver {
    Disabled,
    Enabled {
        recorder: OperationRecorder,
        sequence: Arc<AtomicU64>,
    },
}

impl DashboardObserver {
    pub fn disabled() -> Self {
        Self::Disabled
    }

    pub fn enabled(recorder: OperationRecorder) -> Self {
        Self::Enabled {
            recorder,
            sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn recorder(&self) -> Option<OperationRecorder> {
        match self {
            Self::Disabled => None,
            Self::Enabled { recorder, .. } => Some(recorder.clone()),
        }
    }

    pub fn next_sequence(&self) -> u64 {
        match self {
            Self::Disabled => 0,
            Self::Enabled { sequence, .. } => sequence.fetch_add(1, Ordering::Relaxed) + 1,
        }
    }

    pub fn record_event(&self, event: OperationEvent) {
        if let Self::Enabled { recorder, .. } = self {
            recorder.append(event);
        }
    }

    /// Core session callers leave sequence unset; assignment and publication
    /// happen atomically in the shared recorder.
    pub fn record_ordered_event(&self, event: OperationEvent) {
        if let Self::Enabled { recorder, sequence } = self {
            recorder.append_ordered(event, sequence);
        }
    }

    pub fn record_dashboard_started(&self, base_url: &str) {
        let sequence = self.next_sequence();
        if sequence == 0 {
            return;
        }
        self.record_event(event::dashboard_started(base_url, sequence));
    }

    pub fn record_tool_ok<T: Serialize>(
        &self,
        operation: &str,
        namespace: Option<String>,
        correlation_id: Option<String>,
        summary: String,
        payload: &T,
    ) {
        let sequence = self.next_sequence();
        if sequence == 0 {
            return;
        }
        self.record_event(event::tool_completed(
            Uuid::new_v4().to_string(),
            operation,
            namespace,
            correlation_id,
            sequence,
            summary,
            serde_json::to_value(payload).unwrap_or_else(|_| json!({ "serialization": "failed" })),
        ));
    }

    pub fn record_tool_failed(
        &self,
        operation: &str,
        namespace: Option<String>,
        correlation_id: Option<String>,
        summary: String,
        error: String,
    ) {
        let sequence = self.next_sequence();
        if sequence == 0 {
            return;
        }
        self.record_event(event::tool_failed(
            operation,
            namespace,
            correlation_id,
            sequence,
            summary,
            error,
        ));
    }

    pub fn record_auto_reflection<T: Serialize>(
        &self,
        operation: &str,
        namespace: Option<String>,
        correlation_id: Option<String>,
        status: OperationStatus,
        summary: String,
        payload: &T,
    ) {
        let sequence = self.next_sequence();
        if sequence == 0 {
            return;
        }
        self.record_event(event::auto_reflection_event(
            operation,
            namespace,
            correlation_id,
            sequence,
            status,
            summary,
            serde_json::to_value(payload).unwrap_or_else(|_| json!({ "serialization": "failed" })),
        ));
    }
}
