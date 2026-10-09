use agent_llm_mm::{
    application::{
        build_self_snapshot::BuildSelfSnapshotInput,
        get_evidence_relation::GetEvidenceRelationInput,
        get_memory::{GetMemoryInput, MemoryRecordReference},
        get_reflection_history::{DEFAULT_REFLECTION_HISTORY_LIMIT, GetReflectionHistoryInput},
        get_self_model_history::{DEFAULT_SELF_MODEL_HISTORY_LIMIT, GetSelfModelHistoryInput},
        search_memory::{MemoryRecordType, SearchMemoryInput},
        supersede_memory::SupersedeMemoryInput,
    },
    domain::{
        claim::ClaimDraft,
        event::{Event, MAX_EVIDENCE_MANIFEST_ITEMS},
        types::Owner,
    },
    interfaces::mcp::dto::{
        BuildSelfSnapshotParams, ClaimDraftDto, EventDto, EventKindDto, EvidenceQueryDto,
        GetEvidenceRelationParams, GetMemoryParams, GetReflectionHistoryParams,
        GetSelfModelHistoryParams, MemoryRecordTypeDto, ModeDto, OwnerDto, SearchMemoryParams,
        SelfModelHistoryTypeDto, SupersedeMemoryParams,
    },
    ports::{ClaimStatus, EvidenceQuery, SelfModelHistoryKind},
};
use chrono::{DateTime, Utc};

#[test]
fn get_memory_dto_defaults_ambiguous_raw_ids_to_event_for_compatibility() {
    let input = GetMemoryInput::try_from(GetMemoryParams {
        namespace: "project/dto".to_string(),
        id: "claim:legacy-event-id".to_string(),
        record_type: None,
    })
    .expect("omitted record type should preserve Event parsing");

    match input.id {
        MemoryRecordReference::Event(reference) => {
            assert_eq!(reference.event_id(), "claim:legacy-event-id");
        }
        MemoryRecordReference::Claim(_) => panic!("omitted record type must not select Claim"),
        MemoryRecordReference::Episode(_) => panic!("omitted record type must not select Episode"),
        MemoryRecordReference::Reflection(_) => {
            panic!("omitted record type must not select Reflection")
        }
    }
}

#[test]
fn get_memory_dto_uses_explicit_claim_type_for_raw_or_canonical_claim_ids() {
    for id in ["stored-claim-id", "claim:stored-claim-id"] {
        let input = GetMemoryInput::try_from(GetMemoryParams {
            namespace: "project/dto".to_string(),
            id: id.to_string(),
            record_type: Some(MemoryRecordTypeDto::Claim),
        })
        .expect("explicit Claim record type should parse the claim ID");

        match input.id {
            MemoryRecordReference::Claim(reference) => {
                assert_eq!(reference.claim_id(), "stored-claim-id");
            }
            MemoryRecordReference::Event(_) => panic!("explicit Claim must not select Event"),
            MemoryRecordReference::Episode(_) => panic!("explicit Claim must not select Episode"),
            MemoryRecordReference::Reflection(_) => {
                panic!("explicit Claim must not select Reflection")
            }
        }
    }
}

#[test]
fn search_memory_dto_adds_episode_without_widening_get_memory_record_types() {
    let params = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto",
        "record_type": "Episode",
        "episode_reference": "episode:Persisted-Exactly",
        "limit": 7
    }))
    .expect("search_memory should deserialize the Episode record type");
    let input = SearchMemoryInput::try_from(params).expect("Episode search should convert");

    assert_eq!(input.single_record_type(), Some(MemoryRecordType::Episode));
    assert_eq!(
        input.episode_reference.as_deref(),
        Some("episode:Persisted-Exactly")
    );
    assert_eq!(input.limit, 7);
    assert!(input.claim_status.is_none());

    let get_episode = GetMemoryInput::try_from(
        serde_json::from_value::<GetMemoryParams>(serde_json::json!({
            "namespace": "project/dto",
            "id": "episode:Persisted-Exactly",
            "record_type": "Episode"
        }))
        .expect("get_memory should deserialize the Episode record type"),
    )
    .expect("explicit Episode lookup should convert");
    match get_episode.id {
        MemoryRecordReference::Episode(reference) => {
            assert_eq!(reference, "episode:Persisted-Exactly");
        }
        MemoryRecordReference::Event(_)
        | MemoryRecordReference::Claim(_)
        | MemoryRecordReference::Reflection(_) => {
            panic!("explicit Episode must keep the opaque persisted reference")
        }
    }
}

