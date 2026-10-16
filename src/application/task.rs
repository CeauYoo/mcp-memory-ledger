//! Shared task service for MCP, CLI, dashboard and host adapters.
pub use crate::domain::task::*;
use crate::{
    domain::experience::{RecallCandidatesRequest, identifier},
    error::AppError,
    ports::{
        MemoryReadStore, experience_store::ExperienceStore, task_store::TaskStore,
        text_memory_store::TextMemoryStore,
    },
};

pub async fn create_task<S: TaskStore + Sync>(
    store: &S,
    r: CreateTaskRequest,
) -> Result<TaskWriteResult<TaskRevision>, AppError> {
    validate_create(&r)?;
    store.create_task(r).await
}
pub async fn revise_task<S: TaskStore + Sync>(
    store: &S,
    r: ReviseTaskRequest,
) -> Result<TaskWriteResult<TaskRevision>, AppError> {
    scope(&r.namespace)?;
    bounded(&r)?;
    store.revise_task(r).await
}
pub async fn get_task<S: TaskStore + Sync>(
    store: &S,
    r: GetTaskRequest,
) -> Result<Option<TaskRevision>, AppError> {
    scope(&r.namespace)?;
    identifier(&r.task_id, "task_id")?;
    store.get_task(r).await
}
pub async fn list_tasks<S: TaskStore + Sync>(
    store: &S,
    r: ListTasksRequest,
) -> Result<Vec<TaskRevision>, AppError> {
    scope(&r.namespace)?;
    crate::domain::experience::validate_limit(r.limit)?;
    store.list_tasks(r).await
}
pub async fn record_task_checkpoint<S: TaskStore + Sync>(
    store: &S,
    r: RecordTaskCheckpointRequest,
) -> Result<TaskWriteResult<TaskCheckpoint>, AppError> {
    validate_checkpoint(&r)?;
    store.record_task_checkpoint(r).await
}
pub async fn get_task_snapshot<S: TaskStore + Sync>(
    store: &S,
    namespace: &str,
    task_id: &str,
) -> Result<Option<TaskSnapshot>, AppError> {
    scope(namespace)?;
    identifier(task_id, "task_id")?;
    store.get_task_snapshot(namespace, task_id).await
}

fn size(context: &mut TaskResumeContext) -> Result<usize, AppError> {
    for _ in 0..8 {
        let n = serde_json::to_vec(context)
            .map_err(|e| AppError::Message(e.to_string()))?
            .len();
        if n == context.serialized_bytes {
            return Ok(n);
        }
        context.serialized_bytes = n;
    }
    Err(AppError::Message(
        "task context size did not converge".into(),
    ))
}
/// Task+checkpoint are read atomically. Optional recall uses separate reads, whose
/// exact versions are retained; the response never asserts a cross-store snapshot.
pub async fn build_task_resume_context<
    S: TaskStore
        + MemoryReadStore
        + TextMemoryStore
        + ExperienceStore
        + crate::ports::experience_use_store::ExperienceUseStore
        + Sync,
