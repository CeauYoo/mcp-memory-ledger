//! Session-owned listener lifetime. Optional port contention never disables recording.
use std::{future::pending, io::ErrorKind, time::Duration};

use anyhow::Result;

use crate::{adapters::sqlite::SqliteStore, support::config::DashboardConfig};

use super::{
    DashboardHandle, DashboardObserver, DashboardRuntimeInfo, OperationRecorder,
    start_dashboard_service_with_operation_log,
};

const PORT_RETRY_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct DashboardSessionHandle {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for DashboardSessionHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn address_in_use(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == ErrorKind::AddrInUse)
}

fn record_started(observer: &DashboardObserver, handle: &DashboardHandle) {
    observer.record_dashboard_started(&handle.base_url());
    tracing::info!(dashboard_url = %handle.base_url(), "dashboard service started");
}

pub(crate) async fn start_dashboard_session(
    config: DashboardConfig,
    runtime: DashboardRuntimeInfo,
    operation_log: Option<SqliteStore>,
) -> Result<(DashboardObserver, Option<DashboardSessionHandle>)> {
    if !config.enabled {
        return Ok((DashboardObserver::disabled(), None));
    }
    let recorder = OperationRecorder::new(config.event_capacity);
    let observer = DashboardObserver::enabled(recorder.clone());
    let initial = match start_dashboard_service_with_operation_log(
        config.clone(),
        recorder.clone(),
        runtime.clone(),
        operation_log.clone(),
    )
    .await
    {
        Ok(handle) => Some(handle),
        Err(error) if config.required => return Err(error),
        Err(error) if address_in_use(&error) => {
            tracing::warn!("dashboard port is occupied; waiting while MCP remains connected");
            None
        }
        Err(error) => {
            tracing::warn!(error = %error, "dashboard service failed to start; continuing without dashboard");
            return Ok((DashboardObserver::disabled(), None));
        }
    };
    if let Some(handle) = &initial {
        record_started(&observer, handle);
    }
    let session_observer = observer.clone();
    let task = tokio::spawn(async move {
        // This task owns both the wait and the eventual listener. Aborting it on
        // MCP shutdown drops DashboardHandle, which aborts the HTTP server too.
        let _handle = match initial {
            Some(handle) => handle,
            None => loop {
                tokio::time::sleep(PORT_RETRY_INTERVAL).await;
                match start_dashboard_service_with_operation_log(
                    config.clone(),
                    recorder.clone(),
                    runtime.clone(),
                    operation_log.clone(),
                )
                .await
                {
                    Ok(handle) => {
                        record_started(&session_observer, &handle);
                        break handle;
                    }
                    Err(error) if address_in_use(&error) => {}
                    Err(error) => {
                        tracing::warn!(error = %error, "dashboard port wait stopped with error");
                        return;
                    }
                }
            },
        };
        pending::<()>().await;
    });
    Ok((observer, Some(DashboardSessionHandle { task })))
}