#[test]
fn get_memory_dto_rejects_invalid_episode_references() {
    for id in ["", "   ", " episode:trimmed", "episode:trimmed "] {
        let error = GetMemoryInput::try_from(GetMemoryParams {
            namespace: "project/dto".to_string(),
            id: id.to_string(),
            record_type: Some(MemoryRecordTypeDto::Episode),
        })
        .expect_err("empty or boundary-whitespace Episode ids must fail closed");
        assert!(error.to_string().contains("episode_reference"));
    }
}

#[test]
fn search_memory_dto_keeps_omitted_record_type_as_event() {
    let params = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto"
    }))
    .unwrap();
    let input = SearchMemoryInput::try_from(params).unwrap();

    assert_eq!(input.single_record_type(), Some(MemoryRecordType::Event));
    assert!(input.episode_reference.is_none());
}

#[test]
fn search_memory_dto_validates_exact_episode_filters_and_type_compatibility() {
    for episode_reference in ["", "   ", " episode:trimmed", "episode:trimmed "] {
        let params = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
            "namespace": "project/dto",
            "record_type": "Episode",
            "episode_reference": episode_reference
        }))
        .unwrap();
        let error = SearchMemoryInput::try_from(params)
            .expect_err("empty or boundary-whitespace episode references must fail closed");
        assert!(error.to_string().contains("episode_reference"));
    }

    for params in [
        serde_json::json!({
            "namespace": "project/dto",
            "record_type": "Episode",
            "kind": "Observation"
        }),
        serde_json::json!({
            "namespace": "project/dto",
            "episode_reference": "episode:event-filter"
        }),
        serde_json::json!({
            "namespace": "project/dto",
            "record_type": "Claim",
            "episode_reference": "episode:claim-filter"
        }),
        serde_json::json!({
            "namespace": "project/dto",
            "record_type": "Reflection",
            "episode_reference": "episode:reflection-filter"
        }),
    ] {
        let params = serde_json::from_value::<SearchMemoryParams>(params).unwrap();
        assert!(
            SearchMemoryInput::try_from(params).is_err(),
            "record-type-specific filters must not be accepted by another record type"
        );
    }
}

#[test]
fn search_memory_dto_adds_reflection_without_widening_get_memory_record_types() {
    let params = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto",
        "record_type": "Reflection",
        "reflection_reference": "reflection-persisted",
        "limit": 9
    }))
    .expect("search_memory should deserialize the Reflection record type");
    let input = SearchMemoryInput::try_from(params).expect("Reflection search should convert");

    assert_eq!(
        input.single_record_type(),
        Some(MemoryRecordType::Reflection)
    );
    assert_eq!(
        input.reflection_reference.as_deref(),
        Some("reflection-persisted")
    );
    assert_eq!(input.limit, 9);
    assert!(input.claim_status.is_none());

    let get_reflection = GetMemoryInput::try_from(
        serde_json::from_value::<GetMemoryParams>(serde_json::json!({
            "namespace": "project/dto",
            "id": "reflection-persisted",
            "record_type": "Reflection"
        }))
        .expect("get_memory should deserialize the Reflection record type"),
    )
    .expect("explicit Reflection lookup should convert");
    match get_reflection.id {
        MemoryRecordReference::Reflection(reference) => {
            assert_eq!(reference, "reflection-persisted");
        }
        MemoryRecordReference::Event(_)
        | MemoryRecordReference::Claim(_)
        | MemoryRecordReference::Episode(_) => {
            panic!("explicit Reflection must keep the opaque persisted id")
        }
    }
}

