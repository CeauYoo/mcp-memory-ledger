use super::{
    protocol::{Input, token},
    state::{Link, Observation, Record, State},
    transport::{self, CallError},
};
use crate::adapters::codex_context::protocol::Binding;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::PathBuf;
#[async_trait::async_trait]
pub trait Core: Sync {
    async fn call(
        &self,
        binding: &Binding,
        name: &str,
        args: Value,
    ) -> std::result::Result<Value, CallError>;
}
pub struct Bridge {
    pub executable: PathBuf,
    pub socket: PathBuf,
}
#[async_trait::async_trait]
impl Core for Bridge {
    async fn call(
        &self,
        b: &Binding,
        name: &str,
        args: Value,
    ) -> std::result::Result<Value, CallError> {
        let exe = self.executable.clone();
        let socket = self.socket.clone();
        let b = b.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || transport::call(&exe, &socket, &b, &name, args))
            .await
            .map_err(|_| CallError::Uncertain)?
    }
}
fn read_error(_: CallError) -> anyhow::Error {
    anyhow::anyhow!("scoped core read unavailable")
}
pub async fn make_link(core: &impl Core, b: Binding, input: &Input, use_id: &str) -> Result<Link> {
    token(use_id)?;
    ensure!(
        input.hook_event_name == "PreToolUse" && input.session_id == b.host_session_id,
        "pre-action binding mismatch"
    );
    let started = std::time::Instant::now();
    let mut after = Value::Null;
    for _ in 0..100 {
        ensure!(
            started.elapsed() < std::time::Duration::from_secs(8),
            "bounded use lookup timeout"
        );
        let response = core
            .call(
                &b,
                "list_experience_uses",
                json!({"namespace":b.namespace,"task_id":b.task_id,"limit":50,"after_id":after}),
            )
            .await
            .map_err(read_error)?;
        let rows = response.as_array().context("invalid use response")?;
        ensure!(rows.len() <= 50, "oversized use page");
        for row in rows {
            let o = &row["record"]["observation"];
            if o["use_id"] == use_id {
                ensure!(
                    row["currently_reusable"] == true
                        && o["namespace"] == b.namespace
                        && o["task_id"] == b.task_id,
                    "use not currently applicable in scope"
                );
                let link = Link {
                    binding: b,
                    turn_id: input.turn_id.clone(),
                    tool_id: input.tool_use_id.clone(),
                    tool_name: input.tool_name.clone(),
                    input_digest: input.input_digest()?,
                    use_id: use_id.into(),
                    task_version: o["task_version"].as_i64().context("missing task version")?,
                    candidate_id: o["candidate_id"]
                        .as_str()
                        .context("missing candidate")?
                        .into(),
                    candidate_version: o["candidate_version"]
                        .as_i64()
                        .context("missing candidate version")?,
                    decision: serde_json::from_value(o["decision"].clone())?,
                };
                link.validate()?;
                return Ok(link);
            }
        }
        if rows.len() < 50 {
            break;
        }
        let next = rows.last().unwrap()["record"]["observation"]["use_id"].clone();
        ensure!(next.is_string() && next != after, "invalid pagination");
        after = next;
    }
    anyhow::bail!("explicit use not found within bounded lookup")
}
pub async fn capture(state: &State, mut r: Record, input: &Input) -> Result<Record> {
    ensure!(
        input.hook_event_name == "PostToolUse"
            && input.session_id == r.link.binding.host_session_id
            && input.tool_use_id == r.link.tool_id
            && input.tool_name == r.link.tool_name
            && input.turn_id == r.link.turn_id
            && input.input_digest()? == r.link.input_digest,
        "post-tool linkage conflict"
    );
    let response_digest = input.response_digest()?;
    let response_bytes = input.response_bytes()?;
    let evidence_digest = Observation::digest_for(&r.link, &response_digest, response_bytes)?;
    if let Some(old) = &r.observation {
        ensure!(
            old.evidence_digest == evidence_digest
                && old.response_digest == response_digest
                && old.response_bytes == response_bytes,
            "result replay conflict"
        );
        return Ok(r);
    }
    let old = r.clone();
    r.observation = Some(Observation {
        response_digest,
        evidence_digest,
        response_bytes,
        observed_at: chrono::Utc::now().to_rfc3339(),
    });
    state.save(&old, &r).await?;
    Ok(r)
}
pub(super) fn event_request(r: &Record) -> Result<Value> {
    let o = r.observation.as_ref().context("result not captured")?;
    let l = &r.link;
    Ok(
        json!({"request_id":format!("host-result-{}:event",l.key()),"observed_at":o.observed_at,"event":{"owner":"World","namespace":l.binding.namespace,"kind":"Observation","summary":format!("Host-reported tool output observed; session={} turn={} tool={} action={} evidence_sha256={}; independent outcome unknown.",l.binding.host_session_id,l.turn_id,l.tool_name,l.tool_id,o.evidence_digest),"feedback":{"source_kind":"tool_reported","producer":"codex-results/pinned-contract-0.159.2; origin unverified","observed_target":format!("task:{};tool:{}",l.binding.task_id,l.tool_id),"observed_version":l.task_version.to_string(),"expected":"Collect output metadata only; no outcome criterion evaluated","actual":"output_observed; raw input and output omitted","verification_method":"bounded metadata capture; no independent verification","verification_result":"not_performed","limitations":["Host origin is not authenticated; local caller may spoof payload","No success, applicability, causal attribution, or execution permission established","Digests are correlation evidence, not authentication or secret anonymization"],"evidence_refs":[]}},"claim_drafts":[],"episode_reference":null,"trigger_hints":[]}),
    )
}
async fn use_request(core: &impl Core, state: &State, r: &mut Record) -> Result<Value> {
    let l = r.link.clone();
    let b = &l.binding;
    let t = core
        .call(
            b,
            "get_task",
            json!({"namespace":b.namespace,"task_id":b.task_id}),
        )
        .await
        .map_err(read_error)?;
    ensure!(
        t["namespace"] == b.namespace && t["task_id"] == b.task_id,
        "task head scope mismatch"
    );
    let tv = t["version"].as_i64().context("missing task version")?;
    ensure!(tv >= r.minimum_task_head, "task head rollback");
    if tv > r.minimum_task_head {
        let old = r.clone();
        r.minimum_task_head = tv;
        state.save(&old, r).await?;
    }
    // Commit each observed floor before another read can fail. A partial lookup
    // must not erase a newer revision already observed by this outbox.
    let c = core
        .call(
            b,
            "get_experience_candidate",
            json!({"namespace":b.namespace,"candidate_id":l.candidate_id}),
        )
        .await
        .map_err(read_error)?;
    ensure!(
        c["namespace"] == b.namespace && c["candidate_id"] == l.candidate_id,
        "candidate head scope mismatch"
    );
    let cv = c["version"].as_i64().context("missing candidate version")?;
    ensure!(cv >= r.minimum_candidate_head, "candidate head rollback");
    if cv > r.minimum_candidate_head {
        let old = r.clone();
        r.minimum_candidate_head = cv;
        state.save(&old, r).await?;
    }
    build_use_request(r, tv, cv)
}
pub(super) fn build_use_request(r: &Record, tv: i64, cv: i64) -> Result<Value> {
    let l = &r.link;
    let b = &l.binding;
    ensure!(
        tv >= r.minimum_task_head && cv >= r.minimum_candidate_head,
        "head version rollback"
    );
    let id = format!("host-result-{}", l.key());
    let request = json!({"namespace":b.namespace,"request_id":format!("{}:use:{}",id,r.use_attempt),"use_id":id,"task_id":b.task_id,"task_version":l.task_version,"expected_task_version":tv,"candidate_id":l.candidate_id,"candidate_version":l.candidate_version,"expected_candidate_version":cv,"decision":l.decision,"reason":"Automatic host-reported output observation for explicitly linked use; no causal claim","conditions":[{"condition":"Applicability of the previously selected experience has not been reevaluated","assessment":"unknown"}],"action_event_refs":[],"outcome_event_refs":[format!("event:{}",r.event_id.as_ref().context("event missing")?)],"verification":"not_performed","evaluation_method":"Pinned PostToolUse metadata only; host origin and result correctness unverified","limitations":["Knowledge observation only; not execution permission","Tool completion does not establish success or causal benefit"],"related_use_id":l.use_id});
    // Run the SAME domain validator used by the server before persisting a request.
    serde_json::from_value::<crate::domain::experience_use::RecordExperienceUseRequest>(
        request.clone(),
    )?
    .validate()?;
    Ok(request)
}
/// No automatic retry after a confirmed rejection. Uncertain writes retain exact request/key.
pub async fn deliver(
    core: &impl Core,
    state: &State,
    mut r: Record,
    retry_rejected: bool,
) -> Result<Record> {
    r.validate()?;
    ensure!(r.observation.is_some(), "result not captured");
    if r.completed {
        return Ok(r);
    }
    if r.event_id.is_none() {
        if r.event_request.is_none() {
            let request = event_request(&r)?;
            let old = r.clone();
            r.event_request = Some(request);
            state.save(&old, &r).await?;
        }
        let request = r.event_request.clone().unwrap();
        if r.event_rejected {
            ensure!(
                retry_rejected,
                "event delivery rejected; explicit retry-rejected required"
            );
            let old = r.clone();
            r.event_rejected = false;
            state.save(&old, &r).await?;
        }
        let response = match core
            .call(&r.link.binding, "ingest_interaction", request)
            .await
        {
            Ok(value) => value,
            Err(CallError::Rejected) => {
                let old = r.clone();
                r.event_rejected = true;
                state.save(&old, &r).await?;
                anyhow::bail!("event delivery rejected; explicit retry-rejected required")
            }
            Err(CallError::Uncertain) => {
                anyhow::bail!("event delivery uncertain; exact replay required")
            }
        };
        let event_id = response["event_id"]
            .as_str()
            .context("event response uncertain; exact replay required")?;
        token(event_id)?;
        let old = r.clone();
        r.event_id = Some(event_id.into());
        state.save(&old, &r).await?;
    }
    if r.rejected {
        ensure!(
            retry_rejected,
            "use delivery rejected; explicit retry-rejected required"
        );
        ensure!(r.use_attempt < 32, "bounded retry limit reached");
        let old = r.clone();
        r.use_attempt += 1;
        r.use_request = None;
        r.rejected = false;
        state.save(&old, &r).await?;
    }
    if r.use_request.is_none() {
        let request = use_request(core, state, &mut r).await?;
        let old = r.clone();
        r.minimum_task_head = request["expected_task_version"]
            .as_i64()
            .context("invalid task head")?;
        r.minimum_candidate_head = request["expected_candidate_version"]
            .as_i64()
            .context("invalid candidate head")?;
        r.use_request = Some(request);
        state.save(&old, &r).await?;
    }
    let response = core
        .call(
            &r.link.binding,
            "record_experience_use",
            r.use_request.clone().unwrap(),
        )
        .await;
    match response {
        Ok(v) => {
            let expected: crate::domain::experience_use::RecordExperienceUseRequest =
                serde_json::from_value(r.use_request.clone().unwrap())?;
            let expected = crate::domain::experience_use::ExperienceUseObservation::from(expected);
            ensure!(
                v["record"]["observation"] == serde_json::to_value(expected)?,
                "use response uncertain; exact replay required"
            );
            let old = r.clone();
            r.completed = true;
            state.save(&old, &r).await?;
            Ok(r)
        }
        Err(CallError::Rejected) => {
            let old = r.clone();
            r.rejected = true;
            state.save(&old, &r).await?;
            anyhow::bail!("use delivery rejected; explicit retry-rejected required")
        }
        Err(CallError::Uncertain) => anyhow::bail!("use delivery uncertain; exact replay required"),
    }
}
