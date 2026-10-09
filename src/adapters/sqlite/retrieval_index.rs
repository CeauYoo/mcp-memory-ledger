//! Rebuildable local search projection. The event/claim ledger remains authoritative.
//! Installation/rebuild is explicit; inspection and recall never repair on read.
use super::SqliteStore;
use crate::error::AppError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection};
use tokio_stream::StreamExt;

pub const RETRIEVAL_INDEX_VERSION: i64 = 1;

// Each statement is executed whole. In particular, trigger bodies must NOT be split on ';'.
pub(super) const RETRIEVAL_INDEX_DDL: &[(&str, &str)] = &[
    (
        "text_recall_documents",
        "CREATE TABLE text_recall_documents (doc_id INTEGER PRIMARY KEY, record_type TEXT NOT NULL CHECK (record_type IN ('event', 'claim')), record_id TEXT NOT NULL, owner TEXT NOT NULL, namespace TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '', subject TEXT NOT NULL DEFAULT '', predicate TEXT NOT NULL DEFAULT '', object TEXT NOT NULL DEFAULT '', requires_literal_fallback INTEGER NOT NULL DEFAULT 0 CHECK (requires_literal_fallback IN (0,1)), UNIQUE (record_type, record_id))",
    ),
    (
        "text_recall_fts",
        "CREATE VIRTUAL TABLE text_recall_fts USING fts5(summary, subject, predicate, object, content='text_recall_documents', content_rowid='doc_id', tokenize='trigram case_sensitive 1')",
    ),
    (
        "text_recall_vocab",
        "CREATE VIRTUAL TABLE text_recall_vocab USING fts5vocab(text_recall_fts, 'instance')",
    ),
    (
        "idx_text_recall_scope",
        "CREATE INDEX idx_text_recall_scope ON text_recall_documents(owner, namespace, record_type, doc_id)",
    ),
    (
        "idx_text_recall_literal_fallback",
        "CREATE INDEX idx_text_recall_literal_fallback ON text_recall_documents(owner, namespace, record_type, record_id) WHERE requires_literal_fallback=1",
    ),
    (
        "idx_recall_events_scope",
        "CREATE INDEX idx_recall_events_scope ON events(owner, namespace, event_id)",
    ),
    (
        "idx_recall_claims_scope_status",
        "CREATE INDEX idx_recall_claims_scope_status ON claims(owner, namespace, status, claim_id)",
    ),
    (
        "idx_recall_evidence_event",
        "CREATE INDEX idx_recall_evidence_event ON evidence_links(event_id, claim_id)",
    ),
    (
        "idx_recall_episode_event",
        "CREATE INDEX idx_recall_episode_event ON episode_events(event_id, episode_reference)",
    ),
    (
        "text_recall_documents_ai",
        "CREATE TRIGGER text_recall_documents_ai AFTER INSERT ON text_recall_documents BEGIN INSERT INTO text_recall_fts(rowid, summary, subject, predicate, object) VALUES (new.doc_id, new.summary, new.subject, new.predicate, new.object); END",
    ),
    (
        "text_recall_documents_ad",
        "CREATE TRIGGER text_recall_documents_ad AFTER DELETE ON text_recall_documents BEGIN INSERT INTO text_recall_fts(text_recall_fts, rowid, summary, subject, predicate, object) VALUES ('delete', old.doc_id, old.summary, old.subject, old.predicate, old.object); END",
    ),
    (
        "text_recall_documents_au",
        "CREATE TRIGGER text_recall_documents_au AFTER UPDATE ON text_recall_documents BEGIN INSERT INTO text_recall_fts(text_recall_fts, rowid, summary, subject, predicate, object) VALUES ('delete', old.doc_id, old.summary, old.subject, old.predicate, old.object); INSERT INTO text_recall_fts(rowid, summary, subject, predicate, object) VALUES (new.doc_id, new.summary, new.subject, new.predicate, new.object); END",
    ),
    (
        "text_recall_events_ai",
        "CREATE TRIGGER text_recall_events_ai AFTER INSERT ON events BEGIN INSERT INTO text_recall_documents(record_type, record_id, owner, namespace, summary, requires_literal_fallback) VALUES ('event', new.event_id, new.owner, new.namespace, substr(lower(new.summary),1), instr(new.summary,char(0))>0); END",
    ),
    (
        "text_recall_events_ad",
        "CREATE TRIGGER text_recall_events_ad AFTER DELETE ON events BEGIN DELETE FROM text_recall_documents WHERE record_type='event' AND record_id=old.event_id; END",
    ),
    (
        "text_recall_events_au",
        "CREATE TRIGGER text_recall_events_au AFTER UPDATE ON events BEGIN DELETE FROM text_recall_documents WHERE record_type='event' AND record_id=old.event_id; INSERT INTO text_recall_documents(record_type, record_id, owner, namespace, summary, requires_literal_fallback) VALUES ('event', new.event_id, new.owner, new.namespace, substr(lower(new.summary),1), instr(new.summary,char(0))>0); END",
    ),
    (
        "text_recall_claims_ai",
        "CREATE TRIGGER text_recall_claims_ai AFTER INSERT ON claims WHEN new.status='active' BEGIN INSERT INTO text_recall_documents(record_type, record_id, owner, namespace, subject, predicate, object, requires_literal_fallback) VALUES ('claim', new.claim_id, new.owner, new.namespace, substr(lower(new.subject),1), substr(lower(new.predicate),1), substr(lower(new.object),1), (instr(new.subject,char(0))>0 OR instr(new.predicate,char(0))>0 OR instr(new.object,char(0))>0)); END",
    ),
    (
        "text_recall_claims_ad",
        "CREATE TRIGGER text_recall_claims_ad AFTER DELETE ON claims BEGIN DELETE FROM text_recall_documents WHERE record_type='claim' AND record_id=old.claim_id; END",
    ),
    (
        "text_recall_claims_au",
        "CREATE TRIGGER text_recall_claims_au AFTER UPDATE ON claims BEGIN DELETE FROM text_recall_documents WHERE record_type='claim' AND record_id=old.claim_id; INSERT INTO text_recall_documents(record_type, record_id, owner, namespace, subject, predicate, object, requires_literal_fallback) SELECT 'claim', new.claim_id, new.owner, new.namespace, substr(lower(new.subject),1), substr(lower(new.predicate),1), substr(lower(new.object),1), (instr(new.subject,char(0))>0 OR instr(new.predicate,char(0))>0 OR instr(new.object,char(0))>0) WHERE new.status='active'; END",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetrievalIndexReport {
    pub index_version: i64,
    pub status: String,
    pub derived_documents: Option<i64>,
    pub ledger_documents: Option<i64>,
    pub content_consistent: bool,
    pub postings_consistent: bool,
    pub issues: Vec<String>,
    pub rebuild_performed: bool,
}

impl RetrievalIndexReport {
    pub fn is_usable(&self) -> bool {
        self.status == "ready"
    }
}

fn error(error: sqlx::Error) -> AppError {
    AppError::Message(format!("SQLite retrieval index failed: {error}"))
}

/// Caller owns the schema migration transaction. Never called from a read path.
pub(super) async fn install_retrieval_index(
    connection: &mut SqliteConnection,
) -> Result<(), AppError> {
    for (_, ddl) in RETRIEVAL_INDEX_DDL {
        sqlx::query(ddl)
            .execute(&mut *connection)
            .await
            .map_err(error)?;
    }
    sqlx::query("INSERT INTO text_recall_documents(record_type, record_id, owner, namespace, summary, requires_literal_fallback) SELECT 'event', event_id, owner, namespace, substr(lower(summary),1), instr(summary,char(0))>0 FROM events")
        .execute(&mut *connection).await.map_err(error)?;
    sqlx::query("INSERT INTO text_recall_documents(record_type, record_id, owner, namespace, subject, predicate, object, requires_literal_fallback) SELECT 'claim', claim_id, owner, namespace, substr(lower(subject),1), substr(lower(predicate),1), substr(lower(object),1), (instr(subject,char(0))>0 OR instr(predicate,char(0))>0 OR instr(object,char(0))>0) FROM claims WHERE status='active'")
        .execute(&mut *connection).await.map_err(error)?;
    Ok(())
}

/// Cheap structural gate for recall. This is not a full posting/content integrity check.
pub(super) async fn retrieval_structure_issues(
    connection: &mut SqliteConnection,
) -> Result<Vec<String>, AppError> {
    let rows = sqlx::query("SELECT name, sql FROM sqlite_master WHERE name LIKE 'text_recall_%' OR name LIKE 'idx_recall_%' OR name LIKE 'idx_text_recall_%'")
        .fetch_all(&mut *connection).await.map_err(error)?;
    let mut issues = Vec::new();
    for (name, ddl) in RETRIEVAL_INDEX_DDL {
        match rows
            .iter()
            .find(|row| row.get::<String, _>("name") == *name)
        {
            None => issues.push(format!("missing:{name}")),
            Some(row)
                if row
                    .get::<Option<String>, _>("sql")
                    .as_deref()
                    .map(str::trim)
                    != Some(ddl.trim()) =>
            {
                issues.push(format!("definition_mismatch:{name}"))
            }
            _ => {}
        }
    }
    Ok(issues)
}

/// Read-only, full derived-content and trigram-posting inspection. No FTS INSERT commands.
pub(super) async fn inspect_retrieval_index_connection(
    connection: &mut SqliteConnection,
) -> Result<RetrievalIndexReport, AppError> {
    let issues = retrieval_structure_issues(connection).await?;
    let mut report = RetrievalIndexReport {
        index_version: RETRIEVAL_INDEX_VERSION,
        status: "needs_rebuild".into(),
        derived_documents: None,
        ledger_documents: None,
        content_consistent: false,
        postings_consistent: false,
        issues,
        rebuild_performed: false,
    };
    if !report.issues.is_empty() {
        return Ok(report);
    }
    let check = async {
        let actual: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM text_recall_documents").fetch_one(&mut *connection).await.map_err(error)?;
        let expected: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM events) + (SELECT COUNT(*) FROM claims WHERE status='active')").fetch_one(&mut *connection).await.map_err(error)?;
        let mismatches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM text_recall_documents d WHERE (d.record_type='event' AND NOT EXISTS (SELECT 1 FROM events e WHERE e.event_id=d.record_id AND e.owner=d.owner AND e.namespace=d.namespace AND substr(lower(e.summary),1)=d.summary AND d.requires_literal_fallback=(instr(e.summary,char(0))>0) AND d.subject='' AND d.predicate='' AND d.object='')) OR (d.record_type='claim' AND NOT EXISTS (SELECT 1 FROM claims c WHERE c.claim_id=d.record_id AND c.status='active' AND c.owner=d.owner AND c.namespace=d.namespace AND d.summary='' AND substr(lower(c.subject),1)=d.subject AND substr(lower(c.predicate),1)=d.predicate AND substr(lower(c.object),1)=d.object AND d.requires_literal_fallback=(instr(c.subject,char(0))>0 OR instr(c.predicate,char(0))>0 OR instr(c.object,char(0))>0)))")
            .fetch_one(&mut *connection).await.map_err(error)?;
        report.derived_documents = Some(actual);
        report.ledger_documents = Some(expected);
        report.content_consistent = actual == expected && mismatches == 0;
        if !report.content_consistent { report.issues.push("derived_content_differs_from_ledger".into()); }
        report.postings_consistent = posting_fingerprint(connection).await?;
        if !report.postings_consistent { report.issues.push("trigram_postings_differ_from_documents".into()); }
        Ok::<_, AppError>(())
    }.await;
    if let Err(error) = check {
        report.issues.push(error.to_string());
    }
    if report.issues.is_empty() {
        report.status = "ready".into();
    }
    Ok(report)
}

// Compare count + order-independent SHA-256 fingerprints of every posting. Streaming keeps
// memory bounded; no temporary tables or writes are needed on a read-only doctor connection.
// Derived text is explicitly cut before embedded NUL for stable tokenizer behavior;
// flagged source rows are also literal candidates, preserving matches after that boundary.
async fn posting_fingerprint(connection: &mut SqliteConnection) -> Result<bool, AppError> {
    let mut expected = ([0_u8; 32], 0_u64);
    {
        let mut rows = sqlx::query(
            "SELECT doc_id, summary, subject, predicate, object FROM text_recall_documents",
        )
        .fetch(&mut *connection);
        while let Some(row) = rows.try_next().await.map_err(error)? {
            let id: i64 = row.get("doc_id");
            for column in ["summary", "subject", "predicate", "object"] {
                let value: String = row.get(column);
                let chars: Vec<_> = value.chars().collect();
                for (offset, token) in chars.windows(3).enumerate() {
                    fingerprint_add(
                        &mut expected,
                        id,
                        column,
                        offset as i64,
                        &token.iter().collect::<String>(),
                    );
                }
            }
        }
    }
    let mut actual = ([0_u8; 32], 0_u64);
    let mut rows =
        sqlx::query("SELECT term, doc, col, offset FROM text_recall_vocab").fetch(&mut *connection);
    while let Some(row) = rows.try_next().await.map_err(error)? {
        fingerprint_add(
            &mut actual,
            row.get("doc"),
            &row.get::<String, _>("col"),
            row.get("offset"),
            &row.get::<String, _>("term"),
        );
    }
    Ok(expected == actual)
}

fn fingerprint_add(state: &mut ([u8; 32], u64), id: i64, column: &str, offset: i64, term: &str) {
    let mut hash = Sha256::new();
    hash.update(id.to_le_bytes());
    hash.update((column.len() as u64).to_le_bytes());
    hash.update(column.as_bytes());
    hash.update(offset.to_le_bytes());
    hash.update(term.as_bytes());
    for (slot, byte) in state.0.iter_mut().zip(hash.finalize()) {
        *slot ^= byte;
    }
    state.1 += 1;
}

impl SqliteStore {
    pub async fn inspect_retrieval_index(&self) -> Result<RetrievalIndexReport, AppError> {
        // One read snapshot prevents concurrent committed writes looking like corruption.
        let mut transaction = self.pool.begin().await.map_err(error)?;
        let report = inspect_retrieval_index_connection(&mut transaction).await?;
        transaction.rollback().await.map_err(error)?;
        Ok(report)
    }

    pub async fn rebuild_retrieval_index(&self) -> Result<RetrievalIndexReport, AppError> {
        let mut transaction = self.pool.begin().await.map_err(error)?;
        // Drop source/document triggers first, then virtual tables before their content.
        for (name, ddl) in RETRIEVAL_INDEX_DDL.iter().rev() {
            let kind = if ddl.starts_with("CREATE TRIGGER") {
                "TRIGGER"
            } else if ddl.starts_with("CREATE INDEX") {
                "INDEX"
            } else {
                "TABLE"
            };
            sqlx::query(&format!("DROP {kind} IF EXISTS {name}"))
                .execute(&mut *transaction)
                .await
                .map_err(error)?;
        }
        install_retrieval_index(&mut transaction).await?;
        // Supported FTS consistency command is allowed only inside this explicit write path.
        sqlx::query(
            "INSERT INTO text_recall_fts(text_recall_fts, rank) VALUES ('integrity-check', 1)",
        )
        .execute(&mut *transaction)
        .await
        .map_err(error)?;
        let mut report = inspect_retrieval_index_connection(&mut transaction).await?;
        if report.status != "ready" {
            return Err(AppError::Message(format!(
                "rebuilt index failed validation: {:?}",
                report.issues
            )));
        }
        transaction.commit().await.map_err(error)?;
        report.rebuild_performed = true;
        Ok(report)
    }
}