#[test]
fn get_memory_dto_rejects_invalid_reflection_references() {
    for id in ["", "   ", " reflection-trimmed", "reflection-trimmed "] {
        let error = GetMemoryInput::try_from(GetMemoryParams {
            namespace: "project/dto".to_string(),
            id: id.to_string(),
            record_type: Some(MemoryRecordTypeDto::Reflection),
        })
        .expect_err("empty or boundary-whitespace Reflection ids must fail closed");
        assert!(error.to_string().contains("reflection_reference"));
    }
}

#[test]
fn search_memory_dto_validates_exact_reflection_filters_and_type_compatibility() {
    for reflection_reference in ["", "   ", " reflection-trimmed", "reflection-trimmed "] {
        let params = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
            "namespace": "project/dto",
            "record_type": "Reflection",
            "reflection_reference": reflection_reference
        }))
        .unwrap();
        let error = SearchMemoryInput::try_from(params)
            .expect_err("empty or boundary-whitespace reflection references must fail closed");
        assert!(error.to_string().contains("reflection_reference"));
    }

    for params in [
        serde_json::json!({
            "namespace": "project/dto",
            "record_type": "Reflection",
            "kind": "Observation"
        }),
        serde_json::json!({
            "namespace": "project/dto",
            "reflection_reference": "reflection-event-filter"
        }),
        serde_json::json!({
            "namespace": "project/dto",
            "record_type": "Episode",
            "reflection_reference": "reflection-episode-filter"
        }),
    ] {
        let params = serde_json::from_value::<SearchMemoryParams>(params).unwrap();
        assert!(
            SearchMemoryInput::try_from(params).is_err(),
            "Reflection-specific filters must stay bound to record_type Reflection"
        );
    }
}

#[test]
fn search_memory_dto_adds_union_record_types_without_widening_get_memory() {
    let params = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto",
        "record_types": ["Event", "Claim", "Episode", "Reflection"],
        "limit": 8
    }))
    .expect("search_memory should deserialize a union record_types list");
    let input = SearchMemoryInput::try_from(params).expect("union search should convert");

    assert!(input.is_union());
    assert_eq!(input.single_record_type(), None);
    assert_eq!(
        input.record_types,
        vec![
            MemoryRecordType::Event,
            MemoryRecordType::Claim,
            MemoryRecordType::Episode,
            MemoryRecordType::Reflection
        ]
    );
    assert_eq!(input.limit, 8);
    assert!(input.claim_status.is_none());
}

#[test]
fn search_memory_dto_validates_union_filters_and_type_compatibility() {
    let both = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto",
        "record_type": "Event",
        "record_types": ["Claim"]
    }))
    .unwrap();
    assert!(
        SearchMemoryInput::try_from(both).is_err(),
        "record_type and record_types must stay mutually exclusive"
    );

    let empty = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto",
        "record_types": []
    }))
    .unwrap();
    assert!(
        SearchMemoryInput::try_from(empty).is_err(),
        "empty record_types must fail closed"
    );

    let duplicates = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto",
        "record_types": ["Event", "Event"]
    }))
    .unwrap();
    assert!(
        SearchMemoryInput::try_from(duplicates).is_err(),
        "duplicate record_types must fail closed"
    );

    for params in [
        serde_json::json!({
            "namespace": "project/dto",
            "record_types": ["Event", "Claim"],
            "kind": "Observation"
        }),
        serde_json::json!({
            "namespace": "project/dto",
            "record_types": ["Event", "Episode"],
            "episode_reference": "episode:union-filter"
        }),
        serde_json::json!({
            "namespace": "project/dto",
            "record_types": ["Claim", "Reflection"],
            "claim_status": "Disputed"
        }),
    ] {
        let params = serde_json::from_value::<SearchMemoryParams>(params).unwrap();
        assert!(
            SearchMemoryInput::try_from(params).is_err(),
            "union searches must reject type-specific filters"
        );
    }

    let single_via_array = serde_json::from_value::<SearchMemoryParams>(serde_json::json!({
        "namespace": "project/dto",
        "record_types": ["Claim"]
    }))
    .unwrap();
    let input = SearchMemoryInput::try_from(single_via_array)
        .expect("single-element record_types should keep Claim search semantics");
    assert_eq!(input.single_record_type(), Some(MemoryRecordType::Claim));
    assert_eq!(input.claim_status, Some(ClaimStatus::Active));
}

