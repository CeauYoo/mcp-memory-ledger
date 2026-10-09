use chrono::{DateTime, Utc};

use crate::{
    domain::{claim::ClaimDraft, event::Event},
    error::AppError,
    ports::{ClaimStatus, Clock, IdGenerator, IngestTransactionRunner, StoredClaim, StoredEvent},
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IngestInput {
    event: Event,
    claim_drafts: Vec<ClaimDraft>,
    episode_reference: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    trigger_hints: Vec<String>,
}

impl IngestInput {
    pub fn new(
        event: Event,
        claim_drafts: Vec<ClaimDraft>,
        episode_reference: Option<String>,
    ) -> Self {
        Self {
            event,
            claim_drafts,
            episode_reference,
            request_id: None,
            trigger_hints: Vec::new(),
        }
    }

    pub fn with_trigger_hints(mut self, trigger_hints: Vec<String>) -> Self {
        self.trigger_hints = trigger_hints;
        self
    }

    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }

    fn into_parts(self) -> (Event, Vec<ClaimDraft>, Option<String>) {
        (self.event, self.claim_drafts, self.episode_reference)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IngestResult {
    pub event_id: String,
    #[serde(default)]
    pub replayed: bool,
}

impl IngestResult {
    fn from_event(event: &StoredEvent) -> Self {
        Self {
            event_id: event.event_id.clone(),
            replayed: false,
        }
    }
}

fn build_event(event_id: String, recorded_at: DateTime<Utc>, event: Event) -> StoredEvent {
    StoredEvent::new(event_id, recorded_at, event)
}

fn derive_claims(event_id: &str, drafts: Vec<ClaimDraft>) -> Result<Vec<StoredClaim>, AppError> {
    drafts
        .into_iter()
        .enumerate()
        .map(|(index, draft)| {
            draft.validate(0)?;
            Ok(StoredClaim::new(
                format!("{event_id}:claim:{index}"),
                draft,
                ClaimStatus::Active,
            ))
        })
        .collect()
}

pub async fn execute<D>(deps: &D, input: IngestInput) -> Result<IngestResult, AppError>
where
    D: IngestTransactionRunner + IdGenerator + Clock + Sync,
{
    let event_id = deps.next_id().await?;
    let request = crate::ports::WriteReceiptRequest::new(
        "ingest",
        input.event.namespace().as_str(),
        input.request_id.as_deref().unwrap_or(&event_id),
        &(
            &input.event,
            &input.claim_drafts,
            &input.episode_reference,
            &input.trigger_hints,
        ),
    )?;
    let (event, claim_drafts, episode_reference) = input.into_parts();
    let event = build_event(event_id, deps.now().await?, event);
    let mut transaction = deps.begin_ingest_transaction().await?;

    if let Some(receipt) = transaction
        .load_write_receipt(&request.operation_id)
        .await?
    {
        let mut result: IngestResult = receipt.replay(&request)?;
        result.replayed = true;
        transaction.commit().await?;
        return Ok(result);
    }
    if let Some(feedback) = event.event.feedback() {
        feedback.validate()?;
        for reference in &feedback.evidence_refs {
            let evidence = transaction
                .load_event_for_ingest(reference.event_id())
                .await?
                .ok_or_else(|| {
                    AppError::InvalidParams(
                        "feedback evidence must exist in the event namespace".into(),
                    )
                })?;
            if evidence.event.owner() != event.event.owner()
                || evidence.event.namespace() != event.event.namespace()
            {
                return Err(AppError::InvalidParams(
                    "feedback evidence must exist in the event namespace".into(),
                ));
            }
        }
    }
    transaction.append_event(event.clone()).await?;

    if let Some(episode_reference) = episode_reference {
        transaction
            .record_event_in_episode(episode_reference, event.event_id.clone())
            .await?;
    }

    for claim in derive_claims(&event.event_id, claim_drafts)? {
        transaction.upsert_claim(claim.clone()).await?;
        transaction
            .link_evidence(claim.claim_id.clone(), event.event_id.clone())
            .await?;
    }

    let result = IngestResult::from_event(&event);
    transaction
        .append_write_receipt(
            &request,
            crate::ports::write_receipt::receipt_result(&request, &result)?,
            event.recorded_at,
        )
        .await?;
    transaction.commit().await?;
    Ok(result)
}
