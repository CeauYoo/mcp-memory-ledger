use super::*;
use agent_llm_mm::{adapters::sqlite::SqliteStore, application::experience, domain::experience::*};

impl Fixture {
    async fn histories(&self, suffix: &str, namespace: &str, event: &str, claim: &str) {
        let store = SqliteStore::bootstrap(&agent_llm_mm::support::sqlite_url::sqlite_file_url(
            &self.db,
        ))
        .await
        .unwrap();
        let ep = format!("ep{suffix}");
        let candidate = format!("xp{suffix}");
        experience::create_episode(
            &store,
            CreateEpisodeRequest {
                namespace: namespace.into(),
                request_id: format!("episode{suffix}"),
                episode_id: ep.clone(),
                content: EpisodeContent {
                    title: "Test episode".into(),
                    objective: "Observe outcome".into(),
                    actions: vec!["Run test".into()],
                    observations: vec!["Passed".into()],
                    outcome: "Pass".into(),
                    lesson: "Check first".into(),
                    limitations: vec![],
                    source_event_refs: vec![format!("event:{event}")],
                },
            },
        )
        .await
        .unwrap();
        let content = ExperienceContent {
            kind: ExperienceKind::Semantic,
            title: "Lesson".into(),
            statement: "Check the result".into(),
            steps: vec![],
            limitations: vec![],
            source_episode_ids: vec![ep],
        };
        experience::create_candidate(
            &store,
            CreateCandidateRequest {
                namespace: namespace.into(),
                request_id: format!("candidate{suffix}"),
                candidate_id: candidate.clone(),
                content: content.clone(),
            },
        )
        .await
        .unwrap();
        experience::update_candidate_status(
            &store,
            UpdateCandidateStatusRequest {
                namespace: namespace.into(),
                request_id: format!("activate{suffix}"),
                candidate_id: candidate.clone(),
                expected_version: 1,
                status: ExperienceStatus::Active,
            },
        )
        .await
        .unwrap();
        experience::revise_candidate(
            &store,
            ReviseCandidateRequest {
                namespace: namespace.into(),
                request_id: format!("revise{suffix}"),
                candidate_id: candidate.clone(),
                expected_version: 2,
                content,
            },
        )
        .await
        .unwrap();
        experience::rollback_candidate(
            &store,
            RollbackCandidateRequest {
                namespace: namespace.into(),
                request_id: format!("rollback{suffix}"),
                candidate_id: candidate,
                expected_version: 3,
                target_version: 1,
            },
        )
        .await
        .unwrap();
        drop(store);
        self.sql(&format!(r#"INSERT INTO reflections(reflection_id,recorded_at,summary,superseded_claim_id,supporting_evidence_event_ids,scope_status,evidence_normalized) VALUES('r{suffix}','2026-10-01T00:00:00Z','retained reflection','{claim}','["{event}"]','verified',1);
INSERT INTO reflection_scopes VALUES('r{suffix}','origin','world','{namespace}'),('r{suffix}','affected','world','{namespace}');
INSERT INTO reflection_evidence VALUES('r{suffix}','{event}',0);
INSERT INTO reflection_trigger_ledger(ledger_id,trigger_type,namespace,trigger_key,status,evidence_window,reflection_id) VALUES('t{suffix}','conflict','{namespace}','{namespace}:conflict','handled','["{event}"]','r{suffix}');"#)).await;
        let payload = serde_json::json!({"candidate_id":format!("fc{suffix}"),"proposal":{"namespace":namespace,"target_claim_reference":format!("claim:{claim}"),"expected_target_version":"old-fingerprint","replacement_claim":{"owner":"World","namespace":namespace,"subject":"s","predicate":"p","object":"new","mode":"Said"},"evidence_event_ids":[format!("event:{event}")],"summary":"historical proposal"},"state":"proposed","validation":{"passed":false,"reasons":[],"limitation":"structure only"},"created_at":"2026-10-01T00:00:00Z","updated_at":"2026-10-01T00:00:00Z","revision":0,"rejection_reason":null,"reflection_id":null,"replacement_claim_id":null});
        let mut conn = self.connection().await;
        sqlx::query(
            "INSERT INTO feedback_candidates VALUES(?,?,?,'old-fingerprint','proposed',0,?)",
        )
        .bind(format!("fc{suffix}"))
        .bind(namespace)
        .bind(claim)
        .bind(payload.to_string())
        .execute(&mut conn)
        .await
        .unwrap();
        conn.close().await.unwrap();
        // Freeze WAL into the main file before byte-level assertions.
        self.sql("PRAGMA wal_checkpoint(TRUNCATE)").await;
    }
    async fn foreign(&self) {
        self.sql("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('foreign-e','2026-10-01T00:00:00Z','world','project/foreign','conversation','unrelated'); INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES('foreign-c','world','project/foreign','s','p','o','said','active')").await;
        self.histories("foreign", "project/foreign", "foreign-e", "foreign-c")
            .await;
    }
}
#[tokio::test]
async fn complete_complex_history_and_unrelated_project_are_supported_and_unchanged() {
    let f = Fixture::new().await;
    f.histories("", "project/legacy", "e", "c").await;
    f.foreign().await;
    let bytes = fs::read(&f.db).unwrap();
    let p = f.preview().await;
    assert!(p.inventory.supported, "{:?}", p.inventory.blocked_reasons);
    assert!(
        p.inventory
            .support_contract
            .starts_with("scoped-history-v2")
    );
    assert_eq!(fs::read(&f.db).unwrap(), bytes);
    #[cfg(any(unix, windows))]
    {
        let result = adopt_workspace(&f.registry, &f.root, "project/legacy", &f.db)
            .await
            .unwrap();
        assert!(result.inventory.supported);
        assert_eq!(fs::read(&f.db).unwrap(), bytes);
    }
}
#[tokio::test]
async fn every_history_relation_checks_incoming_outgoing_and_missing_sources() {
    for sql in [
        "UPDATE reflection_scopes SET namespace='project/foreign' WHERE reflection_id='r' AND role='origin'",
        "UPDATE reflections SET superseded_claim_id='foreign-c' WHERE reflection_id='r'",
        "UPDATE reflections SET replacement_claim_id='missing' WHERE reflection_id='r'",
        "UPDATE reflections SET supporting_evidence_event_ids='[\"foreign-e\"]' WHERE reflection_id='r'",
        "UPDATE reflections SET supporting_evidence_event_ids='[\"e\"]' WHERE reflection_id='rforeign'",
        "INSERT INTO reflection_evidence VALUES('rforeign','e',1)",
        "DELETE FROM reflection_evidence WHERE reflection_id='r'",
        "UPDATE reflection_evidence SET position=7 WHERE reflection_id='r'",
        "INSERT INTO reflection_scopes VALUES('missing-reflection','origin','world','project/legacy')",
        "UPDATE experience_episodes SET payload_json=json_set(payload_json,'$.content.source_event_refs',json('[\"event:e\"]')) WHERE episode_id='epforeign'",
        "INSERT INTO experience_episode_sources VALUES('epforeign','e')",
        "DELETE FROM experience_episode_sources WHERE episode_id='ep'",
        "UPDATE experience_episodes SET payload_json=json_set(payload_json,'$.namespace','project/foreign') WHERE episode_id='ep'",
        "UPDATE experience_candidate_versions SET payload_json=json_set(payload_json,'$.content.source_episode_ids',json('[\"ep\"]')) WHERE candidate_id='xpforeign' AND version=1",
        "INSERT INTO experience_candidate_sources VALUES('xpforeign',1,'ep')",
        "DELETE FROM experience_candidate_sources WHERE candidate_id='xp' AND version=1",
        "UPDATE experience_candidate_versions SET payload_json=json_set(payload_json,'$.content.source_episode_ids',json('[\"missing\"]')) WHERE candidate_id='xp' AND version=1",
        "UPDATE experience_candidate_versions SET payload_json=json_set(payload_json,'$.previous_version',99) WHERE candidate_id='xp' AND version=2",
        "UPDATE experience_candidates SET current_version=3 WHERE candidate_id='xp'",
        "DELETE FROM experience_candidate_versions WHERE candidate_id='xp' AND version=2",
        "UPDATE experience_candidate_versions SET payload_json=json_set(payload_json,'$.content.statement','forged rollback') WHERE candidate_id='xp' AND version=4",
        "UPDATE feedback_candidates SET state='committed',revision=2,candidate_json=json_set(candidate_json,'$.state','committed','$.revision',2,'$.validation.passed',json('true')) WHERE candidate_id='fc'",
        "UPDATE feedback_candidates SET candidate_json=json_set(candidate_json,'$.validation.passed',json('true')) WHERE candidate_id='fc'",
        "UPDATE reflection_trigger_ledger SET trigger_key='project/foreign:conflict' WHERE ledger_id='t'",
        "UPDATE experience_candidate_versions SET payload_json=json_set(payload_json,'$.rollback_target_version',99) WHERE candidate_id='xp' AND version=4",
        "UPDATE feedback_candidates SET target_claim_id='c' WHERE candidate_id='fcforeign'",
        "UPDATE feedback_candidates SET candidate_json=json_set(candidate_json,'$.proposal.evidence_event_ids',json('[\"event:e\"]')) WHERE candidate_id='fcforeign'",
        "UPDATE feedback_candidates SET candidate_json=json_set(candidate_json,'$.reflection_id','r') WHERE candidate_id='fcforeign'",
        "UPDATE feedback_candidates SET candidate_json=json_set(candidate_json,'$.replacement_claim_id','c') WHERE candidate_id='fcforeign'",
        "UPDATE feedback_candidates SET candidate_json=json_set(candidate_json,'$.proposal.evidence_event_ids',json('[\"event:missing\"]')) WHERE candidate_id='fc'",
        "UPDATE reflection_trigger_ledger SET evidence_window='[\"e\"]' WHERE ledger_id='tforeign'",
        "UPDATE reflection_trigger_ledger SET reflection_id='r' WHERE ledger_id='tforeign'",
        "UPDATE reflection_trigger_ledger SET reflection_id='missing' WHERE ledger_id='t'",
        "UPDATE reflection_trigger_ledger SET evidence_window='[\"missing\"]' WHERE ledger_id='t'",
        "UPDATE events SET owner='unknown' WHERE event_id='e'",
    ] {
        let f = Fixture::new().await;
        f.histories("", "project/legacy", "e", "c").await;
        f.foreign().await;
        f.sql(&format!("PRAGMA foreign_keys=OFF; {sql}")).await;
        let p = f.preview().await;
        assert!(!p.inventory.supported, "accepted {sql}");
        assert!(!f.registry.exists());
    }
}
#[tokio::test]
async fn unknown_or_unparseable_history_never_gains_attribution() {
    for sql in [
        "UPDATE reflections SET scope_status='unknown' WHERE reflection_id='rforeign'",
        "UPDATE reflections SET reflection_id=NULL WHERE reflection_id='rforeign'",
        "UPDATE reflection_trigger_ledger SET evidence_window=x'7B7D' WHERE ledger_id='tforeign'",
        "UPDATE reflections SET evidence_normalized=0 WHERE reflection_id='r'",
        "UPDATE experience_episodes SET payload_json='{}' WHERE episode_id='epforeign'",
        "UPDATE experience_candidate_versions SET payload_json=json_set(payload_json,'$.future_provenance',json('[\"e\"]')) WHERE candidate_id='xpforeign' AND version=1",
        "UPDATE feedback_candidates SET candidate_json='{}' WHERE candidate_id='fcforeign'",
        "UPDATE reflection_trigger_ledger SET evidence_window='{}' WHERE ledger_id='tforeign'",
        "UPDATE reflections SET supporting_evidence_event_ids='{}' WHERE reflection_id='rforeign'",
    ] {
        let f = Fixture::new().await;
        f.histories("", "project/legacy", "e", "c").await;
        f.foreign().await;
        f.sql(&format!("PRAGMA foreign_keys=OFF; {sql}")).await;
        assert!(!f.preview().await.inventory.supported, "accepted {sql}");
    }
}
#[cfg(any(unix, windows))]
#[tokio::test]
async fn execution_rechecks_complex_history_added_after_preview() {
    let f = Fixture::new().await;
    f.histories("", "project/legacy", "e", "c").await;
    f.foreign().await;
    assert!(f.preview().await.inventory.supported);
    f.sql("INSERT INTO experience_candidate_sources VALUES('xpforeign',1,'ep')")
        .await;
    let bytes = fs::read(&f.db).unwrap();
    assert!(
        adopt_workspace(&f.registry, &f.root, "project/legacy", &f.db)
            .await
            .is_err()
    );
    assert!(!f.registry.exists());
    assert_eq!(fs::read(&f.db).unwrap(), bytes);
}

#[tokio::test]
async fn unattributable_global_self_model_versions_remain_explicitly_unsupported() {
    let f = Fixture::new().await;
    f.histories("", "project/legacy", "e", "c").await;
    f.sql("INSERT INTO self_model_versions(version,previous_version,kind,reflection_id,recorded_at,effective_at,identity_json,commitments_json,identity_written,commitments_written,identity_source_version,commitment_source_version,rollback_target_version) SELECT 1,0,'update','r','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z',identity_json,commitments_json,1,0,1,0,NULL FROM self_model_versions WHERE version=0").await;
    let p = f.preview().await;
    assert!(!p.inventory.supported);
    assert!(
        p.inventory
            .blocked_reasons
            .iter()
            .any(|r| r.contains("unsupported self-model history"))
    );
}

#[tokio::test]
async fn oversized_identifiers_fail_closed_but_unrelated_opaque_text_is_not_loaded() {
    let f = Fixture::new().await;
    f.histories("", "project/legacy", "e", "c").await;
    f.sql("UPDATE reflections SET summary=hex(zeroblob(9000000)) WHERE reflection_id='r'")
        .await;
    assert!(f.preview().await.inventory.supported);
    f.sql("UPDATE reflection_trigger_ledger SET trigger_key=hex(zeroblob(9000000)) WHERE ledger_id='t'").await;
    let p = f.preview().await;
    assert!(!p.inventory.supported);
    assert!(
        p.inventory
            .blocked_reasons
            .iter()
            .any(|r| r.contains("bounded inventory limit"))
    );
}

#[tokio::test]
async fn custom_database_programs_are_not_implicitly_adopted() {
    for sql in [
        "CREATE VIEW custom_provenance AS SELECT event_id FROM events",
        "CREATE TRIGGER custom_history AFTER INSERT ON events BEGIN SELECT 1; END",
    ] {
        let f = Fixture::new().await;
        f.sql(sql).await;
        let p = f.preview().await;
        assert!(!p.inventory.supported);
        assert!(
            p.inventory
                .blocked_reasons
                .iter()
                .any(|r| r.contains("unsupported view or noncanonical trigger"))
        );
    }
}
#[tokio::test]
async fn legacy_unambiguous_replacement_only_effects_are_preserved() {
    let f = Fixture::new().await;
    f.histories("", "project/legacy", "e", "c").await;
    f.sql("UPDATE reflections SET scope_status='legacy_unambiguous',replacement_claim_id=superseded_claim_id,superseded_claim_id=NULL WHERE reflection_id='r'").await;
    let p = f.preview().await;
    assert!(p.inventory.supported, "{:?}", p.inventory.blocked_reasons);
}

#[tokio::test]
async fn blob_feedback_is_rejected_without_decoding_or_panicking() {
    let f = Fixture::new().await;
    f.sql("UPDATE events SET feedback_json=x'7B7D' WHERE event_id='e'")
        .await;
    let p = f.preview().await;
    assert!(!p.inventory.supported);
    assert!(
        p.inventory
            .blocked_reasons
            .iter()
            .any(|r| r.contains("feedback provenance storage type"))
    );
}

#[tokio::test]
async fn committed_feedback_must_match_its_exact_reflection_not_just_namespace() {
    let f = Fixture::new().await;
    f.histories("", "project/legacy", "e", "c").await;
    f.sql("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES('c2','world','project/legacy','s','p','new','said','active'); UPDATE reflections SET replacement_claim_id='c2' WHERE reflection_id='r'; UPDATE feedback_candidates SET state='committed',revision=2,candidate_json=json_set(candidate_json,'$.state','committed','$.revision',2,'$.validation.passed',json('true'),'$.reflection_id','r','$.replacement_claim_id','c2') WHERE candidate_id='fc'").await;
    let p = f.preview().await;
    assert!(p.inventory.supported, "{:?}", p.inventory.blocked_reasons);
    f.sql("UPDATE feedback_candidates SET target_claim_id='c2',candidate_json=json_set(candidate_json,'$.proposal.target_claim_reference','claim:c2') WHERE candidate_id='fc'").await;
    let p = f.preview().await;
    assert!(!p.inventory.supported);
    assert!(
        p.inventory
            .blocked_reasons
            .iter()
            .any(|r| r.contains("feedback commit references differ"))
    );
}