#[test]
fn get_evidence_relation_dto_parses_mixed_references_and_defaults_selection() {
    let input = GetEvidenceRelationInput::try_from(GetEvidenceRelationParams {
        namespace: "project/dto".to_string(),
        trigger_window_event_ids: vec![
            "event:evt-new".to_string(),
            "evt-mid".to_string(),
            "evt-new".to_string(),
        ],
        selected_evidence_event_ids: None,
        selection_basis: None,
    })
    .expect("raw and canonical trigger ids should parse");

    assert_eq!(
        input
            .trigger_window
            .iter()
            .map(|reference| reference.event_id())
            .collect::<Vec<_>>(),
        vec!["evt-new", "evt-mid"]
    );
    assert!(input.selected_evidence.is_empty());
}

#[test]
fn get_evidence_relation_dto_rejects_invalid_scope_ids_and_limits() {
    let missing_namespace = GetEvidenceRelationInput::try_from(GetEvidenceRelationParams {
        namespace: "invalid".to_string(),
        trigger_window_event_ids: vec!["evt-1".to_string()],
        selected_evidence_event_ids: None,
        selection_basis: None,
    })
    .expect_err("invalid namespace should fail closed");
    assert!(missing_namespace.to_string().contains("InvalidNamespace"));

    for invalid in ["", " ", "event:", "event:event:evt-window"] {
        let error = GetEvidenceRelationInput::try_from(GetEvidenceRelationParams {
            namespace: "project/dto".to_string(),
            trigger_window_event_ids: vec![invalid.to_string()],
            selected_evidence_event_ids: None,
            selection_basis: None,
        })
        .expect_err("invalid trigger-window references must fail closed");
        assert!(error.to_string().contains("InvalidEventReference"));
    }

    let oversized = GetEvidenceRelationInput::try_from(GetEvidenceRelationParams {
        namespace: "project/dto".to_string(),
        trigger_window_event_ids: (0..=MAX_EVIDENCE_MANIFEST_ITEMS)
            .map(|index| format!("evt-{index}"))
            .collect(),
        selected_evidence_event_ids: None,
        selection_basis: None,
    })
    .expect_err("oversized trigger windows must fail closed");
    assert!(oversized.to_string().contains("at most"));

    let blank_basis = GetEvidenceRelationInput::try_from(GetEvidenceRelationParams {
        namespace: "project/dto".to_string(),
        trigger_window_event_ids: vec!["evt-1".to_string()],
        selected_evidence_event_ids: Some(vec!["evt-1".to_string()]),
        selection_basis: Some("   ".to_string()),
    })
    .expect_err("blank selection_basis must fail closed");
    assert!(blank_basis.to_string().contains("selection_basis"));
}

#[test]
fn get_self_model_history_dto_requires_history_type_and_defaults_limit() {
    let input = GetSelfModelHistoryInput::try_from(GetSelfModelHistoryParams {
        namespace: "project/dto".to_string(),
        history_type: SelfModelHistoryTypeDto::Identity,
        limit: None,
    })
    .expect("identity history should parse");
    assert_eq!(input.history_kind, SelfModelHistoryKind::Identity);
    assert_eq!(input.limit, DEFAULT_SELF_MODEL_HISTORY_LIMIT);

    let commitment = GetSelfModelHistoryInput::try_from(GetSelfModelHistoryParams {
        namespace: "self".to_string(),
        history_type: SelfModelHistoryTypeDto::Commitment,
        limit: Some(7),
    })
    .expect("commitment history should parse");
    assert_eq!(commitment.history_kind, SelfModelHistoryKind::Commitment);
    assert_eq!(commitment.limit, 7);
}

