use super::{SqliteStore, retrieval_index::retrieval_structure_issues};
use crate::{
    domain::types::Owner,
    error::AppError,
    ports::text_memory_store::{
        TextMemoryHit, TextMemoryPage, TextMemoryQuery, TextMemoryReference, TextMemoryStore,
    },
};
use async_trait::async_trait;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

#[async_trait]
impl TextMemoryStore for SqliteStore {
    async fn recall_text(&self, query: TextMemoryQuery) -> Result<TextMemoryPage, AppError> {
        query.validate()?;
        let mut transaction = self.pool.begin().await.map_err(sqlite_error)?;
        let structure = retrieval_structure_issues(&mut transaction).await?;
        let long_terms: Vec<_> = query
            .terms
            .iter()
            .filter(|term| term.chars().count() >= 3)
            .collect();
        let mut warning = (!structure.is_empty())
            .then(|| format!("derived_index_needs_rebuild: {}", structure.join(", ")));
        let use_index = warning.is_none() && !long_terms.is_empty();
        let groups = match select_groups(&mut transaction, &query, use_index).await {
            Ok(groups) => groups,
            Err(error) if use_index => {
                // Corrupt/unavailable derived data must not make the authoritative ledger
                // unreadable. A failed indexed statement never repairs or mutates anything.
                warning = Some(format!(
                    "derived_index_query_failed; explicit rebuild recommended: {error}"
                ));
                select_groups(&mut transaction, &query, false).await?
            }
            Err(error) => return Err(error),
        };
        let strategy = if warning.is_some() {
            "degraded_literal_scan"
        } else if long_terms.is_empty() {
            "literal_short_terms"
        } else if long_terms.len() == query.terms.len() {
            "fts5_trigram"
        } else {
            "fts5_trigram_with_short_literal_fallback"
        };
        let [claims, events] = groups;
        let available = claims.len() + events.len();
        // Reserve both types when present: ceil(limit/2) claims, floor(limit/2) events.
        // Redistribute unfilled quota. Claims retain precedence when only one slot exists.
        let mut claim_count = claims.len().min(query.limit.div_ceil(2));
        let mut event_count = events.len().min(query.limit / 2);
        let remaining = query.limit - claim_count - event_count;
        let extra_claims = remaining.min(claims.len() - claim_count);
        claim_count += extra_claims;
        event_count += (remaining - extra_claims).min(events.len() - event_count);
        // Interleave the allocated types so subsequent byte-budget packing also gives
        // historical evidence an early opportunity instead of consuming it all on claims.
        let mut claims = claims.into_iter().take(claim_count);
        let mut events = events.into_iter().take(event_count);
        let mut hits = Vec::with_capacity(claim_count + event_count);
        loop {
            let claim = claims.next();
            let event = events.next();
            if claim.is_none() && event.is_none() {
                break;
            }
            hits.extend(claim);
            hits.extend(event);
        }
        transaction.rollback().await.map_err(sqlite_error)?;
        Ok(TextMemoryPage {
            hits,
            has_more: available > query.limit,
            strategy,
            index_warning: warning,
        })
    }
}

fn sqlite_error(error: sqlx::Error) -> AppError {
    AppError::Message(format!("SQLite text recall failed: {error}"))
}

async fn select_groups(
    connection: &mut SqliteConnection,
    query: &TextMemoryQuery,
    indexed: bool,
) -> Result<[Vec<TextMemoryHit>; 2], AppError> {
    let mut groups = [Vec::new(), Vec::new()];
    for (group, hits) in groups.iter_mut().enumerate() {
        let mut sql = selection_query(query, group, indexed, false);
        for row in sql
            .build()
            .fetch_all(&mut *connection)
            .await
            .map_err(sqlite_error)?
        {
            let id = row.get("id");
            hits.push(TextMemoryHit {
                reference: if group == 0 {
                    TextMemoryReference::Claim(id)
                } else {
                    TextMemoryReference::Event(id)
                },
                matched_terms: row.get::<i64, _>("matched_terms") as usize,
            });
        }
    }
    Ok(groups)
}

