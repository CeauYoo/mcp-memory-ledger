use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::{CallToolResult, JsonObject, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use tracing::{info, warn};

use crate::{
    adapters::sqlite::{SqliteStore, open_current_database},
    application::{
        auto_reflect_if_needed::{self, AutoReflectInput, RecursionGuard},
        build_self_snapshot,
        daemon::DaemonHandle,
        decide_with_snapshot, get_evidence_relation, get_memory, get_reflection_history,
        get_self_model_history, get_self_model_versions, ingest_interaction,
        ingest_interaction::IngestInput,
        run_reflection,
        run_reflection::ReflectionInput,
        search_memory, supersede_memory,
    },
    domain::self_revision::SELF_REVISION_DURABLE_WRITE_PATH,
    error::AppError,
    interfaces::dashboard::{
        DashboardHandle, DashboardObserver, DashboardRuntimeInfo, OperationRecorder,
        start_dashboard_service_with_operation_log,
    },
    support::config::{AppConfig, ModelProviderKind, TransportKind},
};

use super::dto::{
    BuildSelfSnapshotParams, DecideWithSnapshotParams, GetEvidenceRelationParams, GetMemoryParams,
    GetReflectionHistoryParams, GetSelfModelHistoryParams, GetSelfModelVersionsParams,
    IngestInteractionParams, RunReflectionParams, SearchMemoryParams, SupersedeMemoryParams,
};

pub const AUTO_REFLECTION_RUNTIME_HOOKS: [&str; 4] = [
    "ingest_interaction:failure",
    "ingest_interaction:conflict",
    "decide_with_snapshot:conflict",
    "build_self_snapshot:periodic",
];
pub const SELF_REVISION_WRITE_PATH: &str = SELF_REVISION_DURABLE_WRITE_PATH;

mod caller_budget;
mod diagnostics;
mod runtime;
mod transport;
use runtime::Runtime;

use diagnostics::{
    ToolOperationRecord, log_auto_reflection_success, map_tool_error, runtime_hook_for,
};

use transport::{decode_tool_params, generated_mcp_correlation_id, structured};

pub async fn run_stdio_server() -> Result<()> {
    let config = AppConfig::load().map_err(anyhow::Error::msg)?;
    run_stdio_server_with_config(config).await
}

pub async fn run_stdio_server_with_config(config: AppConfig) -> Result<()> {
    config.validate().map_err(anyhow::Error::msg)?;
    let store = open_current_database(&config.database_url).await?;
    let (dashboard_observer, _dashboard_handle) =
        start_configured_dashboard(&config, Some(store.clone())).await?;
    let daemon_handle = start_configured_daemon(&config);
    let server = Server::from_parts(config, store, dashboard_observer).await?;
    let service = server.serve(stdio()).await?;
    let wait_result = service.waiting().await;
    if let Some(handle) = daemon_handle {
        handle.stop().await;
    }
    wait_result?;
    Ok(())
}

fn start_configured_daemon(config: &AppConfig) -> Option<DaemonHandle> {
    if !config.daemon.enabled {
        return None;
    }

    let handle = DaemonHandle::start(config.daemon.clone());
    info!(
        mode = handle.mode(),
        poll_interval_ms = handle.poll_interval_ms(),
        writes_allowed = handle.writes_allowed(),
        remote_listener_enabled = handle.remote_listener_enabled(),
        "observe-only daemon lifecycle started"
    );
    Some(handle)
}

pub async fn validate_stdio_runtime(config: &AppConfig) -> Result<SqliteStore, AppError> {
    config.validate().map_err(AppError::Message)?;
    open_current_database(&config.database_url).await
}

async fn start_configured_dashboard(
    config: &AppConfig,
    operation_log: Option<SqliteStore>,
) -> Result<(DashboardObserver, Option<DashboardHandle>)> {
    if !config.dashboard.enabled {
        return Ok((DashboardObserver::disabled(), None));
    }

    let recorder = OperationRecorder::new(config.dashboard.event_capacity);
    let runtime = DashboardRuntimeInfo {
        service_name: "agent-llm-mm".to_string(),
        transport: transport_label(config.transport).to_string(),
        provider: provider_label(config.model_provider).to_string(),
        dashboard_enabled: true,
        read_only: true,
    };

    match start_dashboard_service_with_operation_log(
        config.dashboard.clone(),
        recorder.clone(),
        runtime,
        operation_log,
    )
    .await
    {
        Ok(handle) => {
            let observer = DashboardObserver::enabled(recorder);
            observer.record_dashboard_started(&handle.base_url());
            info!(dashboard_url = %handle.base_url(), "dashboard service started");
            Ok((observer, Some(handle)))
        }
        Err(error) if config.dashboard.required => Err(error),
        Err(error) => {
            warn!(error = %error, "dashboard service failed to start; continuing without dashboard");
            Ok((DashboardObserver::disabled(), None))
        }
    }
}

fn transport_label(transport: TransportKind) -> &'static str {
    match transport {
        TransportKind::Stdio => "stdio",
    }
}