#[test]
fn get_self_model_history_dto_rejects_invalid_scope_and_limits() {
    let invalid_namespace = GetSelfModelHistoryInput::try_from(GetSelfModelHistoryParams {
        namespace: "invalid".to_string(),
        history_type: SelfModelHistoryTypeDto::Identity,
        limit: None,
    })
    .expect_err("invalid namespace should fail closed");
    assert!(invalid_namespace.to_string().contains("InvalidNamespace"));

    for limit in [0, 101] {
        let error = GetSelfModelHistoryInput::try_from(GetSelfModelHistoryParams {
            namespace: "project/dto".to_string(),
            history_type: SelfModelHistoryTypeDto::Commitment,
            limit: Some(limit),
        })
        .expect_err("out-of-range self-model history limit should fail");
        assert!(error.to_string().contains("limit"));
    }
}

#[test]
fn supersede_memory_dto_parses_canonical_claim_and_event_references() {
    let input = SupersedeMemoryInput::try_from(SupersedeMemoryParams {
        request_id: None,
        namespace: "project/dto".to_string(),
        claim_reference: "claim:stored-claim".to_string(),
        replacement_claim: ClaimDraftDto {
            owner: OwnerDto::World,
            namespace: Some("project/dto".to_string()),
            subject: "project.role".to_string(),
            predicate: "is".to_string(),
            object: "corrected".to_string(),
            mode: ModeDto::Observed,
        },
        replacement_evidence_event_ids: vec![
            "event:evidence-1".to_string(),
            "evidence-1".to_string(),
        ],
        summary: "correct the stored claim".to_string(),
    })
    .expect("scoped supersede should parse");
    assert_eq!(input.claim_reference.canonical(), "claim:stored-claim");
    assert_eq!(
        input
            .evidence_event_ids
            .iter()
            .map(|reference| reference.canonical())
            .collect::<Vec<_>>(),
        vec!["event:evidence-1"]
    );
}

#[test]
fn supersede_memory_dto_rejects_invalid_scope_target_and_empty_evidence() {
    let invalid_namespace = SupersedeMemoryInput::try_from(SupersedeMemoryParams {
        request_id: None,
        namespace: "invalid".to_string(),
        claim_reference: "claim:stored-claim".to_string(),
        replacement_claim: ClaimDraftDto {
            owner: OwnerDto::World,
            namespace: Some("project/dto".to_string()),
            subject: "project.role".to_string(),
            predicate: "is".to_string(),
            object: "corrected".to_string(),
            mode: ModeDto::Observed,
        },
        replacement_evidence_event_ids: vec!["event:evidence-1".to_string()],
        summary: "correct the stored claim".to_string(),
    })
    .expect_err("invalid namespace should fail closed");
    assert!(invalid_namespace.to_string().contains("InvalidNamespace"));

    let empty_evidence = SupersedeMemoryInput::try_from(SupersedeMemoryParams {
        request_id: None,
        namespace: "project/dto".to_string(),
        claim_reference: "stored-claim".to_string(),
        replacement_claim: ClaimDraftDto {
            owner: OwnerDto::World,
            namespace: Some("project/dto".to_string()),
            subject: "project.role".to_string(),
            predicate: "is".to_string(),
            object: "corrected".to_string(),
            mode: ModeDto::Observed,
        },
        replacement_evidence_event_ids: vec![],
        summary: "correct the stored claim".to_string(),
    })
    .expect_err("empty evidence should fail closed");
    assert!(empty_evidence.to_string().contains("evidence"));

    let mismatched_scope = SupersedeMemoryInput::try_from(SupersedeMemoryParams {
        request_id: None,
        namespace: "project/dto".to_string(),
        claim_reference: "stored-claim".to_string(),
        replacement_claim: ClaimDraftDto {
            owner: OwnerDto::World,
            namespace: Some("project/other".to_string()),
            subject: "project.role".to_string(),
            predicate: "is".to_string(),
            object: "corrected".to_string(),
            mode: ModeDto::Observed,
        },
        replacement_evidence_event_ids: vec!["event:evidence-1".to_string()],
        summary: "correct the stored claim".to_string(),
    })
    .expect_err("replacement leaving the requested namespace should fail closed");
    assert!(mismatched_scope.to_string().contains("namespace"));
}