fn selection_query(
    query: &TextMemoryQuery,
    group: usize,
    indexed: bool,
    explain: bool,
) -> QueryBuilder<'_, Sqlite> {
    let owner = match query.scope.owner().expect("validated scope") {
        Owner::Self_ => "self",
        Owner::User => "user",
        Owner::World => "world",
        Owner::Unknown => "unknown",
    };
    let namespace = query.scope.namespace().expect("validated scope").as_str();
    let (table, id, columns, kind) = if group == 0 {
        (
            "claims",
            "claim_id",
            &["subject", "predicate", "object"][..],
            "claim",
        )
    } else {
        ("events", "event_id", &["summary"][..], "event")
    };
    let mut sql = QueryBuilder::<Sqlite>::new(if explain { "EXPLAIN QUERY PLAN " } else { "" });
    if indexed {
        // FTS syntax is always a quoted literal; user punctuation is never an operator.
        let expression = query
            .terms
            .iter()
            .filter(|term| term.chars().count() >= 3)
            .map(|term| format!("\"{}\"", term.to_ascii_lowercase().replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        sql.push("WITH candidates AS MATERIALIZED (SELECT d.record_id FROM text_recall_fts CROSS JOIN text_recall_documents d ON d.doc_id = text_recall_fts.rowid WHERE text_recall_fts MATCH ")
            .push_bind(expression).push(" AND d.owner = ").push_bind(owner)
            .push(" AND d.namespace = ").push_bind(namespace)
            .push(" AND d.record_type = ").push_bind(kind);
        // Older SQLite trigram tokenizers stop at embedded NUL. Only exceptional
        // source rows need this additional path; a partial scope index keeps it cheap.
        sql.push(" UNION SELECT record_id FROM text_recall_documents WHERE owner = ")
            .push_bind(owner)
            .push(" AND namespace = ")
            .push_bind(namespace)
            .push(" AND record_type = ")
            .push_bind(kind)
            .push(" AND requires_literal_fallback=1");
        let short: Vec<_> = query
            .terms
            .iter()
            .filter(|term| term.chars().count() < 3)
            .collect();
        if !short.is_empty() {
            sql.push(format!(" UNION SELECT {id} FROM {table} WHERE "));
            scope(&mut sql, owner, namespace, group == 0);
            sql.push(" AND (");
            for (i, term) in short.into_iter().enumerate() {
                if i > 0 {
                    sql.push(" OR ");
                }
                literal_match(&mut sql, columns, term);
            }
            sql.push(")");
        }
        sql.push(") ");
    }
    sql.push(format!("SELECT {id} AS id, ("));
    for (i, term) in query.terms.iter().enumerate() {
        if i > 0 {
            sql.push(" + ");
        }
        sql.push("CASE WHEN ");
        literal_match(&mut sql, columns, term);
        sql.push(" THEN 1 ELSE 0 END");
    }
    sql.push(format!(") AS matched_terms FROM {table} WHERE "));
    scope(&mut sql, owner, namespace, group == 0);
    if indexed {
        sql.push(format!(" AND {id} IN (SELECT record_id FROM candidates)"));
    }
    sql.push(" AND matched_terms > 0 ORDER BY matched_terms DESC, ");
    // Persisted UTC nanosecond keys keep offsets equivalent and unknown legacy
    // Claim times last without inventing creation times from row order.
    sql.push("recorded_at_sort_key DESC, ");
    sql.push("id ASC LIMIT ")
        .push_bind((query.limit + 1) as i64);
    sql
}

fn scope<'a>(sql: &mut QueryBuilder<'a, Sqlite>, owner: &'a str, namespace: &'a str, claim: bool) {
    sql.push("owner = ")
        .push_bind(owner)
        .push(" AND namespace = ")
        .push_bind(namespace);
    if claim {
        sql.push(" AND status = 'active'");
    }
}