>(
    store: &S,
    r: TaskContextRequest,
) -> Result<TaskResumeContext, AppError> {
    let namespace = scope(&r.namespace)?;
    identifier(&r.task_id, "task_id")?;
    if !(1024..=262144).contains(&r.max_bytes) {
        return invalid("task context max_bytes must be 1024..262144 UTF-8 JSON bytes");
    }
    let snapshot = get_task_snapshot(store, &r.namespace, &r.task_id)
        .await?
        .ok_or_else(|| AppError::InvalidParams("task not found in requested namespace".into()))?;
    let mandatory: Vec<_> = snapshot
        .task
        .requirements
        .iter()
        .filter(|q| q.state == RequirementState::Reported)
        .cloned()
        .collect();
    let mut context = TaskResumeContext {
        namespace: r.namespace.clone(),
        task_id: r.task_id.clone(),
        title: snapshot.task.title.clone(),
        task_version: snapshot.task.version,
        status: snapshot.task.status,
        context_id: format!("task-context:{}", uuid::Uuid::new_v4()),
        complete: true,
        can_resume: snapshot.task.status == TaskStatus::Active && !mandatory.is_empty(),
        mandatory_requirement_count: mandatory.len(),
        mandatory_requirements: mandatory,
        checkpoint: snapshot.checkpoint,
        diagnostics: vec!["sources_and_verification_are_caller_reports_not_authenticated".into()],
        snapshot_consistent: true,
        mandatory_snapshot_consistent: true,
        task_version_after_selection: None,
        related_memory: None,
        experiences: vec![],
        serialized_bytes: 0,
        max_bytes: r.max_bytes,
        authority: "knowledge_only_no_execution_authority",
        verification_basis: "caller_reported_not_independently_verified",
        artifact_verification: crate::domain::feedback::FeedbackVerificationResult::NotPerformed,
    };
    if context.mandatory_requirement_count == 0 {
        context
            .diagnostics
            .push("no_reported_requirements_candidates_need_review".into());
    }
    if context.status != TaskStatus::Active {
        context
            .diagnostics
            .push("task_not_active_do_not_resume".into());
    }
    if let Some(cp) = &context.checkpoint {
        context
            .diagnostics
            .push("checkpoint_artifacts_and_results_require_current_revalidation".into());
        if cp.task_version != context.task_version {
            context
                .diagnostics
                .push("stale_checkpoint_task_version".into());
            context.can_resume = false;
        }
    }
    // Derived reuse eligibility is computed anew, never a persisted trusted cache.
    match super::experience_use::list_experience_uses(
        store,
        crate::domain::experience_use::ListExperienceUsesRequest {
            namespace: r.namespace.clone(),
            task_id: r.task_id.clone(),
            limit: 100,
            after_id: None,
        },
    )
    .await
    {
        Ok(uses) => {
            if uses.iter().any(|u| {
                !u.currently_reusable
                    && u.record.observation.decision
                        == crate::domain::experience_use::ExperienceUseDecision::Adopted
            }) {
                context.diagnostics.push(
                    "prior_adopted_experience_invalidated_review_history_before_reuse".into(),
                );
            }
            if uses.len() == 100 {
                context.diagnostics.push("experience_use_invalidation_scan_bounded_paginate_history_for_remaining_records".into());
            }
        }
        Err(_) => context.diagnostics.push(
            "experience_use_invalidation_status_unavailable_do_not_assume_prior_reuse_valid".into(),
        ),
    }
    // Required task data always takes precedence. Reserve space for honest selection diagnostics.
    let base = size(&mut context)?;
    if base.saturating_add(1024) < r.max_bytes {
        context.snapshot_consistent = false;
        context
            .diagnostics
            .push("related_memory_and_experience_reads_are_not_one_atomic_snapshot".into());
        let budget = (r.max_bytes - base - 1024) / 2;
        if budget >= 1024 {
            match super::build_task_context::execute(
                store,
                super::build_task_context::BuildTaskContextInput {
                    namespace,
                    query: snapshot.task.title.clone(),
                    limit: 8,
                    max_bytes: budget,
                },
            )
            .await
            {
                Ok(memory) => {
                    context.related_memory = Some(
                        serde_json::to_value(memory)
                            .map_err(|e| AppError::Message(e.to_string()))?,
                    )
                }
                Err(_) => context
                    .diagnostics
                    .push("optional_related_memory_unavailable".into()),
            }
            match super::experience::recall_candidates(
                store,
                RecallCandidatesRequest {
                    namespace: r.namespace.clone(),
                    query: snapshot.task.title,
                    limit: 8,
                    max_bytes: budget.min(65536),
                },
            )
            .await
            {
                Ok(experiences) => {
                    if experiences.truncated {
                        context
                            .diagnostics
                            .push("optional_experiences_truncated".into());
                    }
                    context.experiences = experiences.candidates;
                }
                Err(_) => context
                    .diagnostics
                    .push("optional_experiences_unavailable".into()),
            }
        }
        // Optional experience is a read-time suggestion, never an applicability proof.
        context.diagnostics.push(
            "experience_applicability_unknown_review_conditions_and_limitations_before_adoption"
                .into(),
        );
        let mut revalidated = Vec::new();
        for selected in std::mem::take(&mut context.experiences) {
            let current = super::experience::get_candidate(
                store,
                crate::domain::experience::GetCandidateRequest {
                    namespace: r.namespace.clone(),
                    candidate_id: selected.candidate_id.clone(),
                    version: None,
                },
            )
            .await;
            match current {
                Ok(Some(current))
                    if current.version == selected.version
                        && current.status
                            == crate::domain::experience::ExperienceStatus::Active =>
                {
                    revalidated.push(selected)
                }
                _ => context
                    .diagnostics
                    .push("experience_invalidated_during_selection_omitted".into()),
            }
        }
        context.experiences = revalidated;
        let latest = get_task_snapshot(store, &r.namespace, &r.task_id)
            .await?
            .ok_or_else(|| {
                AppError::InvalidParams("task disappeared during context selection".into())
            })?;
        context.task_version_after_selection = Some(latest.task.version);
        if latest.task.version != context.task_version
            || latest.checkpoint.as_ref().map(|c| c.sequence)
                != context.checkpoint.as_ref().map(|c| c.sequence)
        {
            context.complete = false;
            context.can_resume = false;
            context
                .diagnostics
                .push("stale_task_or_checkpoint_changed_during_selection_rebuild_context".into());
        }
    } else {
        context
            .diagnostics
            .push("optional_recall_skipped_for_required_task_budget".into());
    }
    // Omit whole optional blocks, never individual hard constraints.
    if size(&mut context)? > r.max_bytes && !context.experiences.is_empty() {
        context.experiences.clear();
        context
            .diagnostics
            .push("optional_experiences_omitted_byte_budget".into());
    }
    if size(&mut context)? > r.max_bytes && context.related_memory.is_some() {
        context.related_memory = None;
        context
            .diagnostics
            .push("optional_memory_omitted_byte_budget".into());
    }
    if size(&mut context)? > r.max_bytes && context.checkpoint.is_some() {
        context.checkpoint = None;
        context.complete = false;
        context.can_resume = false;
        context
            .diagnostics
            .push("checkpoint_omitted_byte_budget_read_task_snapshot_or_increase_budget".into());
    }
    if size(&mut context)? > r.max_bytes {
        context.mandatory_requirements.clear();
        context.complete = false;
        context.can_resume = false;
        context.diagnostics.push(
            "mandatory_requirements_do_not_fit_narrow_task_or_increase_budget_do_not_continue"
                .into(),
        );
    }
    if size(&mut context)? > r.max_bytes {
        return invalid("task context metadata cannot fit budget; increase max_bytes");
    }
    Ok(context)
}