#[test]
fn get_reflection_history_dto_requires_claim_type_and_defaults_limit() {
    for claim_reference in ["stored-claim-id", "claim:stored-claim-id"] {
        let input = GetReflectionHistoryInput::try_from(GetReflectionHistoryParams {
            namespace: "project/dto".to_string(),
            claim_reference: claim_reference.to_string(),
            limit: None,
        })
        .expect("raw and canonical claim references should parse");

        assert_eq!(input.claim_reference.claim_id(), "stored-claim-id");
        assert_eq!(input.limit, DEFAULT_REFLECTION_HISTORY_LIMIT);
    }
}

#[test]
fn get_reflection_history_dto_rejects_invalid_limits() {
    for limit in [0, 101] {
        let error = GetReflectionHistoryInput::try_from(GetReflectionHistoryParams {
            namespace: "project/dto".to_string(),
            claim_reference: "claim:stored-claim-id".to_string(),
            limit: Some(limit),
        })
        .expect_err("out-of-range history limit should fail");

        assert!(error.to_string().contains("limit"));
    }
}

#[test]
fn evidence_query_dto_parses_recency_window_fields() {
    let query: EvidenceQuery = serde_json::from_value::<EvidenceQueryDto>(serde_json::json!({
        "recorded_after": "2026-03-23T10:00:00Z",
        "recorded_before": "2026-03-23T11:00:00Z",
        "limit": 3
    }))
    .expect("dto should parse")
    .try_into()
    .expect("query should convert");

    assert_eq!(
        query.recorded_after,
        Some(
            DateTime::parse_from_rfc3339("2026-03-23T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    assert_eq!(
        query.recorded_before,
        Some(
            DateTime::parse_from_rfc3339("2026-03-23T11:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    assert_eq!(query.limit, Some(3));
}

#[test]
fn evidence_query_dto_rejects_invalid_recency_timestamp() {
    let dto = serde_json::from_value::<EvidenceQueryDto>(serde_json::json!({
        "recorded_after": "not-a-date"
    }))
    .expect("dto deserialization should keep validation in conversion");

    let error = EvidenceQuery::try_from(dto).expect_err("invalid timestamp should fail");

    assert!(error.to_string().contains("recorded_after"));
}

#[test]
fn evidence_query_dto_parses_event_id_prefix() {
    let query: EvidenceQuery = serde_json::from_value::<EvidenceQueryDto>(serde_json::json!({
        "event_id_prefix": "alpha-"
    }))
    .expect("dto should parse")
    .try_into()
    .expect("query should convert");

    assert_eq!(query.event_id_prefix, Some("alpha-".to_string()));

    let empty = serde_json::from_value::<EvidenceQueryDto>(serde_json::json!({
        "event_id_prefix": "   "
    }))
    .expect("dto deserialization keeps validation in conversion");
    let error = EvidenceQuery::try_from(empty).expect_err("empty prefix should fail");
    assert!(error.to_string().contains("event_id_prefix"));
}

#[test]
fn evidence_query_dto_rejects_zero_limit() {
    let dto = serde_json::from_value::<EvidenceQueryDto>(serde_json::json!({
        "limit": 0
    }))
    .expect("dto deserialization keeps validation in conversion");

    let error = EvidenceQuery::try_from(dto).expect_err("zero limit should fail");

    assert!(error.to_string().contains("at least 1"));
}

#[test]
fn snapshot_dto_derives_owner_from_namespace_without_accepting_owner_input() {
    let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "namespace": "project/agent-llm-mm"
    }))
    .expect("snapshot params should parse");

    let input = BuildSelfSnapshotInput::try_from(params).expect("scope should convert");

    assert_eq!(input.scope.owner(), Some(Owner::World));
    assert_eq!(
        input.scope.namespace().map(|namespace| namespace.as_str()),
        Some("project/agent-llm-mm")
    );
}

#[test]
fn snapshot_dto_keeps_omitted_namespace_as_legacy_compatibility_scope() {
    let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4
    }))
    .expect("legacy snapshot params should parse");

    let input = BuildSelfSnapshotInput::try_from(params).expect("legacy scope should convert");

    assert!(input.scope.is_legacy_unscoped());
    assert!(input.time_window.is_unbounded());
}

