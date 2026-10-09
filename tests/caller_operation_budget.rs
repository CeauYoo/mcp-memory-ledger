use agent_llm_mm::{
    application::run_reflection::ReflectionInput,
    domain::caller_budget::{
        CALLER_BUDGET_CONTRACT, CallerBudget, CallerEvidenceSignal, CallerOperation,
        CallerOperationCounts, CallerStopReason, MAX_CALLER_OPERATION_COUNT,
    },
    interfaces::mcp::dto::{BuildTaskContextParams, RecallMemoryParams, RunReflectionParams},
};
use serde_json::{Value, json};

fn counts(retrievals: u32, reflections: u32, retries: u32) -> CallerOperationCounts {
    CallerOperationCounts {
        retrievals,
        reflections,
        retries,
    }
}

fn budget() -> CallerBudget {
    CallerBudget {
        limits: counts(3, 2, 1),
        used: counts(0, 0, 0),
        is_retry: false,
        evidence: CallerEvidenceSignal::Unknown,
    }
}

#[test]
fn first_call_unknown_evidence_is_admitted_without_inventing_progress() {
    for operation in [CallerOperation::Retrieval, CallerOperation::Reflection] {
        let decision = budget().evaluate(operation).unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.contract, CALLER_BUDGET_CONTRACT);
        assert_eq!(decision.stop_reason, None);
        assert_eq!(decision.next_used.retries, 0);
        assert_eq!(
            decision.next_used.retrievals + decision.next_used.reflections,
            1
        );
    }
}

#[test]
fn retry_consumes_both_counts_and_denial_consumes_neither() {
    let mut input = budget();
    input.is_retry = true;
    input.used = counts(1, 0, 0);
    let decision = input.evaluate(CallerOperation::Retrieval).unwrap();
    assert!(decision.allowed);
    assert_eq!(decision.next_used, counts(2, 0, 1));
    assert_eq!(decision.remaining, counts(1, 2, 0));
    input.used = decision.next_used;
    let stopped = input.evaluate(CallerOperation::Retrieval).unwrap();
    assert_eq!(
        stopped.stop_reason,
        Some(CallerStopReason::RetryBudgetExhausted)
    );
    assert_eq!(stopped.next_used, input.used);
    assert!(!stopped.allowed);
    input.is_retry = false;
    assert!(input.evaluate(CallerOperation::Retrieval).unwrap().allowed);
}

#[test]
fn each_operation_budget_stops_at_zero_equal_or_lowered_limit() {
    for used in [0, 1, MAX_CALLER_OPERATION_COUNT] {
        let mut input = budget();
        input.limits = counts(0, 0, 0);
        input.used = counts(used, used, used);
        for (operation, reason) in [
            (
                CallerOperation::Retrieval,
                CallerStopReason::RetrievalBudgetExhausted,
            ),
            (
                CallerOperation::Reflection,
                CallerStopReason::ReflectionBudgetExhausted,
            ),
        ] {
            let stopped = input.evaluate(operation).unwrap();
            assert_eq!(stopped.stop_reason, Some(reason));
            assert_eq!(stopped.next_used, input.used);
            assert_eq!(stopped.remaining, counts(0, 0, 0));
        }
    }
    let mut input = budget();
    input.used.retrievals = input.limits.retrievals;
    assert_eq!(
        input
            .evaluate(CallerOperation::Retrieval)
            .unwrap()
            .stop_reason,
        Some(CallerStopReason::RetrievalBudgetExhausted)
    );
    assert!(input.evaluate(CallerOperation::Reflection).unwrap().allowed);
}

