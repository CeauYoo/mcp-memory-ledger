pub(super) const OWNER_NAMESPACE_SCOPE_CONSTRAINT_NAME: &str = "owner_namespace_scope";
pub(super) const CURRENT_SCHEMA_VERSION: i64 = 4;

pub(super) const SCHEMA_MIGRATIONS: [(i64, &str); 4] = [
    (1, "baseline_schema"),
    (2, "owner_namespace_scope"),
    (3, "reflection_audit_columns"),
    (4, "event_feedback_metadata"),
];

const OWNER_NAMESPACE_SCOPE_CONSTRAINT_SQL: &str = r#"    CONSTRAINT owner_namespace_scope CHECK (
        (owner = 'self' AND namespace = 'self')
        OR (owner = 'user' AND namespace LIKE 'user/%')
        OR (owner = 'world' AND (namespace = 'world' OR namespace LIKE 'project/%'))
        OR (owner = 'unknown' AND (namespace = 'world' OR namespace LIKE 'project/%'))
    )"#;

const EVIDENCE_LINKS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS evidence_links (
    claim_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    PRIMARY KEY (claim_id, event_id),
    FOREIGN KEY (claim_id) REFERENCES claims(claim_id),
    FOREIGN KEY (event_id) REFERENCES events(event_id)
)"#;

const EPISODE_EVENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS episode_events (
    episode_reference TEXT NOT NULL,
    event_id TEXT NOT NULL,
    PRIMARY KEY (episode_reference, event_id),
    FOREIGN KEY (event_id) REFERENCES events(event_id)
)"#;

const REFLECTIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS reflections (
    reflection_id TEXT PRIMARY KEY,
    recorded_at TEXT NOT NULL,
    summary TEXT NOT NULL,
    superseded_claim_id TEXT,
    replacement_claim_id TEXT,
    supporting_evidence_event_ids TEXT NOT NULL DEFAULT '[]',
    requested_identity_update TEXT,
    requested_commitment_updates TEXT
)"#;

const REFLECTION_TRIGGER_LEDGER_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS reflection_trigger_ledger (
    ledger_id TEXT PRIMARY KEY,
    trigger_type TEXT NOT NULL,
    namespace TEXT NOT NULL,
    trigger_key TEXT NOT NULL,
    status TEXT NOT NULL,
    evidence_window TEXT NOT NULL DEFAULT '[]',
    handled_at TEXT,
    cooldown_until TEXT,
    episode_watermark INTEGER,
    reflection_id TEXT
)"#;

const IDENTITY_CLAIMS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS identity_claims (
    position INTEGER NOT NULL PRIMARY KEY,
    claim TEXT NOT NULL
)"#;

const COMMITMENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS commitments (
    description TEXT PRIMARY KEY,
    owner TEXT NOT NULL
)"#;

const OPERATION_LOG_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS operation_log (
    operation_id TEXT PRIMARY KEY,
    occurred_at TEXT NOT NULL,
    namespace TEXT,
    actor_kind TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    entrypoint TEXT NOT NULL,
    operation_kind TEXT NOT NULL,
    status TEXT NOT NULL,
    correlation_id TEXT,
    request_summary_json TEXT,
    response_summary_json TEXT,
    diagnostic_summary_json TEXT,
    redaction_version INTEGER NOT NULL DEFAULT 1
)"#;

const SCHEMA_MIGRATIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
)"#;

const LEGACY_NAMESPACE_BACKFILL_WITH_NAMESPACE_SQL: &str = "COALESCE(NULLIF(namespace, ''), CASE owner WHEN 'self' THEN 'self' WHEN 'user' THEN 'user/default' ELSE 'world' END)";
const LEGACY_NAMESPACE_BACKFILL_WITHOUT_NAMESPACE_SQL: &str =
    "CASE owner WHEN 'self' THEN 'self' WHEN 'user' THEN 'user/default' ELSE 'world' END";

pub(super) fn init_sql() -> String {
    format!(
        r#"
{events_table};

{claims_table};

{evidence_links_table};

{episode_events_table};

{reflections_table};

{reflection_trigger_ledger_table};

{identity_claims_table};

{commitments_table};

{operation_log_table};

{schema_migrations_table};
"#,
        events_table = events_table_sql(true),
        claims_table = claims_table_sql(true),
        evidence_links_table = EVIDENCE_LINKS_TABLE_SQL,
        episode_events_table = EPISODE_EVENTS_TABLE_SQL,
        reflections_table = REFLECTIONS_TABLE_SQL,
        reflection_trigger_ledger_table = REFLECTION_TRIGGER_LEDGER_TABLE_SQL,
        identity_claims_table = IDENTITY_CLAIMS_TABLE_SQL,
        commitments_table = COMMITMENTS_TABLE_SQL,
        operation_log_table = OPERATION_LOG_TABLE_SQL,
        schema_migrations_table = SCHEMA_MIGRATIONS_TABLE_SQL,
    )
}

pub(super) fn events_table_sql(include_if_not_exists: bool) -> String {
    let if_not_exists_clause = if include_if_not_exists {
        " IF NOT EXISTS"
    } else {
        ""
    };

    format!(
        r#"
CREATE TABLE{if_not_exists_clause} events (
    event_id TEXT PRIMARY KEY,
    recorded_at TEXT NOT NULL,
    owner TEXT NOT NULL,
    namespace TEXT NOT NULL,
    kind TEXT NOT NULL,
    summary TEXT NOT NULL,
    feedback_json TEXT,
{owner_namespace_scope_constraint}
)"#,
        owner_namespace_scope_constraint = OWNER_NAMESPACE_SCOPE_CONSTRAINT_SQL,
    )
}

pub(super) fn claims_table_sql(include_if_not_exists: bool) -> String {
    let if_not_exists_clause = if include_if_not_exists {
        " IF NOT EXISTS"
    } else {
        ""
    };

    format!(
        r#"
CREATE TABLE{if_not_exists_clause} claims (
    claim_id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    namespace TEXT NOT NULL,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    mode TEXT NOT NULL,
    status TEXT NOT NULL,
{owner_namespace_scope_constraint}
)"#,
        owner_namespace_scope_constraint = OWNER_NAMESPACE_SCOPE_CONSTRAINT_SQL,
    )
}

pub(super) fn legacy_namespace_backfill_expression(
    legacy_table_has_namespace: bool,
) -> &'static str {
    if legacy_table_has_namespace {
        LEGACY_NAMESPACE_BACKFILL_WITH_NAMESPACE_SQL
    } else {
        LEGACY_NAMESPACE_BACKFILL_WITHOUT_NAMESPACE_SQL
    }
}