#[test]
fn snapshot_dto_parses_and_normalizes_time_window() {
    let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "namespace": "project/agent-llm-mm",
        "recorded_after": "2026-07-11T10:00:00+08:00",
        "recorded_before": "2026-07-11T03:00:00Z"
    }))
    .expect("snapshot params should parse");

    let input = BuildSelfSnapshotInput::try_from(params).expect("time window should convert");

    assert_eq!(
        input.time_window.recorded_after,
        Some(
            DateTime::parse_from_rfc3339("2026-07-11T02:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    assert_eq!(
        input.time_window.recorded_before,
        Some(
            DateTime::parse_from_rfc3339("2026-07-11T03:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
}

#[test]
fn snapshot_dto_rejects_invalid_or_reversed_time_window() {
    let invalid_timestamp = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "namespace": "project/agent-llm-mm",
        "recorded_after": "not-a-date"
    }))
    .unwrap();
    let error = BuildSelfSnapshotInput::try_from(invalid_timestamp)
        .expect_err("invalid snapshot timestamp should fail");
    assert!(error.to_string().contains("recorded_after"));

    let reversed = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "namespace": "project/agent-llm-mm",
        "recorded_after": "2026-07-11T04:00:00Z",
        "recorded_before": "2026-07-11T03:00:00Z"
    }))
    .unwrap();
    let error = BuildSelfSnapshotInput::try_from(reversed)
        .expect_err("reversed snapshot time window should fail");
    assert!(error.to_string().contains("less than or equal"));
}

#[test]
fn snapshot_dto_requires_namespace_for_any_explicit_time_bound() {
    for params in [
        serde_json::json!({
            "budget": 4,
            "recorded_after": "2026-07-11T02:00:00Z"
        }),
        serde_json::json!({
            "budget": 4,
            "recorded_before": "2026-07-11T03:00:00Z"
        }),
    ] {
        let params = serde_json::from_value::<BuildSelfSnapshotParams>(params).unwrap();
        let error = BuildSelfSnapshotInput::try_from(params)
            .expect_err("time-bounded snapshots must not use legacy unscoped reads");
        assert!(error.to_string().contains("explicit namespace"));
    }
}

#[test]
fn snapshot_dto_rejects_non_empty_manifest_without_explicit_namespace() {
    let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "evidence_manifest": ["evt-1"]
    }))
    .expect("validation should stay in conversion");

    let error = BuildSelfSnapshotInput::try_from(params)
        .expect_err("manifest lookup must not use the legacy unscoped compatibility path");

    assert!(
        error
            .to_string()
            .contains("evidence_manifest requires an explicit namespace")
    );
}

#[test]
fn snapshot_dto_rejects_empty_manifest_without_explicit_namespace() {
    let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "evidence_manifest": []
    }))
    .expect("validation should stay in conversion");

    let error = BuildSelfSnapshotInput::try_from(params)
        .expect_err("an explicit empty manifest must still require a bounded scope");

    assert!(
        error
            .to_string()
            .contains("evidence_manifest requires an explicit namespace")
    );
}