fn literal_match(sql: &mut QueryBuilder<'_, Sqlite>, columns: &[&str], term: &str) {
    sql.push("(");
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            sql.push(" OR ");
        }
        sql.push(format!("instr(lower({column}), "))
            .push_bind(term.to_ascii_lowercase())
            .push(") > 0");
    }
    sql.push(")");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::{
            recall_memory::{self, RecallMemoryInput},
            search_memory::SearchMemoryRecord,
        },
        domain::types::{MemoryScope, Namespace},
    };

    async fn event(store: &SqliteStore, id: &str, owner: &str, namespace: &str, text: &str) {
        sqlx::query("INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary) VALUES (?, '2026-01-01T00:00:00Z', ?, ?, 'observation', ?)")
            .bind(id).bind(owner).bind(namespace).bind(text).execute(&store.pool).await.unwrap();
    }
    async fn claim(store: &SqliteStore, id: &str, namespace: &str, text: &str, status: &str) {
        sqlx::query("INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status) VALUES (?, 'user', ?, 'user', 'prefers', ?, 'observed', ?)")
            .bind(id).bind(namespace).bind(text).bind(status).execute(&store.pool).await.unwrap();
    }
    #[tokio::test]
    async fn bilingual_scope_literal_and_claim_priority_eval() {
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}",
            directory.path().join("recall.sqlite").display()
        );
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        event(
            &store,
            "beijing",
            "user",
            "user/alice",
            "北京计划：喜欢咖啡，保存记忆",
        )
        .await;
        event(
            &store,
            "english",
            "user",
            "user/alice",
            "Coffee planning in Beijing",
        )
        .await;
        event(
            &store,
            "punctuation",
            "user",
            "user/alice",
            "literal %_\"*\\ coffee",
        )
        .await;
        event(&store, "decoy", "user", "user/bob", "北京咖啡记忆 Coffee").await;
        event(
            &store,
            "world-decoy",
            "world",
            "world",
            "北京咖啡记忆 Coffee",
        )
        .await;
        claim(&store, "active", "user/alice", "北京咖啡", "active").await;
        claim(&store, "stale", "user/alice", "北京咖啡", "superseded").await;
        claim(&store, "claim-decoy", "user/bob", "北京咖啡", "active").await;
        for i in 0..130 {
            event(
                &store,
                &format!("flood-{i:03}"),
                "user",
                "user/alice",
                "unrelated recent noise",
            )
            .await;
        }
        sqlx::query("INSERT INTO evidence_links (claim_id, event_id) VALUES ('active', 'beijing')")
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO episode_events (episode_reference, event_id) VALUES ('episode:bilingual', 'beijing')").execute(&store.pool).await.unwrap();
        let namespace = Namespace::parse("user/alice").unwrap();
        for (query, expected) in [
            ("北京", vec!["claim:active", "event:beijing"]),
            ("咖啡", vec!["claim:active", "event:beijing"]),
            ("记忆", vec!["event:beijing"]),
            ("COFFEE", vec!["event:english", "event:punctuation"]),
            ("%_\"*\\", vec!["event:punctuation"]),
        ] {
            let input = RecallMemoryInput {
                namespace: namespace.clone(),
                query: query.into(),
                limit: 5,
            };
            let result = recall_memory::execute(&store, input.clone()).await.unwrap();
            assert_eq!(result, recall_memory::execute(&store, input).await.unwrap());
            let ids: Vec<_> = result
                .records
                .iter()
                .map(|hit| match &hit.record {
                    SearchMemoryRecord::Claim { id, .. } | SearchMemoryRecord::Event { id, .. } => {
                        id.as_str()
                    }
                    _ => panic!("unsupported recall kind"),
                })
                .collect();
            assert_eq!(ids, expected, "query: {query}");
            if let Some(hit) = result.records.first()
                && let SearchMemoryRecord::Claim { provenance, .. } = &hit.record
            {
                assert_eq!(provenance.evidence_event_references, ["event:beijing"]);
                assert_eq!(provenance.episode_references, ["episode:bilingual"]);
            }
        }
        for i in 0..130 {
            event(
                &store,
                &format!("coffee-flood-{i:03}"),
                "user",
                "user/alice",
                "北京咖啡",
            )
            .await;
        }
        let result = recall_memory::execute(
            &store,
            RecallMemoryInput {
                namespace,
                query: "咖啡".into(),
                limit: 1,
            },
        )
        .await
        .unwrap();
        assert!(result.has_more);
        assert!(
            matches!(&result.records[0].record, SearchMemoryRecord::Claim { id, .. } if id == "claim:active")
        );
    }
    #[tokio::test]
    async fn store_rejects_unscoped_and_oversized_queries() {
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}",
            directory.path().join("recall.sqlite").display()
        );
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        for query in [
            TextMemoryQuery {
                scope: MemoryScope::legacy_unscoped(),
                terms: vec!["coffee".into()],
                limit: 1,
            },
            TextMemoryQuery {
                scope: MemoryScope::self_(),
                terms: vec!["咖".repeat(200)],
                limit: 1,
            },
            TextMemoryQuery {
                scope: MemoryScope::self_(),
                terms: vec!["coffee".into()],
                limit: 101,
            },
        ] {
            assert!(store.recall_text(query).await.is_err());
        }
    }
    #[tokio::test]
    async fn indexed_query_plan_materializes_match_before_scope_lookup() {
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}",
            directory.path().join("plan.sqlite").display()
        );
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        event(&store, "event", "user", "user/alice", "coffee target").await;
        claim(&store, "claim", "user/alice", "coffee target", "active").await;
        let version: String = sqlx::query_scalar("SELECT sqlite_version()")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        let query = TextMemoryQuery {
            scope: MemoryScope::for_namespace(Namespace::parse("user/alice").unwrap()),
            terms: vec!["coffee".into()],
            limit: 20,
        };
        for group in 0..2 {
            let mut explain = selection_query(&query, group, true, true);
            let statement = explain.sql().to_string();
            let rows = explain.build().fetch_all(&store.pool).await.unwrap();
            let plan: Vec<String> = rows.iter().map(|row| row.get("detail")).collect();
            assert!(
                plan.iter()
                    .any(|detail| detail.contains("MATERIALIZE candidates")),
                "{plan:?}"
            );
            let fts: Vec<_> = plan
                .iter()
                .filter(|detail| detail.contains("text_recall_fts VIRTUAL TABLE"))
                .collect();
            assert_eq!(fts.len(), 1, "{plan:?}");
            assert!(
                fts[0].contains("INDEX 0:M4"),
                "MATCH must run without per-source rowid probes: {plan:?}"
            );
            assert!(
                plan.iter()
                    .any(|detail| detail.contains("SEARCH d USING INTEGER PRIMARY KEY")),
                "{plan:?}"
            );
            assert!(
                plan.iter()
                    .any(|detail| detail.contains("idx_text_recall_literal_fallback")),
                "{plan:?}"
            );
            println!(
                "{}",
                serde_json::json!({"sqlite_runtime_version":version,"record_type":if group==0 {"claim"} else {"event"},"sql":statement,"bindings":{"owner":"user","namespace":"user/alice","terms":["coffee"],"limit":21},"plan":plan})
            );
        }
    }
}