#[test]
fn evidence_stops_only_explicit_reflection_and_has_stable_precedence() {
    for (evidence, reason) in [
        (
            CallerEvidenceSignal::Unchanged,
            CallerStopReason::NoNewEvidence,
        ),
        (
            CallerEvidenceSignal::Insufficient,
            CallerStopReason::InsufficientEvidence,
        ),
    ] {
        let mut input = budget();
        input.evidence = evidence;
        assert!(input.evaluate(CallerOperation::Retrieval).unwrap().allowed);
        input.is_retry = true;
        input.limits = counts(0, 0, 0);
        let stopped = input.evaluate(CallerOperation::Reflection).unwrap();
        assert_eq!(stopped.stop_reason, Some(reason));
        assert_eq!(stopped.next_used, input.used);
    }
    let mut input = budget();
    input.evidence = CallerEvidenceSignal::New;
    input.limits.reflections = 0;
    assert_eq!(
        input
            .evaluate(CallerOperation::Reflection)
            .unwrap()
            .stop_reason,
        Some(CallerStopReason::ReflectionBudgetExhausted)
    );
}

#[test]
fn bounded_counters_cannot_overflow_or_smuggle_invalid_unused_dimensions() {
    let mut input = budget();
    input.limits = counts(1000, 1000, 1000);
    input.used = counts(999, 999, 999);
    input.is_retry = true;
    let decision = input.evaluate(CallerOperation::Reflection).unwrap();
    assert_eq!(decision.next_used, counts(999, 1000, 1000));
    input.used = decision.next_used;
    assert!(!input.evaluate(CallerOperation::Reflection).unwrap().allowed);
    for field in ["limits", "used"] {
        for dimension in ["retrievals", "reflections", "retries"] {
            let mut value = serde_json::to_value(budget()).unwrap();
            value[field][dimension] = json!(1001);
            let parsed: CallerBudget = serde_json::from_value(value).unwrap();
            assert!(parsed.evaluate(CallerOperation::Retrieval).is_err());
        }
    }
}

#[test]
fn malformed_optional_budget_fails_closed_and_required_counts_never_reset() {
    for invalid in [
        json!(-1),
        json!(1.5),
        json!("1"),
        json!(true),
        json!(u64::MAX),
    ] {
        let mut value = serde_json::to_value(budget()).unwrap();
        value["used"]["retrievals"] = invalid;
        assert!(serde_json::from_value::<CallerBudget>(value).is_err());
    }
    for pointer in ["/limits", "/used", "/used/retrievals", "/limits/retries"] {
        let mut value = serde_json::to_value(budget()).unwrap();
        *value.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(serde_json::from_value::<CallerBudget>(value).is_err());
    }
    for pointer in ["", "/limits", "/used"] {
        let mut value = serde_json::to_value(budget()).unwrap();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("retrievels".into(), json!(99));
        assert!(serde_json::from_value::<CallerBudget>(value).is_err());
    }
    let mut value = serde_json::to_value(budget()).unwrap();
    value.as_object_mut().unwrap().remove("evidence");
    value.as_object_mut().unwrap().remove("is_retry");
    assert_eq!(
        serde_json::from_value::<CallerBudget>(value).unwrap(),
        budget()
    );
}

#[test]
fn optional_dto_fields_preserve_omitted_payloads_and_business_reflection_hash_input() {
    let recall = json!({"namespace":"project/budget", "query":"evidence"});
    let parsed: RecallMemoryParams = serde_json::from_value(recall.clone()).unwrap();
    assert!(parsed.caller_budget.is_none());
    assert!(
        serde_json::to_value(parsed)
            .unwrap()
            .get("caller_budget")
            .is_none()
    );
    let parsed: BuildTaskContextParams = serde_json::from_value(recall).unwrap();
    assert!(parsed.caller_budget.is_none());
    let reflection = json!({
        "reflection":{"summary":"An evidence-backed record"},
        "origin_namespace":"project/budget", "replacement_evidence_event_ids":["event:one"]
    });
    let original = ReflectionInput::try_from(
        serde_json::from_value::<RunReflectionParams>(reflection.clone()).unwrap(),
    )
    .unwrap();
    let mut with_budget = reflection;
    with_budget["caller_budget"] = serde_json::to_value(budget()).unwrap();
    let opted_in = ReflectionInput::try_from(
        serde_json::from_value::<RunReflectionParams>(with_budget).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(original).unwrap(),
        serde_json::to_value(opted_in).unwrap()
    );
}