#[test]
fn snapshot_dto_rejects_invalid_namespace() {
    let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "namespace": "tenant/not-supported"
    }))
    .expect("validation should stay in conversion");

    assert!(BuildSelfSnapshotInput::try_from(params).is_err());
}

#[test]
fn snapshot_dto_canonicalizes_and_deduplicates_evidence_manifest() {
    let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
        "budget": 4,
        "namespace": "project/agent-llm-mm",
        "evidence_manifest": ["evt-1", "event:evt-1", "event:evt-2"]
    }))
    .expect("snapshot params should parse");

    let input = BuildSelfSnapshotInput::try_from(params).expect("manifest should convert");
    let manifest = input
        .evidence_manifest
        .expect("manifest should remain explicit");

    assert_eq!(
        manifest
            .iter()
            .map(|reference| reference.canonical())
            .collect::<Vec<_>>(),
        vec!["event:evt-1", "event:evt-2"]
    );
}

#[test]
fn snapshot_dto_rejects_invalid_evidence_manifest_entry() {
    for invalid in ["", "event:", "event:event:evt-1", "evt 1"] {
        let params = serde_json::from_value::<BuildSelfSnapshotParams>(serde_json::json!({
            "budget": 4,
            "namespace": "project/agent-llm-mm",
            "evidence_manifest": [invalid]
        }))
        .expect("validation should stay in conversion");

        assert!(BuildSelfSnapshotInput::try_from(params).is_err());
    }
}

#[test]
fn snapshot_dto_bounds_evidence_manifest_before_query_construction() {
    let at_limit = (0..MAX_EVIDENCE_MANIFEST_ITEMS)
        .map(|index| format!("evt-{index}"))
        .collect::<Vec<_>>();
    let input = BuildSelfSnapshotInput::try_from(BuildSelfSnapshotParams {
        budget: 4,
        namespace: Some("project/agent-llm-mm".to_string()),
        evidence_manifest: Some(at_limit),
        recorded_after: None,
        recorded_before: None,
        auto_reflect_namespace: None,
    })
    .expect("manifest at the documented limit should convert");
    assert_eq!(
        input
            .evidence_manifest
            .as_deref()
            .map(|manifest| manifest.len()),
        Some(MAX_EVIDENCE_MANIFEST_ITEMS)
    );

    let oversized = (0..=MAX_EVIDENCE_MANIFEST_ITEMS)
        .map(|index| format!("evt-{index}"))
        .collect::<Vec<_>>();
    let error = BuildSelfSnapshotInput::try_from(BuildSelfSnapshotParams {
        budget: 4,
        namespace: Some("project/agent-llm-mm".to_string()),
        evidence_manifest: Some(oversized),
        recorded_after: None,
        recorded_before: None,
        auto_reflect_namespace: None,
    })
    .expect_err("oversized manifests must fail before SQLite bind construction");
    assert!(
        error
            .to_string()
            .contains("evidence_manifest must contain at most 256 entries")
    );
}

#[test]
fn event_and_claim_dtos_reject_unknown_owner_for_new_writes() {
    let event = Event::try_from(EventDto {
        feedback: None,
        owner: OwnerDto::Unknown,
        namespace: Some("world".to_string()),
        kind: EventKindDto::Observation,
        summary: "legacy-looking write".to_string(),
    });
    assert!(event.is_err());

    let claim = ClaimDraft::try_from(ClaimDraftDto {
        owner: OwnerDto::Unknown,
        namespace: Some("project/demo".to_string()),
        subject: "project.fact".to_string(),
        predicate: "is".to_string(),
        object: "legacy".to_string(),
        mode: ModeDto::Observed,
    });
    assert!(claim.is_err());

    let accepted = Event::try_from(EventDto {
        feedback: None,
        owner: OwnerDto::World,
        namespace: Some("project/demo".to_string()),
        kind: EventKindDto::Observation,
        summary: "canonical write".to_string(),
    })
    .expect("canonical world/project writes remain accepted");
    assert_eq!(accepted.owner(), Owner::World);
}