fn provider_label(provider: ModelProviderKind) -> &'static str {
    match provider {
        ModelProviderKind::Mock => "mock",
        ModelProviderKind::OpenAiCompatible => "openai-compatible",
        ModelProviderKind::OpenRouter => "openrouter",
        ModelProviderKind::OpenAiResponses => "openai-responses",
        ModelProviderKind::Anthropic => "anthropic",
    }
}

#[derive(Clone)]
pub struct Server {
    runtime: Runtime,
    tool_router: ToolRouter<Self>,
}

impl Server {
    async fn from_parts(
        config: AppConfig,
        store: SqliteStore,
        dashboard: DashboardObserver,
    ) -> Result<Self, AppError> {
        let runtime = Runtime::from_store(&config, store, dashboard).await?;
        Ok(Self {
            runtime,
            tool_router: Self::tool_router(),
        })
    }
}

#[tool_router]
impl Server {
    #[tool(
        description = "Persist an interaction event and any derived claims.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<IngestInteractionParams>>()
    )]
    async fn ingest_interaction(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "ingest_interaction",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<IngestInteractionParams>(raw_params),
        )
        .await?;
        let auto_reflect_input = map_tool_error(
            &self.runtime,
            "ingest_interaction",
            None,
            Some(correlation_id.clone()),
            AutoReflectInput::from_ingest(&params),
        )
        .await?;
        let dashboard_namespace = Some(auto_reflect_input.namespace.as_str().to_string());
        let runtime_hook = runtime_hook_for("ingest_interaction", auto_reflect_input.trigger_type);
        let auto_reflect_trigger_type = auto_reflect_input.trigger_type;
        let auto_reflect_trigger_key = auto_reflect_input.trigger_key();
        let input = map_tool_error(
            &self.runtime,
            "ingest_interaction",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            IngestInput::try_from(params).map_err(AppError::from),
        )
        .await?;
        let result = map_tool_error(
            &self.runtime,
            "ingest_interaction",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            ingest_interaction::execute(&self.runtime, input).await,
        )
        .await?;
        if !result.replayed {
            match auto_reflect_if_needed::execute(
                &self.runtime,
                auto_reflect_input.with_recursion_guard(RecursionGuard::Allow),
            )
            .await
            {
                Ok(diagnostics) => {
                    log_auto_reflection_success(
                        runtime_hook,
                        &diagnostics,
                        Some(result.event_id.as_str()),
                        &self.runtime.dashboard,
                        dashboard_namespace.clone(),
                        Some(correlation_id.clone()),
                    );
                    self.runtime
                        .record_auto_reflection_operation(
                            "ingest_interaction",
                            &diagnostics,
                            dashboard_namespace.clone(),
                            Some(correlation_id.clone()),
                        )
                        .await;
                }
                Err(error) => {
                    warn!(
                        runtime_hook,
                        event_id = %result.event_id,
                        trigger_type = ?auto_reflect_trigger_type,
                        trigger_key = %auto_reflect_trigger_key,
                        error = %error,
                        "best-effort auto-reflection failed after successful ingest"
                    );
                    self.runtime
                        .record_auto_reflection_failure_operation(
                            "ingest_interaction",
                            dashboard_namespace.clone(),
                            Some(correlation_id.clone()),
                            auto_reflect_trigger_type,
                            &auto_reflect_trigger_key,
                            &error,
                        )
                        .await;
                }
            }
        }
        self.runtime.dashboard.record_tool_ok(
            "ingest_interaction",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            format!("ingest stored event {}", result.event_id),
            &result,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok(
                    "ingest_interaction",
                    dashboard_namespace,
                    Some(correlation_id),
                )
                .with_response_summary(serde_json::json!({ "event_id": result.event_id })),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Recall active claims and event summaries by literal text in one explicit namespace, entirely offline. ASCII case-insensitive; CJK matches exact substrings including two-character terms. Up to 8 whitespace-separated terms and 512 query bytes, any-term matches. Bounded balanced Claim/Event allocation and interleaving; within each type rank by matched-term count, known recording time, then stable ID. Unknown recording time does not imply recency. Full provenance included. Existing search_memory remains history browsing. No semantic/vector search or model call. Optional caller_budget cooperatively limits caller-reported attempts and returns a receipt; omitted preserves legacy behavior.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<super::dto::RecallMemoryParams>>()
    )]
    async fn recall_memory(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let decision = caller_budget::admit_raw(
            &raw_params,
            crate::domain::caller_budget::CallerOperation::Retrieval,
        )?;
        let params =
            decode_tool_params::<super::dto::RecallMemoryParams>(raw_params).map_err(|error| {
                caller_budget::attach_error(
                    McpError::invalid_params(error.to_string(), None),
                    &decision,
                )
            })?;
        let input = crate::application::recall_memory::RecallMemoryInput::try_from(params)
            .map_err(|error| {
                caller_budget::attach_error(
                    McpError::invalid_params(error.to_string(), None),
                    &decision,
                )
            })?;
        let result = map_tool_error(
            &self.runtime,
            "recall_memory",
            Some(input.namespace.as_str().to_string()),
            None,
            crate::application::recall_memory::execute(&self.runtime.store, input).await,
        )
        .await
        .map_err(|error| caller_budget::attach_error(error, &decision))?;
        caller_budget::structured(result, decision)
    }

    #[tool(
        description = "Build offline task context from active claims and event observations in an explicit namespace. Returns whole records with provenance, bounded source-linked rich Episode snapshots (legacy Episode references remain in primary provenance), status and missing-evidence diagnostics when they fit, and explicit omission counts. max_bytes is a hard UTF-8 byte cap on compact serialized result JSON including all metadata, excluding the MCP/JSON-RPC transport envelope. Too-small budgets are rejected; no token-count guarantee. Fetch omitted records using scoped get_memory or get_episode_detail. Optional caller_budget receipt is included inside the result byte cap; counts are caller-owned, not durable quotas.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<super::dto::BuildTaskContextParams>>()
    )]
    async fn build_task_context(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let decision = caller_budget::admit_raw(
            &raw_params,
            crate::domain::caller_budget::CallerOperation::Retrieval,
        )?;
        let params = decode_tool_params::<super::dto::BuildTaskContextParams>(raw_params).map_err(
            |error| {
                caller_budget::attach_error(
                    McpError::invalid_params(error.to_string(), None),
                    &decision,
                )
            },
        )?;
        let input = crate::application::build_task_context::BuildTaskContextInput::try_from(params)
            .map_err(|error| {
                caller_budget::attach_error(
                    McpError::invalid_params(error.to_string(), None),
                    &decision,
                )
            })?;
        let result = map_tool_error(
            &self.runtime,
            "build_task_context",
            Some(input.namespace.as_str().to_string()),
            None,
            crate::application::build_task_context::execute_with_caller_budget(
                &self.runtime.store,
                input,
                decision.clone(),
            )
            .await,
        )
        .await
        .map_err(|error| caller_budget::attach_error(error, &decision))?;
        // Structured-only, matching the other tools: no duplicate text payload.
        structured(result)
    }

    #[tool(
        description = "Search complete event, claim, scoped Episode, or scoped Reflection provenance records in one explicit local memory namespace. Omitted record_type preserves Event behavior. Additive record_types runs a scoped union of the requested types with a stable recorded_at / type / id order. Event queries support exact reference, kind, inclusive time range, and bounded recent-first results. Claim queries support exact reference, status and mode, with nullable recording/observation metadata; explicit time-window filters remain Event-only. Episode queries support an exact persisted episode_reference. Reflection queries preserve same-scope Claim endpoint rules and additionally admit safely attributed targetless records; unknown or incompatible origin/effect/evidence scope stays hidden. Union queries reject type-specific filters.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<SearchMemoryParams>>()
    )]
    async fn search_memory(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "search_memory",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<SearchMemoryParams>(raw_params),
        )
        .await?;
        let dashboard_namespace = Some(params.namespace.clone());
        let input = map_tool_error(
            &self.runtime,
            "search_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            search_memory::SearchMemoryInput::try_from(params),
        )
        .await?;
        let log_record_type = if input.is_union() {
            "union".to_string()
        } else {
            input.record_types[0].as_str().to_string()
        };
        let result = map_tool_error(
            &self.runtime,
            "search_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            search_memory::execute(&self.runtime, input).await,
        )
        .await?;
        self.runtime.dashboard.record_tool_ok(
            "search_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            format!(
                "memory search returned {} {} records",
                result.records.len(),
                log_record_type
            ),
            &serde_json::json!({
                "record_type": log_record_type,
                "result_count": result.records.len(),
            }),
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok("search_memory", dashboard_namespace, Some(correlation_id))
                    .with_response_summary(serde_json::json!({
                        "record_type": log_record_type,
                        "result_count": result.records.len(),
                    })),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Get one complete event, claim, scoped Episode, or scoped Reflection record by stable ID inside one explicit local memory namespace. Omitted record_type preserves Event behavior. Claim, Episode, and Reflection lookup require their explicit record_type. Episode and Reflection ids are opaque exact persisted references. Safely attributed record-only reflections are readable; unknown or incompatible origin/effect/evidence scope stays hidden. Canonical and raw Event/Claim IDs are supported. A missing or cross-scope record returns null without widening the query.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<GetMemoryParams>>()
    )]
    async fn get_memory(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "get_memory",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<GetMemoryParams>(raw_params),
        )
        .await?;
        let dashboard_namespace = Some(params.namespace.clone());
        let input = map_tool_error(
            &self.runtime,
            "get_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_memory::GetMemoryInput::try_from(params),
        )
        .await?;
        let record_type = input.id.record_type();
        let result = map_tool_error(
            &self.runtime,
            "get_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_memory::execute(&self.runtime, input).await,
        )
        .await?;
        let found = result.record.is_some();
        self.runtime.dashboard.record_tool_ok(
            "get_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            if found {
                format!("memory lookup found one {} record", record_type.as_str())
            } else {
                format!("memory lookup found no {} record", record_type.as_str())
            },
            &serde_json::json!({"record_type": record_type.as_str(), "found": found}),
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok("get_memory", dashboard_namespace, Some(correlation_id))
                    .with_response_summary(
                        serde_json::json!({"record_type": record_type.as_str(), "found": found}),
                    ),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Read the newest claim-linked reflection records reachable from one exact claim inside an explicit local memory namespace. Missing and cross-scope claims return an empty history; mixed-scope revision edges are excluded. Identity and commitment revision audits are read through get_self_model_history.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<GetReflectionHistoryParams>>()
    )]
    async fn get_reflection_history(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "get_reflection_history",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<GetReflectionHistoryParams>(raw_params),
        )
        .await?;
        let dashboard_namespace = Some(params.namespace.clone());
        let input = map_tool_error(
            &self.runtime,
            "get_reflection_history",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_reflection_history::GetReflectionHistoryInput::try_from(params),
        )
        .await?;
        let result = map_tool_error(
            &self.runtime,
            "get_reflection_history",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_reflection_history::execute(&self.runtime, input).await,
        )
        .await?;
        let response_summary = serde_json::json!({
            "history_type": "claim",
            "result_count": result.reflections.len(),
            "has_more": result.has_more,
        });
        self.runtime.dashboard.record_tool_ok(
            "get_reflection_history",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            format!(
                "claim reflection history returned {} record(s)",
                result.reflections.len()
            ),
            &response_summary,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok(
                    "get_reflection_history",
                    dashboard_namespace,
                    Some(correlation_id),
                )
                .with_response_summary(response_summary),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Read scoped identity or commitment revision audits persisted on reflections inside one explicit local memory namespace. Claim-attributed and safely source/effect-attributed targetless audits are visible; unknown or incompatible record-only scope stays hidden. This compatibility history does not expose aggregate snapshots; get_self_model_versions is the separate opt-in version reader.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<GetSelfModelHistoryParams>>()
    )]
    async fn get_self_model_history(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "get_self_model_history",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<GetSelfModelHistoryParams>(raw_params),
        )
        .await?;
        let dashboard_namespace = Some(params.namespace.clone());
        let input = map_tool_error(
            &self.runtime,
            "get_self_model_history",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_self_model_history::GetSelfModelHistoryInput::try_from(params),
        )
        .await?;
        let history_type = input.history_kind.as_str().to_string();
        let result = map_tool_error(
            &self.runtime,
            "get_self_model_history",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_self_model_history::execute(&self.runtime, input).await,
        )
        .await?;
        let response_summary = serde_json::json!({
            "history_type": history_type,
            "result_count": result.records.len(),
            "has_more": result.has_more,
        });
        self.runtime.dashboard.record_tool_ok(
            "get_self_model_history",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            format!(
                "{history_type} history returned {} record(s)",
                result.records.len()
            ),
            &response_summary,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok(
                    "get_self_model_history",
                    dashboard_namespace,
                    Some(correlation_id),
                )
                .with_response_summary(response_summary),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Read bounded experimental self-model versions, newest first, within one explicit namespace. Requires allow_global_version_metadata:true because global counters reveal cross-namespace activity and do not provide authorization. Returns only written patches whose source reflection has verified single-scope durable evidence. Inherited aggregate state is omitted; previous values are redacted unless their own source has the same verified scope. Baselines and unknown/mixed provenance are hidden. Uses one consistent read transaction and fails closed on projection drift.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<GetSelfModelVersionsParams>>()
    )]
    async fn get_self_model_versions(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "get_self_model_versions",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<GetSelfModelVersionsParams>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let input = map_tool_error(
            &self.runtime,
            "get_self_model_versions",
            namespace.clone(),
            Some(correlation_id.clone()),
            get_self_model_versions::GetSelfModelVersionsInput::try_from(params),
        )
        .await?;
        let result = map_tool_error(
            &self.runtime,
            "get_self_model_versions",
            namespace.clone(),
            Some(correlation_id.clone()),
            get_self_model_versions::execute(&self.runtime, input).await,
        )
        .await?;
        let summary = serde_json::json!({
            "result_count": result.records.len(),
            "has_more": result.has_more,
        });
        self.runtime.dashboard.record_tool_ok(
            "get_self_model_versions",
            namespace.clone(),
            Some(correlation_id.clone()),
            format!(
                "self-model versions returned {} record(s)",
                result.records.len()
            ),
            &summary,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok("get_self_model_versions", namespace, Some(correlation_id))
                    .with_response_summary(summary),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Read a scoped evidence-relation report for one explicit local memory namespace. The trigger window is the caller-provided event ID list intersected with events that exist in that owner+namespace. Selected IDs must stay inside the scoped window. Missing and cross-scope trigger IDs are omitted without widening; this first slice does not rank or score evidence.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<GetEvidenceRelationParams>>()
    )]
    async fn get_evidence_relation(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "get_evidence_relation",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<GetEvidenceRelationParams>(raw_params),
        )
        .await?;
        let dashboard_namespace = Some(params.namespace.clone());
        let input = map_tool_error(
            &self.runtime,
            "get_evidence_relation",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_evidence_relation::GetEvidenceRelationInput::try_from(params),
        )
        .await?;
        let result = map_tool_error(
            &self.runtime,
            "get_evidence_relation",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            get_evidence_relation::execute(&self.runtime, input).await,
        )
        .await?;
        let response_summary = serde_json::json!({
            "report_type": "evidence_relation",
            "trigger_window_size": result.trigger_window_size,
            "selected_count": result.selected_count,
            "result_count": result.relations.len(),
        });
        self.runtime.dashboard.record_tool_ok(
            "get_evidence_relation",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            format!(
                "evidence relation returned {} row(s)",
                result.relations.len()
            ),
            &response_summary,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok(
                    "get_evidence_relation",
                    dashboard_namespace,
                    Some(correlation_id),
                )
                .with_response_summary(response_summary),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Build a self snapshot from the persisted memory store.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<BuildSelfSnapshotParams>>()
    )]
    async fn build_self_snapshot(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "build_self_snapshot",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<BuildSelfSnapshotParams>(raw_params),
        )
        .await?;
        let dashboard_namespace = params.namespace.clone();
        let snapshot_input = map_tool_error(
            &self.runtime,
            "build_self_snapshot",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            build_self_snapshot::BuildSelfSnapshotInput::try_from(params.clone()),
        )
        .await?;
        let auto_reflect_input = map_tool_error(
            &self.runtime,
            "build_self_snapshot",
            params.auto_reflect_namespace.clone(),
            Some(correlation_id.clone()),
            AutoReflectInput::from_build_snapshot(&params),
        )
        .await?;
        if let Some(auto_reflect_input) = auto_reflect_input {
            let auto_reflect_namespace = Some(auto_reflect_input.namespace.as_str().to_string());
            let auto_reflect_trigger_type = auto_reflect_input.trigger_type;
            let auto_reflect_trigger_key = auto_reflect_input.trigger_key();
            match auto_reflect_if_needed::execute(
                &self.runtime,
                auto_reflect_input.with_recursion_guard(RecursionGuard::Allow),
            )
            .await
            {
                Ok(diagnostics) => {
                    log_auto_reflection_success(
                        runtime_hook_for("build_self_snapshot", auto_reflect_trigger_type),
                        &diagnostics,
                        None,
                        &self.runtime.dashboard,
                        auto_reflect_namespace.clone(),
                        Some(correlation_id.clone()),
                    );
                    self.runtime
                        .record_auto_reflection_operation(
                            "build_self_snapshot",
                            &diagnostics,
                            auto_reflect_namespace,
                            Some(correlation_id.clone()),
                        )
                        .await;
                }
                Err(error) => {
                    warn!(
                        runtime_hook =
                            runtime_hook_for("build_self_snapshot", auto_reflect_trigger_type),
                        trigger_type = ?auto_reflect_trigger_type,
                        trigger_key = %auto_reflect_trigger_key,
                        error = %error,
                        "best-effort periodic auto-reflection failed"
                    );
                    self.runtime
                        .record_auto_reflection_failure_operation(
                            "build_self_snapshot",
                            auto_reflect_namespace,
                            Some(correlation_id.clone()),
                            auto_reflect_trigger_type,
                            &auto_reflect_trigger_key,
                            &error,
                        )
                        .await;
                }
            }
        }
        let result = map_tool_error(
            &self.runtime,
            "build_self_snapshot",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            build_self_snapshot::execute(&self.runtime, snapshot_input).await,
        )
        .await?;
        self.runtime.dashboard.record_tool_ok(
            "build_self_snapshot",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            format!(
                "snapshot built with {} evidence links",
                result.snapshot.evidence.len()
            ),
            &result,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok(
                    "build_self_snapshot",
                    dashboard_namespace,
                    Some(correlation_id),
                )
                .with_response_summary(serde_json::json!({
                    "snapshot_evidence_count": result.snapshot.evidence.len()
                })),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Return a bounded experimental action-string result using a provided self snapshot and server-side commitments.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<DecideWithSnapshotParams>>()
    )]
    async fn decide_with_snapshot(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "decide_with_snapshot",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<DecideWithSnapshotParams>(raw_params),
        )
        .await?;
        let auto_reflect_input = map_tool_error(
            &self.runtime,
            "decide_with_snapshot",
            params.auto_reflect_namespace.clone(),
            Some(correlation_id.clone()),
            AutoReflectInput::from_decide(&params),
        )
        .await?;
        let dashboard_namespace = params.auto_reflect_namespace.clone();
        let result = map_tool_error(
            &self.runtime,
            "decide_with_snapshot",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            decide_with_snapshot::execute(&self.runtime, params.into()).await,
        )
        .await?;
        if !result.blocked
            && let Some(auto_reflect_input) = auto_reflect_input
        {
            let auto_reflect_namespace = Some(auto_reflect_input.namespace.as_str().to_string());
            let auto_reflect_trigger_type = auto_reflect_input.trigger_type;
            let auto_reflect_trigger_key = auto_reflect_input.trigger_key();
            match auto_reflect_if_needed::execute(
                &self.runtime,
                auto_reflect_input.with_recursion_guard(RecursionGuard::Allow),
            )
            .await
            {
                Ok(diagnostics) => {
                    log_auto_reflection_success(
                        runtime_hook_for("decide_with_snapshot", auto_reflect_trigger_type),
                        &diagnostics,
                        None,
                        &self.runtime.dashboard,
                        auto_reflect_namespace.clone(),
                        Some(correlation_id.clone()),
                    );
                    self.runtime
                        .record_auto_reflection_operation(
                            "decide_with_snapshot",
                            &diagnostics,
                            auto_reflect_namespace,
                            Some(correlation_id.clone()),
                        )
                        .await;
                }
                Err(error) => {
                    warn!(
                        runtime_hook =
                            runtime_hook_for("decide_with_snapshot", auto_reflect_trigger_type),
                        trigger_type = ?auto_reflect_trigger_type,
                        trigger_key = %auto_reflect_trigger_key,
                        error = %error,
                        "best-effort conflict auto-reflection failed after successful decide_with_snapshot"
                    );
                    self.runtime
                        .record_auto_reflection_failure_operation(
                            "decide_with_snapshot",
                            auto_reflect_namespace,
                            Some(correlation_id.clone()),
                            auto_reflect_trigger_type,
                            &auto_reflect_trigger_key,
                            &error,
                        )
                        .await;
                }
            }
        }
        let summary = if result.blocked {
            "decision blocked by commitment gate".to_string()
        } else {
            "decision returned experimental non-authoritative model action".to_string()
        };
        self.runtime.dashboard.record_tool_ok(
            "decide_with_snapshot",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            summary,
            &result,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok(
                    "decide_with_snapshot",
                    dashboard_namespace,
                    Some(correlation_id),
                )
                .with_response_summary(serde_json::json!({
                    "blocked": result.blocked,
                    "decision_authority": result.decision_authority,
                    "policy_scope": result.policy_scope,
                })),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Supersede one scoped claim with an explicit replacement and same-scope evidence. The write reuses the existing run_reflection transaction, marks the old claim superseded, and does not hard delete. Missing, cross-scope, or mixed-namespace targets fail closed. Identity and commitment updates stay on run_reflection.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<SupersedeMemoryParams>>()
    )]
    async fn supersede_memory(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "supersede_memory",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<SupersedeMemoryParams>(raw_params),
        )
        .await?;
        let dashboard_namespace = Some(params.namespace.clone());
        let input = map_tool_error(
            &self.runtime,
            "supersede_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            supersede_memory::SupersedeMemoryInput::try_from(params),
        )
        .await?;
        let result = map_tool_error(
            &self.runtime,
            "supersede_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            supersede_memory::execute(&self.runtime, input).await,
        )
        .await?;
        let response_summary = serde_json::json!({
            "correction_type": "supersede",
            "durable_write_path": result.durable_write_path,
        });
        self.runtime.dashboard.record_tool_ok(
            "supersede_memory",
            dashboard_namespace.clone(),
            Some(correlation_id.clone()),
            "scoped claim superseded through run_reflection".to_string(),
            &response_summary,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok(
                    "supersede_memory",
                    dashboard_namespace,
                    Some(correlation_id),
                )
                .with_response_summary(response_summary),
            )
            .await;
        structured(result)
    }

    #[tool(
        description = "Record a governed Claim correction, or evidence-backed targetless record-only history with explicit origin_namespace. Targetless MCP calls cannot change identity or commitments; scope metadata grants no authority. Scoped Claim correction also uses supersede_memory. Optional global patches append self-model versions; expected_self_model_version guards the global head. Explicit component rollback additionally requires confirm:true, origin_namespace, an existing Claim target, evidence, and request_id, and appends a compensating version. Optional caller_budget limits caller-reported attempts and can explicitly stop unchanged or insufficient evidence before writes; it grants no authority.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<RunReflectionParams>>()
    )]
    async fn run_reflection(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let decision = caller_budget::admit_raw(
            &raw_params,
            crate::domain::caller_budget::CallerOperation::Reflection,
        )?;
        let correlation_id = generated_mcp_correlation_id();
        let params = map_tool_error(
            &self.runtime,
            "run_reflection",
            None,
            Some(correlation_id.clone()),
            decode_tool_params::<RunReflectionParams>(raw_params),
        )
        .await
        .map_err(|error| caller_budget::attach_error(error, &decision))?;
        let input = map_tool_error(
            &self.runtime,
            "run_reflection",
            None,
            Some(correlation_id.clone()),
            ReflectionInput::try_from(params),
        )
        .await
        .map_err(|error| caller_budget::attach_error(error, &decision))?;
        let result = map_tool_error(
            &self.runtime,
            "run_reflection",
            None,
            Some(correlation_id.clone()),
            run_reflection::execute(&self.runtime, input).await,
        )
        .await
        .map_err(|error| caller_budget::attach_error(error, &decision))?;
        self.runtime.dashboard.record_tool_ok(
            "run_reflection",
            None,
            Some(correlation_id.clone()),
            format!("reflection recorded {}", result.reflection_id),
            &result,
        );
        self.runtime
            .record_tool_operation(
                ToolOperationRecord::ok("run_reflection", None, Some(correlation_id))
                    .with_response_summary(
                        serde_json::json!({ "reflection_id": result.reflection_id }),
                    ),
            )
            .await;
        caller_budget::structured(result, decision)
    }
    #[tool(description = "Record a bounded, immutable scoped episode linked to existing same-scope events. Requires request_id for safe retry; observations and lessons are caller reports, not authenticated truth.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::CreateEpisodeRequest>>()
    )]
    async fn record_episode(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "record_episode",
            None,
            None,
            decode_tool_params::<crate::domain::experience::CreateEpisodeRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "record_episode",
            namespace,
            None,
            crate::application::experience::create_episode(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Read one scoped persisted rich episode including objective, actions, observations, result, lesson and source references.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::GetEpisodeRequest>>()
    )]
    async fn get_episode_detail(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "get_episode_detail",
            None,
            None,
            decode_tool_params::<crate::domain::experience::GetEpisodeRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "get_episode_detail",
            namespace,
            None,
            crate::application::experience::get_episode(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "List scoped persisted rich episodes with a bounded result limit.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::ListEpisodesRequest>>()
    )]
    async fn list_episode_details(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "list_episode_details",
            None,
            None,
            decode_tool_params::<crate::domain::experience::ListEpisodesRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "list_episode_details",
            namespace,
            None,
            crate::application::experience::list_episodes(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Persist an inspectable pending semantic or procedural candidate linked to scoped episodes. Procedure steps remain inert data; this never grants authority.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::CreateCandidateRequest>>()
    )]
    async fn propose_experience_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "propose_experience_candidate",
            None,
            None,
            decode_tool_params::<crate::domain::experience::CreateCandidateRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "propose_experience_candidate",
            namespace,
            None,
            crate::application::experience::create_candidate(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Read a scoped experience candidate current version or explicit historical version.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::GetCandidateRequest>>()
    )]
    async fn get_experience_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "get_experience_candidate",
            None,
            None,
            decode_tool_params::<crate::domain::experience::GetCandidateRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "get_experience_candidate",
            namespace,
            None,
            crate::application::experience::get_candidate(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "List scoped current experience candidates and their lifecycle states.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::ListCandidatesRequest>>()
    )]
    async fn list_experience_candidates(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "list_experience_candidates",
            None,
            None,
            decode_tool_params::<crate::domain::experience::ListCandidatesRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "list_experience_candidates",
            namespace,
            None,
            crate::application::experience::list_candidates(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Explicitly activate, reject or supersede experience with expected_version conflict protection. Activation only permits knowledge recall, never actions or permissions.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::UpdateCandidateStatusRequest>>()
    )]
    async fn set_experience_candidate_status(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "set_experience_candidate_status",
            None,
            None,
            decode_tool_params::<crate::domain::experience::UpdateCandidateStatusRequest>(
                raw_params,
            ),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "set_experience_candidate_status",
            namespace,
            None,
            crate::application::experience::update_candidate_status(&self.runtime.store, params)
                .await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Append a new pending experience version using expected_version. Previous versions remain inspectable; activation is a separate explicit step.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::ReviseCandidateRequest>>()
    )]
    async fn revise_experience_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "revise_experience_candidate",
            None,
            None,
            decode_tool_params::<crate::domain::experience::ReviseCandidateRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "revise_experience_candidate",
            namespace,
            None,
            crate::application::experience::revise_candidate(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Copy an earlier experience version into a new pending version after expected_version check. Preserves history and requires separate reactivation.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::RollbackCandidateRequest>>()
    )]
    async fn rollback_experience_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "rollback_experience_candidate",
            None,
            None,
            decode_tool_params::<crate::domain::experience::RollbackCandidateRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "rollback_experience_candidate",
            namespace,
            None,
            crate::application::experience::rollback_candidate(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Recall active current semantic/procedural experience within namespace and exact response-byte budget, with source episodes. Never executes procedures.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::experience::RecallCandidatesRequest>>()
    )]
    async fn recall_experience_candidates(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "recall_experience_candidates",
            None,
            None,
            decode_tool_params::<crate::domain::experience::RecallCandidatesRequest>(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.clone());
        let result = map_tool_error(
            &self.runtime,
            "recall_experience_candidates",
            namespace,
            None,
            crate::application::experience::recall_candidates(&self.runtime.store, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Read current scoped Claim fingerprint, status and object for an evidence-bound feedback correction.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::application::feedback_candidate::FeedbackTargetInput>>()
    )]
    async fn get_feedback_target_version(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "get_feedback_target_version",
            None,
            None,
            decode_tool_params::<crate::application::feedback_candidate::FeedbackTargetInput>(
                raw_params,
            ),
        )
        .await?;
        let namespace = Some(params.namespace.as_str().to_string());
        let result = map_tool_error(
            &self.runtime,
            "get_feedback_target_version",
            namespace,
            None,
            crate::application::feedback_candidate::get_target_version(&self.runtime, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Persist a pending object-only Claim revision candidate linked to structured feedback events. Does not commit the correction or authenticate tool reports.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::application::feedback_candidate::ProposeFeedbackCandidateInput>>()
    )]
    async fn propose_feedback_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "propose_feedback_candidate",
            None,
            None,
            decode_tool_params::<
                crate::application::feedback_candidate::ProposeFeedbackCandidateInput,
            >(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.as_str().to_string());
        let result = map_tool_error(
            &self.runtime,
            "propose_feedback_candidate",
            namespace,
            None,
            crate::application::feedback_candidate::propose(&self.runtime, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Inspect a scoped feedback candidate, validation reasons, lifecycle state and committed result references.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::application::feedback_candidate::GetFeedbackCandidateInput>>()
    )]
    async fn get_feedback_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "get_feedback_candidate",
            None,
            None,
            decode_tool_params::<crate::application::feedback_candidate::GetFeedbackCandidateInput>(
                raw_params,
            ),
        )
        .await?;
        let namespace = Some(params.namespace.as_str().to_string());
        let result = map_tool_error(
            &self.runtime,
            "get_feedback_candidate",
            namespace,
            None,
            crate::application::feedback_candidate::get(&self.runtime, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Deterministically validate feedback source category, exact target version, expected/actual alignment, scope and evidence existence. This does not establish semantic truth.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::application::feedback_candidate::FeedbackCandidateActionInput>>()
    )]
    async fn validate_feedback_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "validate_feedback_candidate",
            None,
            None,
            decode_tool_params::<
                crate::application::feedback_candidate::FeedbackCandidateActionInput,
            >(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.as_str().to_string());
        let result = map_tool_error(
            &self.runtime,
            "validate_feedback_candidate",
            namespace,
            None,
            crate::application::feedback_candidate::validate(&self.runtime, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Reject a pending or blocked feedback candidate with a persisted reason and durable retry receipt.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::application::feedback_candidate::RejectFeedbackCandidateInput>>()
    )]
    async fn reject_feedback_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "reject_feedback_candidate",
            None,
            None,
            decode_tool_params::<
                crate::application::feedback_candidate::RejectFeedbackCandidateInput,
            >(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.as_str().to_string());
        let result = map_tool_error(
            &self.runtime,
            "reject_feedback_candidate",
            namespace,
            None,
            crate::application::feedback_candidate::reject(&self.runtime, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(description = "Revalidate an evidence-bound feedback candidate and atomically commit its Claim correction, history, candidate state and receipt. Never alters identity, commitments or permissions.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::application::feedback_candidate::FeedbackCandidateActionInput>>()
    )]
    async fn commit_feedback_candidate(
        &self,
        raw_params: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let params = map_tool_error(
            &self.runtime,
            "commit_feedback_candidate",
            None,
            None,
            decode_tool_params::<
                crate::application::feedback_candidate::FeedbackCandidateActionInput,
            >(raw_params),
        )
        .await?;
        let namespace = Some(params.namespace.as_str().to_string());
        let result = map_tool_error(
            &self.runtime,
            "commit_feedback_candidate",
            namespace,
            None,
            crate::application::feedback_candidate::commit(&self.runtime, params).await,
        )
        .await?;
        structured(result)
    }

    #[tool(
        description = "Export safe same-scope durable memory as bounded inspectable JSON from one read snapshot. No database writes or logs, including on failure. Excludes global identity/commitments, operation logs/receipts, feedback candidates and derived indexes. This is not a replayable backup, secret-redaction service or authorization boundary. Limits fail closed rather than produce dangling truncated graphs.",
        input_schema = rmcp::handler::server::tool::cached_schema_for_type::<Parameters<crate::domain::ledger_export::ExportMemoryRequest>>()
    )]
    async fn export_memory(&self, raw_params: JsonObject) -> Result<CallToolResult, McpError> {
        // Export intentionally bypasses diagnostic wrappers too: a read-only
        // extraction must not leave operation-log rows when it succeeds or fails.
        let params =
            decode_tool_params::<crate::domain::ledger_export::ExportMemoryRequest>(raw_params)
                .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let result = crate::application::export_memory::export_memory(&self.runtime.store, params)
            .await
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        structured(result)
    }

    #[tool(
        description = "Inspect the rebuildable local FTS index without writing. Checks source projection, postings, and trigger definitions; ledger facts remain authoritative."
    )]
    async fn inspect_retrieval_index(&self) -> Result<CallToolResult, McpError> {
        let result = map_tool_error(
            &self.runtime,
            "inspect_retrieval_index",
            None,
            None,
            self.runtime.store.inspect_retrieval_index().await,
        )
        .await?;
        structured(result)
    }

    #[tool(
        description = "Explicitly rebuild only derived local retrieval tables and triggers from durable ledger facts in one transaction. Does not alter events, claims, permissions, identity or commitments."
    )]
    async fn rebuild_retrieval_index(&self) -> Result<CallToolResult, McpError> {
        let result = map_tool_error(
            &self.runtime,
            "rebuild_retrieval_index",
            None,
            None,
            self.runtime.store.rebuild_retrieval_index().await,
        )
        .await?;
        structured(result)
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some("Self-agent memory tools exposed over MCP stdio.".to_string()),
            ..Default::default()
        }
    }
}
