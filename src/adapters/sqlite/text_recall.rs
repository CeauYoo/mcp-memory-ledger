use super::SqliteStore;
use crate::{
    domain::types::Owner,
    error::AppError,
    ports::text_memory_store::{
        TextMemoryHit, TextMemoryPage, TextMemoryQuery, TextMemoryReference, TextMemoryStore,
    },
};
use async_trait::async_trait;
use sqlx::{QueryBuilder, Row, Sqlite};

#[async_trait]
impl TextMemoryStore for SqliteStore {
    async fn recall_text(&self, query: TextMemoryQuery) -> Result<TextMemoryPage, AppError> {
        query.validate()?;
        let owner = match query.scope.owner().expect("validated scope") {
            Owner::Self_ => "self",
            Owner::User => "user",
            Owner::World => "world",
            Owner::Unknown => "unknown",
        };
        let namespace = query.scope.namespace().expect("validated scope").as_str();
        let mut hits = Vec::new();
        // Independent per-type selection prevents a recent event flood starving active claims.
        for (table, id, columns, is_claim) in [
            (
                "claims",
                "claim_id",
                &["subject", "predicate", "object"][..],
                true,
            ),
            ("events", "event_id", &["summary"][..], false),
        ] {
            let mut sql = QueryBuilder::<Sqlite>::new(format!("SELECT {id} AS id, ("));
            for (i, term) in query.terms.iter().enumerate() {
                if i > 0 {
                    sql.push(" + ");
                }
                sql.push("CASE WHEN (");
                for (j, column) in columns.iter().enumerate() {
                    if j > 0 {
                        sql.push(" OR ");
                    }
                    // instr treats %, _, quotes and punctuation literally; CJK needs no tokenizer.
                    sql.push(format!("instr(lower({column}), lower("));
                    sql.push_bind(term).push(")) > 0");
                }
                sql.push(") THEN 1 ELSE 0 END");
            }
            sql.push(format!(") AS matched_terms FROM {table} WHERE owner = "));
            sql.push_bind(owner)
                .push(" AND namespace = ")
                .push_bind(namespace);
            if is_claim {
                sql.push(" AND status = 'active'");
            }
            sql.push(" AND matched_terms > 0 ORDER BY matched_terms DESC, id ASC LIMIT ")
                .push_bind((query.limit + 1) as i64);
            let rows = sql.build().fetch_all(&self.pool).await.map_err(|error| {
                AppError::Message(format!("SQLite text recall failed: {error}"))
            })?;
            for row in rows {
                let id: String = row.get("id");
                hits.push(TextMemoryHit {
                    reference: if is_claim {
                        TextMemoryReference::Claim(id)
                    } else {
                        TextMemoryReference::Event(id)
                    },
                    matched_terms: row.get::<i64, _>("matched_terms") as usize,
                });
            }
        }
        // Each group is already score-descending/ID-ascending; claims are the first group.
        let has_more = hits.len() > query.limit;
        hits.truncate(query.limit);
        Ok(TextMemoryPage { hits, has_more })
    }
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
}
