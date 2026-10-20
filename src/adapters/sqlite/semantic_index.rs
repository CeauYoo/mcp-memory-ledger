//! Explicit, disposable full-source indexing. Never a truth store or execution grant.
//! Rebuild and search are bounded resumable operations; no background inference.
use super::{
    SqliteStore,
    selection_reads::{hydrate_by_reference, hydrate_selection},
    store::rows::owner_as_str,
};
use crate::{
    adapters::local_embedding::LocalEmbeddingConfig,
    application::selection_source::SelectionSource,
    domain::types::{MemoryScope, Namespace},
    error::AppError,
    ports::{
        embedding::{EmbeddingPort, EmbeddingPurpose},
        text_memory_store::TextMemoryStore,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection};
use std::sync::Arc;

const VERSION: i64 = 2;
const V1_SOURCES: &[&str] = &[
    "claims",
    "events",
    "evidence_links",
    "episode_events",
    "reflections",
    "recall_applicability",
];
fn error() -> AppError {
    AppError::Message(
        "semantic index unavailable, stale or invalid; explicitly restart rebuild".into(),
    )
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticIndexStatus {
    pub namespace: String,
    pub generation: String,
    pub source_generation: String,
    pub index_version: i64,
    pub profile_digest: String,
    pub examined: u64,
    pub indexed: u64,
    pub omitted: u64,
    pub complete: bool,
    pub stale: bool,
    /// Complete means all eligible source kinds were examined at this generation; omissions remain explicit.
    pub next_valid_from: Option<String>,
    pub coverage: &'static str,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticSearchCursor {
    generation: String,
    query_digest: String,
    query_vector_digest: String,
    after: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct SemanticIndexHit {
    pub reference: String,
    pub content_digest: String,
    pub score: f64,
    pub applicability: &'static str,
    pub declared_applicability: Option<crate::domain::recall_applicability::RecallApplicability>,
    pub record: SelectionSource,
}
#[derive(Debug, Serialize)]
pub struct SemanticSearchPage {
    pub status: SemanticIndexStatus,
    pub hits: Vec<SemanticIndexHit>,
    pub scanned: usize,
    pub omitted: usize,
    pub omission_reasons: std::collections::BTreeMap<String, usize>,
    pub next_cursor: Option<SemanticSearchCursor>,
    pub has_more: bool,
    pub ranking: &'static str,
    pub applicability_policy: &'static str,
}
/// A transient query vector bound to the exact host profile and index generation.
/// Never deserialized from a tool request and never an execution grant.
pub struct PreparedSemanticQuery {
    namespace: String,
    generation: String,
    profile: String,
    query: String,
    query_digest: String,
    vector: Vec<f32>,
}
/// Source-balanced internal windows; these are not caller-controlled SQL fragments.
#[derive(Debug, Clone, Copy)]
pub enum SemanticSourceKind {
    Claim,
    Event,
    Episode,
    Experience,
}
impl SemanticSourceKind {
    pub const ALL: [Self; 4] = [Self::Claim, Self::Event, Self::Episode, Self::Experience];
    pub fn name(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Event => "event",
            Self::Episode => "experience_episode",
            Self::Experience => "experience_candidate",
        }
    }
    fn range(self) -> (&'static str, &'static str) {
        match self {
            Self::Claim => ("claim:", "claim;"),
            Self::Event => ("event:", "event;"),
            Self::Episode => ("experience_episode:", "experience_episode;"),
            Self::Experience => ("experience_candidate:", "experience_candidate;"),
        }
    }
}
#[derive(Clone, Copy)]
enum VectorWindow<'a> {
    AfterId(Option<&'a str>, i64),
    Kind(SemanticSourceKind),
}
// The existing UNIQUE(namespace,generation,reference) autoindex supplies a bounded
// binary range seek. No LIKE, source-kind table, schema change or sort is required.
const KIND_WINDOW_SQL: &str = "SELECT rowid AS rid,typeof(reference) AS rt,length(CAST(reference AS BLOB)) AS rb,typeof(content_digest) AS dt,length(CAST(content_digest AS BLOB)) AS db,dimensions,typeof(vector) AS vt,length(vector) AS vb,typeof(checksum) AS ct,length(CAST(checksum AS BLOB)) AS cb FROM derived_semantic_vectors INDEXED BY sqlite_autoindex_derived_semantic_vectors_1 WHERE namespace=? AND generation=? AND reference>=? AND reference<? ORDER BY reference LIMIT ?";
struct IndexApplicability {
    next_valid_from: Option<String>,
    eligible: bool,
    status: &'static str,
    declaration: Option<crate::domain::recall_applicability::RecallApplicability>,
}

#[derive(Serialize)]
struct RunControl {
    kind: i64,
    rowid: i64,
    examined: u64,
    indexed: u64,
    omitted: u64,
    complete: bool,
    next_valid_from: Option<String>,
}
fn run_checksum(
    namespace: &str,
    generation: &str,
    source: &str,
    version: i64,
    profile: &str,
    control: &RunControl,
) -> Result<String, AppError> {
    Ok(digest(
        &serde_json::to_vec(&(namespace, generation, source, version, profile, control))
            .map_err(|_| error())?,
    ))
}
struct State {
    status: SemanticIndexStatus,
    kind: i64,
    rowid: i64,
}
/// Host-owned policy is the existing local-embedding configuration, never tool arguments.
/// The injected port permits offline tests; its profile must exactly match host policy.
pub struct SemanticIndex {
    store: SqliteStore,
    config: LocalEmbeddingConfig,
    port: Arc<dyn EmbeddingPort>,
    profile: String,
}
impl SemanticIndex {
    pub fn new(
        store: SqliteStore,
        config: LocalEmbeddingConfig,
        port: Arc<dyn EmbeddingPort>,
    ) -> Result<Self, AppError> {
        if !config.enabled || !config.local_only_declared {
            return Err(invalid(
                "semantic index requires host-enabled local-only embedding",
            ));
        }
        config
            .validate()
            .map_err(|_| invalid("invalid semantic index host embedding configuration"))?;
        if &config.profile != port.profile() {
            return Err(invalid(
                "semantic index adapter profile differs from host configuration",
            ));
        }
        let profile = digest(&serde_json::to_vec(&config.profile).map_err(|_| error())?);
        Ok(Self {
            store,
            config,
            port,
            profile,
        })
    }
    fn authorize(&self, namespace: &Namespace) -> Result<(), AppError> {
        if !self
            .config
            .namespace_allowlist
            .iter()
            .any(|n| n == namespace.as_str())
        {
            return Err(invalid("semantic index namespace not allowed by host"));
        }
        Ok(())
    }
    /// Explicitly begins/replaces this namespace's rebuild. Does not invoke inference.
    /// Other namespace rows remain intact; source mutations conservatively stale all runs.
    pub async fn start(&self, namespace: &Namespace) -> Result<SemanticIndexStatus, AppError> {
        self.authorize(namespace)?;
        let mut tx = self.store.pool.begin().await.map_err(|_| error())?;
        install(&mut tx).await?;
        let source = source_generation(&mut tx).await?;
        let generation = uuid::Uuid::new_v4().to_string();
        let checksum = run_checksum(
            namespace.as_str(),
            &generation,
            &source,
            VERSION,
            &self.profile,
            &RunControl {
                kind: -1,
                rowid: 0,
                examined: 0,
                indexed: 0,
                omitted: 0,
                complete: false,
                next_valid_from: None,
            },
        )?;

        sqlx::query("DELETE FROM derived_semantic_vectors WHERE namespace=?")
            .bind(namespace.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|_| error())?;
        sqlx::query("INSERT OR REPLACE INTO derived_semantic_runs(namespace,generation,source_generation,version,profile,kind,source_rowid,examined,indexed,omitted,complete,control_digest) VALUES(?,?,?,?,?,-1,0,0,0,0,0,?)")
            .bind(namespace.as_str()).bind(&generation).bind(source).bind(VERSION).bind(&self.profile).bind(checksum).execute(&mut *tx).await.map_err(|_| error())?;
        tx.commit().await.map_err(|_| error())?;
        self.status(namespace).await
    }
    /// No source text or provider calls; absence/corruption is not silently repaired.
    pub async fn status(&self, namespace: &Namespace) -> Result<SemanticIndexStatus, AppError> {
        self.authorize(namespace)?;
        let mut tx = self.store.pool.begin().await.map_err(|_| error())?;
        let result = self.state(&mut tx, namespace).await?.status;
        tx.rollback().await.map_err(|_| error())?;
        Ok(result)
    }
    async fn state(
        &self,
        conn: &mut SqliteConnection,
        namespace: &Namespace,
    ) -> Result<State, AppError> {
        let source = source_generation(conn).await?;
        // Bound every stored text field before materialization, including corrupt state.
        let row = sqlx::query("SELECT generation,source_generation,version,profile,kind,source_rowid,examined,indexed,omitted,complete,vector_count,control_digest,next_valid_from FROM derived_semantic_runs WHERE namespace=? AND typeof(generation)='text' AND length(CAST(generation AS BLOB))=36 AND typeof(source_generation)='text' AND length(CAST(source_generation AS BLOB))=32 AND typeof(profile)='text' AND length(CAST(profile AS BLOB))=64 AND typeof(control_digest)='text' AND length(CAST(control_digest AS BLOB))=64 AND (next_valid_from IS NULL OR (typeof(next_valid_from)='text' AND length(CAST(next_valid_from AS BLOB))<=40))")
            .bind(namespace.as_str()).fetch_optional(&mut *conn).await.map_err(|_| error())?.ok_or_else(error)?;
        let get = |name| row.try_get::<i64, _>(name).map_err(|_| error());
        let generation: String = row.try_get("generation").map_err(|_| error())?;
        let recorded_source: String = row.try_get("source_generation").map_err(|_| error())?;
        let profile: String = row.try_get("profile").map_err(|_| error())?;
        let version = get("version")?;
        let complete = get("complete")?;
        let kind = get("kind")?;
        let examined = u64::try_from(get("examined")?).map_err(|_| error())?;
        let indexed = u64::try_from(get("indexed")?).map_err(|_| error())?;
        let omitted = u64::try_from(get("omitted")?).map_err(|_| error())?;
        if !(0..=1).contains(&complete)
            || !(-1..=3).contains(&kind)
            || indexed.checked_add(omitted) != Some(examined)
        {
            return Err(error());
        }
        if u64::try_from(get("vector_count")?).ok() != Some(indexed) {
            return Err(error());
        }
        let control = RunControl {
            kind,
            rowid: get("source_rowid")?,
            examined,
            indexed,
            omitted,
            complete: complete == 1,
            next_valid_from: row.try_get("next_valid_from").map_err(|_| error())?,
        };
        let stored_checksum: String = row.try_get("control_digest").map_err(|_| error())?;
        if stored_checksum
            != run_checksum(
                namespace.as_str(),
                &generation,
                &recorded_source,
                version,
                &profile,
                &control,
            )?
        {
            return Err(error());
        }
        let boundary_passed = control
            .next_valid_from
            .as_deref()
            .map(|value| {
                chrono::DateTime::parse_from_rfc3339(value).map(|t| chrono::Utc::now() >= t)
            })
            .transpose()
            .map_err(|_| error())?
            .unwrap_or(false);
        let stale = source != recorded_source
            || version != VERSION
            || profile != self.profile
            || boundary_passed;
        if complete == 1
            && !stale
            && !source_page(conn, namespace, kind, get("source_rowid")?, 1)
                .await?
                .is_empty()
        {
            return Err(error());
        }

        Ok(State {
            kind,
            rowid: get("source_rowid")?,
            status: SemanticIndexStatus {
                namespace: namespace.as_str().into(),
                generation,
                source_generation: recorded_source,
                index_version: version,
                profile_digest: profile,
                examined,
                indexed,
                omitted,
                complete: complete == 1,
                stale,
                next_valid_from: control.next_valid_from,
                coverage: if stale {
                    "stale_rebuild_required"
                } else if complete == 0 {
                    "building_partial"
                } else if omitted != 0 {
                    "completed_with_omissions"
                } else {
                    "all_active_claims_events_persisted_episodes_current_active_experiences_at_source_generation"
                },
            },
        })
    }
    /// Each call examines at most four source records (32 KiB each) and invokes at
    /// most one document embedding request. Failure commits neither cursor nor vectors.
    pub async fn resume(
        &self,
        namespace: &Namespace,
        generation: &str,
        batch_size: usize,
    ) -> Result<SemanticIndexStatus, AppError> {
        self.authorize(namespace)?;
        if !(1..=4).contains(&batch_size) || generation.len() != 36 {
            return Err(invalid(
                "semantic rebuild requires generation and batch_size 1..4",
            ));
        }
        tokio::time::timeout(
            std::time::Duration::from_millis(self.config.timeout_ms + 5000),
            self.resume_inner(namespace, generation, batch_size),
        )
        .await
        .map_err(|_| error())?
    }
    async fn resume_inner(
        &self,
        namespace: &Namespace,
        generation: &str,
        batch_size: usize,
    ) -> Result<SemanticIndexStatus, AppError> {
        let mut tx = self.store.pool.begin().await.map_err(|_| error())?;
        let state = self.state(&mut tx, namespace).await?;
        if state.status.stale || state.status.generation != generation {
            return Err(error());
        }
        if state.status.complete {
            return Ok(state.status);
        }
        // Keyset traversal avoids OFFSET and includes negative source rowids.
        let rows = source_page(&mut tx, namespace, state.kind, state.rowid, batch_size + 1).await?;
        let complete = rows.len() <= batch_size;
        let examined = rows.len().min(batch_size);
        let mut cursor = (state.kind, state.rowid);
        let mut records = Vec::new();
        for &(kind, rid) in rows.iter().take(batch_size) {
            cursor = (kind, rid);
            if let Some(record) = hydrate_selection(&mut tx, namespace, kind, rid, 32768).await? {
                records.push(record);
            }
        }
        tx.rollback().await.map_err(|_| error())?;
        // Applicability is authoritative and checked before any document transmission.
        let mut eligible = Vec::new();
        let mut next_valid_from = state.status.next_valid_from.clone();
        for record in records {
            let assessment = self.assess_applicability(namespace, &record).await?;
            if assessment.eligible {
                eligible.push(record);
            } else if let Some(boundary) = assessment.next_valid_from {
                let boundary_time =
                    chrono::DateTime::parse_from_rfc3339(&boundary).map_err(|_| error())?;
                let earlier = next_valid_from
                    .as_deref()
                    .map(|old| {
                        chrono::DateTime::parse_from_rfc3339(old).map(|old| boundary_time < old)
                    })
                    .transpose()
                    .map_err(|_| error())?
                    .unwrap_or(true);
                if earlier {
                    next_valid_from = Some(boundary);
                }
            }
        }
        let before_inference = self.status(namespace).await?;
        if before_inference.stale || before_inference.generation != generation {
            return Err(error());
        }
        let texts: Vec<String> = eligible
            .iter()
            .map(SelectionSource::embedding_text)
            .collect();
        let vectors = if texts.is_empty() {
            Vec::new()
        } else {
            self.port
                .embed(namespace, EmbeddingPurpose::Document, &texts)
                .await?
        };
        if vectors.len() != eligible.len()
            || vectors
                .iter()
                .any(|v| !valid_vector(v, self.config.profile.dimensions))
        {
            return Err(error());
        }
        let next_control = RunControl {
            next_valid_from,
            kind: cursor.0,
            rowid: cursor.1,
            complete,
            examined: state
                .status
                .examined
                .checked_add(examined as u64)
                .ok_or_else(error)?,
            indexed: state
                .status
                .indexed
                .checked_add(eligible.len() as u64)
                .ok_or_else(error)?,
            omitted: state
                .status
                .omitted
                .checked_add((examined - eligible.len()) as u64)
                .ok_or_else(error)?,
        };
        let checksum = run_checksum(
            namespace.as_str(),
            generation,
            &state.status.source_generation,
            VERSION,
            &self.profile,
            &next_control,
        )?;
        let mut tx = self.store.pool.begin().await.map_err(|_| error())?;
        // Compare-and-swap makes retries/concurrent workers/reset safe. A mutation
        // during inference invalidates the whole page instead of publishing stale data.
        let update = sqlx::query("UPDATE derived_semantic_runs SET kind=?,source_rowid=?,examined=examined+?,indexed=indexed+?,omitted=omitted+?,complete=?,control_digest=?,next_valid_from=? WHERE namespace=? AND generation=? AND source_generation=(SELECT generation FROM derived_semantic_source WHERE id=1) AND version=? AND profile=? AND kind=? AND source_rowid=? AND complete=0")
            .bind(cursor.0).bind(cursor.1).bind(examined as i64).bind(eligible.len() as i64).bind((examined-eligible.len()) as i64).bind(i64::from(complete)).bind(checksum).bind(&next_control.next_valid_from)
            .bind(namespace.as_str()).bind(generation).bind(VERSION).bind(&self.profile).bind(state.kind).bind(state.rowid).execute(&mut *tx).await.map_err(|_| error())?;
        if update.rows_affected() != 1 {
            return Err(error());
        }
        // Recheck trigger installation before persisting, including external schema edits.
        source_generation(&mut tx).await?;
        for (record, vector) in eligible.iter().zip(vectors) {
            let bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
            sqlx::query("INSERT INTO derived_semantic_vectors(namespace,generation,reference,content_digest,dimensions,vector,checksum) VALUES(?,?,?,?,?,?,?)")
                .bind(namespace.as_str()).bind(generation).bind(record.reference()).bind(record.content_digest()?).bind(vector.len() as i64).bind(&bytes).bind(digest(&bytes))
                .execute(&mut *tx).await.map_err(|_| error())?;
        }
        tx.commit().await.map_err(|_| error())?;
        self.status(namespace).await
    }
    async fn assess_applicability(
        &self,
        namespace: &Namespace,
        record: &SelectionSource,
    ) -> Result<IndexApplicability, AppError> {
        let declaration = self
            .store
            .recall_applicability(namespace.as_str(), &record.reference())
            .await?;
        let mut next_valid_from = None;
        let (eligible, status) = match &declaration {
            None => (true, "unknown_no_current_applicability_declaration"),
            Some(metadata) if metadata.content_digest != record.content_digest()? => {
                (true, "declaration_for_other_content_digest_unknown")
            }
            Some(metadata) => {
                let time_state = metadata.time_state(chrono::Utc::now())?;
                if time_state == "not_yet_valid" {
                    next_valid_from = metadata.valid_from.clone();
                }
                (
                    time_state != "expired" && time_state != "not_yet_valid",
                    match time_state {
                        "expired" => "expired",
                        "not_yet_valid" => "not_yet_valid",
                        _ => "declared_conditions_and_environment_not_evaluated",
                    },
                )
            }
        };
        Ok(IndexApplicability {
            next_valid_from,
            eligible,
            status,
            declaration,
        })
    }
    /// Page-local ranking only. Walk next_cursor to exhaustion and merge scores
    /// for full-index ranking. Every page is bound to the same run/query/profile.
    /// Incomplete/stale builds are rejected, never mislabeled as full coverage.
    pub async fn search_page(
        &self,
        namespace: &Namespace,
        generation: &str,
        query: &str,
        cursor: Option<SemanticSearchCursor>,
        page_size: usize,
    ) -> Result<SemanticSearchPage, AppError> {
        self.authorize(namespace)?;
        if query.trim().is_empty()
            || query.len() > 4096
            || !(1..=64).contains(&page_size)
            || generation.len() != 36
        {
            return Err(invalid(
                "semantic search requires bounded query, generation and page_size 1..64",
            ));
        }
        let query_digest = digest(query.as_bytes());
        let expected_vector = cursor.as_ref().map(|c| c.query_vector_digest.clone());
        let after = match cursor {
            Some(cursor)
                if cursor.generation == generation
                    && cursor.query_digest == query_digest
                    && cursor.after >= 0
                    && cursor.query_vector_digest.len() == 64 =>
            {
                cursor.after
            }
            Some(_) => {
                return Err(invalid(
                    "semantic search cursor differs from query or generation",
                ));
            }
            None => 0,
        };
        tokio::time::timeout(
            std::time::Duration::from_millis(self.config.timeout_ms + 5000),
            self.search_inner(
                namespace,
                generation,
                (query, &query_digest),
                VectorWindow::AfterId(expected_vector.as_deref(), after),
                page_size,
                None,
            ),
        )
        .await
        .map_err(|_| error())?
    }
    pub async fn prepare_query(
        &self,
        namespace: &Namespace,
        query: &str,
    ) -> Result<PreparedSemanticQuery, AppError> {
        self.authorize(namespace)?;
        if query.trim().is_empty() || query.len() > 4096 {
            return Err(invalid("semantic query must be 1..4096 bytes"));
        }
        let status = self.status(namespace).await?;
        if status.stale || !status.complete {
            return Err(error());
        }
        let vectors = self
            .port
            .embed(namespace, EmbeddingPurpose::Query, &[query.to_owned()])
            .await?;
        if vectors.len() != 1 || !valid_vector(&vectors[0], self.config.profile.dimensions) {
            return Err(error());
        }
        let after = self.status(namespace).await?;
        if after.stale || !after.complete || after.generation != status.generation {
            return Err(error());
        }
        Ok(PreparedSemanticQuery {
            namespace: namespace.as_str().into(),
            generation: status.generation,
            profile: self.profile.clone(),
            query: query.into(),
            query_digest: digest(query.as_bytes()),
            vector: vectors.into_iter().next().ok_or_else(error)?,
        })
    }
    pub async fn search_prepared_page(
        &self,
        namespace: &Namespace,
        prepared: &PreparedSemanticQuery,
        cursor: Option<SemanticSearchCursor>,
        page_size: usize,
    ) -> Result<SemanticSearchPage, AppError> {
        self.authorize(namespace)?;
        if !(1..=64).contains(&page_size) {
            return Err(invalid("page_size must be 1..64"));
        }
        let after = match cursor.as_ref() {
            None => 0,
            Some(cursor)
                if cursor.generation == prepared.generation
                    && cursor.query_digest == prepared.query_digest
                    && cursor.after >= 0 =>
            {
                cursor.after
            }
            Some(_) => return Err(invalid("query cursor binding mismatch")),
        };
        self.search_inner(
            namespace,
            &prepared.generation,
            (&prepared.query, &prepared.query_digest),
            VectorWindow::AfterId(
                cursor.as_ref().map(|c| c.query_vector_digest.as_str()),
                after,
            ),
            page_size,
            Some(prepared),
        )
        .await
    }
    /// One bounded source-kind prefix window using the same transient query vector.
    /// `has_more` reports an unexamined suffix; no ID cursor is emitted for this window.
    pub async fn search_prepared_kind(
        &self,
        namespace: &Namespace,
        prepared: &PreparedSemanticQuery,
        kind: SemanticSourceKind,
        limit: usize,
    ) -> Result<SemanticSearchPage, AppError> {
        self.authorize(namespace)?;
        if !(1..=64).contains(&limit) {
            return Err(invalid("kind window limit must be 1..64"));
        }
        self.search_inner(
            namespace,
            &prepared.generation,
            (&prepared.query, &prepared.query_digest),
            VectorWindow::Kind(kind),
            limit,
            Some(prepared),
        )
        .await
    }
    async fn search_inner(
        &self,
        namespace: &Namespace,
        generation: &str,
        query_binding: (&str, &str),
        window: VectorWindow<'_>,
        page_size: usize,
        prepared: Option<&PreparedSemanticQuery>,
    ) -> Result<SemanticSearchPage, AppError> {
        let (query, query_digest) = query_binding;
        let (expected_vector, after) = match window {
            VectorWindow::AfterId(expected, after) => (expected, after),
            VectorWindow::Kind(_) => (None, 0),
        };
        let status = self.status(namespace).await?;
        if status.stale || !status.complete || status.generation != generation {
            return Err(error());
        }
        let queries = if let Some(prepared) = prepared {
            if prepared.namespace != namespace.as_str()
                || prepared.generation != generation
                || prepared.profile != self.profile
                || prepared.query_digest != query_digest
            {
                return Err(error());
            }
            vec![prepared.vector.clone()]
        } else {
            self.port
                .embed(namespace, EmbeddingPurpose::Query, &[query.to_owned()])
                .await?
        };
        if queries.len() != 1 || !valid_vector(&queries[0], self.config.profile.dimensions) {
            return Err(error());
        }
        let query_vector_digest = digest(
            &queries[0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        if expected_vector.is_some_and(|v| v != query_vector_digest) {
            return Err(invalid(
                "query embedding changed between search pages; restart search",
            ));
        }

        let mut tx = self.store.pool.begin().await.map_err(|_| error())?;
        let state = self.state(&mut tx, namespace).await?;
        if state.status.stale || !state.status.complete || state.status.generation != generation {
            return Err(error());
        }
        // Preflight all metadata before vector/text allocation in this snapshot.
        let rows = match window {
            VectorWindow::AfterId(_, _) => {
                sqlx::query("SELECT rowid AS rid,typeof(reference) AS rt,length(CAST(reference AS BLOB)) AS rb,typeof(content_digest) AS dt,length(CAST(content_digest AS BLOB)) AS db,dimensions,typeof(vector) AS vt,length(vector) AS vb,typeof(checksum) AS ct,length(CAST(checksum AS BLOB)) AS cb FROM derived_semantic_vectors INDEXED BY derived_semantic_vector_page WHERE namespace=? AND generation=? AND id>? ORDER BY id LIMIT ?")
            .bind(namespace.as_str()).bind(generation).bind(after).bind((page_size+1) as i64).fetch_all(&mut *tx).await.map_err(|_| error())?
            },
            VectorWindow::Kind(kind) => {
                let (lower, upper) = kind.range();
                sqlx::query(KIND_WINDOW_SQL).bind(namespace.as_str()).bind(generation).bind(lower).bind(upper).bind((page_size+1) as i64).fetch_all(&mut *tx).await.map_err(|_| error())?
            },
        };
        for row in rows.iter().take(page_size) {
            let s = |name| row.try_get::<String, _>(name).map_err(|_| error());
            let n = |name| row.try_get::<i64, _>(name).map_err(|_| error());
            if s("rt")? != "text"
                || !(1..=1024).contains(&n("rb")?)
                || s("dt")? != "text"
                || n("db")? != 64
                || n("dimensions")? != self.config.profile.dimensions as i64
                || s("vt")? != "blob"
                || n("vb")? != self.config.profile.dimensions as i64 * 4
                || s("ct")? != "text"
                || n("cb")? != 64
            {
                return Err(error());
            }
        }
        let mut hits = Vec::new();
        let mut omitted = 0;
        let mut omission_reasons = std::collections::BTreeMap::new();
        let mut last = after;
        for row in rows.iter().take(page_size) {
            last = row.try_get("rid").map_err(|_| error())?;
            let row = sqlx::query("SELECT reference,content_digest,vector,checksum FROM derived_semantic_vectors WHERE rowid=? AND namespace=? AND generation=?")
                .bind(last).bind(namespace.as_str()).bind(generation).fetch_one(&mut *tx).await.map_err(|_| error())?;
            let reference: String = row.try_get("reference").map_err(|_| error())?;
            let expected: String = row.try_get("content_digest").map_err(|_| error())?;
            let bytes: Vec<u8> = row.try_get("vector").map_err(|_| error())?;
            let checksum: String = row.try_get("checksum").map_err(|_| error())?;
            let vector: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes([v[0], v[1], v[2], v[3]]))
                .collect();
            if digest(&bytes) != checksum || !valid_vector(&vector, self.config.profile.dimensions)
            {
                return Err(error());
            }
            if let Some(record) =
                hydrate_by_reference(&mut tx, namespace, &reference, 32768).await?
                && record.content_digest()? == expected
            {
                hits.push(SemanticIndexHit {
                    reference,
                    content_digest: expected,
                    score: cosine(&vector, &queries[0]),
                    applicability: "unknown_no_current_applicability_declaration",
                    declared_applicability: None,
                    record,
                });
            } else {
                omitted += 1;
                *omission_reasons
                    .entry("source_unavailable_oversized_or_digest_changed".into())
                    .or_default() += 1;
            }
        }
        tx.rollback().await.map_err(|_| error())?;
        let mut applicable = Vec::new();
        for mut hit in hits {
            let assessment = self.assess_applicability(namespace, &hit.record).await?;
            if assessment.eligible {
                hit.applicability = assessment.status;
                hit.declared_applicability = assessment.declaration;
                applicable.push(hit);
            } else {
                omitted += 1;
                *omission_reasons
                    .entry(assessment.status.into())
                    .or_default() += 1;
            }
        }
        // Applicability can expire with time without a write; checked on every read.
        // Any actual source write during hydration invalidates all page results.
        let final_status = self.status(namespace).await?;
        if final_status.stale || final_status.generation != generation {
            return Err(error());
        }
        applicable.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.reference.cmp(&b.reference))
        });
        Ok(SemanticSearchPage {
            status: final_status,
            hits: applicable,
            scanned: rows.len().min(page_size),
            omitted,
            omission_reasons,
            has_more: rows.len() > page_size,
            next_cursor: (rows.len() > page_size && matches!(window, VectorWindow::AfterId(_, _)))
                .then(|| SemanticSearchCursor {
                    generation: generation.into(),
                    query_digest: query_digest.into(),
                    query_vector_digest,
                    after: last,
                }),
            ranking: match window {
                VectorWindow::AfterId(_, _) => {
                    "page_local_cosine_not_global_top_k_no_execution_authority"
                }
                VectorWindow::Kind(_) => {
                    "kind_prefix_local_cosine_not_global_top_k_no_execution_authority"
                }
            },
            applicability_policy: "current_digest_time_declarations_only_conditions_and_environment_not_evaluated",
        })
    }
}
// Equality constraints use explicit scope indexes whose trailing implicit rowid
// supports an actual bounded seek, not a full-scope sort behind LIMIT.
async fn source_page(
    conn: &mut SqliteConnection,
    namespace: &Namespace,
    kind: i64,
    after: i64,
    limit: usize,
) -> Result<Vec<(i64, i64)>, AppError> {
    let scope = MemoryScope::for_namespace(namespace.clone());
    let owner = owner_as_str(scope.owner().ok_or_else(error)?);
    let mut result = Vec::new();
    // Visit candidate roots, including inactive ones, with a real bounded seek.
    // Hydration filters current active versions before payload materialization.
    for next_kind in kind.max(0)..=3 {
        let (comparison, bound) = if next_kind == kind {
            (">", after)
        } else {
            (">=", i64::MIN)
        };
        let sql = if next_kind == 0 {
            format!(
                "SELECT rowid FROM claims INDEXED BY derived_semantic_claims_scope WHERE owner=? AND namespace=? AND status='active' AND rowid{comparison}? ORDER BY rowid LIMIT ?"
            )
        } else if next_kind == 1 {
            format!(
                "SELECT rowid FROM events INDEXED BY derived_semantic_events_scope WHERE owner=? AND namespace=? AND rowid{comparison}? ORDER BY rowid LIMIT ?"
            )
        } else {
            String::new()
        };
        let sql = if next_kind == 2 {
            format!(
                "SELECT rowid FROM experience_episodes INDEXED BY derived_semantic_episodes_scope WHERE namespace=?2 AND ?1=?1 AND rowid{comparison}?3 ORDER BY rowid LIMIT ?4"
            )
        } else if next_kind == 3 {
            format!(
                "SELECT rowid FROM experience_candidates INDEXED BY derived_semantic_candidates_scope WHERE namespace=?2 AND ?1=?1 AND rowid{comparison}?3 ORDER BY rowid LIMIT ?4"
            )
        } else {
            sql
        };
        let ids: Vec<i64> = sqlx::query_scalar(&sql)
            .bind(owner)
            .bind(namespace.as_str())
            .bind(bound)
            .bind((limit - result.len()) as i64)
            .fetch_all(&mut *conn)
            .await
            .map_err(|_| error())?;
        result.extend(ids.into_iter().map(|id| (next_kind, id)));
        if result.len() == limit {
            break;
        }
    }
    Ok(result)
}
fn valid_vector(v: &[f32], dimensions: usize) -> bool {
    v.len() == dimensions
        && !v.is_empty()
        && v.iter().all(|v| v.is_finite())
        && v.iter().any(|v| *v != 0.0)
}
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let norm = |v: &[f32]| v.iter().map(|v| (*v as f64).powi(2)).sum::<f64>().sqrt();
    a.iter()
        .zip(b)
        .map(|(a, b)| *a as f64 * *b as f64)
        .sum::<f64>()
        / (norm(a) * norm(b))
}
fn ddl_v1() -> Vec<(String, String)> {
    let mut entries = vec![
        ("derived_semantic_source".into(), "CREATE TABLE derived_semantic_source(id INTEGER PRIMARY KEY CHECK(id=1),generation TEXT NOT NULL)".into()),
        ("derived_semantic_runs".into(), "CREATE TABLE derived_semantic_runs(namespace TEXT PRIMARY KEY,generation TEXT NOT NULL,source_generation TEXT NOT NULL,version INTEGER NOT NULL,profile TEXT NOT NULL,kind INTEGER NOT NULL,source_rowid INTEGER NOT NULL,examined INTEGER NOT NULL,indexed INTEGER NOT NULL,omitted INTEGER NOT NULL,complete INTEGER NOT NULL,vector_count INTEGER NOT NULL DEFAULT 0,control_digest TEXT NOT NULL)".into()),
        ("derived_semantic_vectors".into(), "CREATE TABLE derived_semantic_vectors(id INTEGER PRIMARY KEY,namespace TEXT NOT NULL,generation TEXT NOT NULL,reference TEXT NOT NULL,content_digest TEXT NOT NULL,dimensions INTEGER NOT NULL,vector BLOB NOT NULL,checksum TEXT NOT NULL,UNIQUE(namespace,generation,reference))".into()),
    ];
    entries.extend([
        ("derived_semantic_claims_scope".into(),"CREATE INDEX derived_semantic_claims_scope ON claims(owner,namespace,status)".into()),
        ("derived_semantic_events_scope".into(),"CREATE INDEX derived_semantic_events_scope ON events(owner,namespace)".into()),
        ("derived_semantic_vector_page".into(),"CREATE INDEX derived_semantic_vector_page ON derived_semantic_vectors(namespace,generation,id)".into()),
        ("derived_semantic_vector_insert".into(),"CREATE TRIGGER derived_semantic_vector_insert AFTER INSERT ON derived_semantic_vectors BEGIN UPDATE derived_semantic_runs SET vector_count=vector_count+1 WHERE namespace=new.namespace AND generation=new.generation; END".into()),
        ("derived_semantic_vector_delete".into(),"CREATE TRIGGER derived_semantic_vector_delete AFTER DELETE ON derived_semantic_vectors BEGIN UPDATE derived_semantic_runs SET vector_count=vector_count-1 WHERE namespace=old.namespace AND generation=old.generation; END".into()),
        ("derived_semantic_vector_update".into(),"CREATE TRIGGER derived_semantic_vector_update AFTER UPDATE ON derived_semantic_vectors BEGIN UPDATE derived_semantic_runs SET complete=-1 WHERE (namespace=old.namespace AND generation=old.generation) OR (namespace=new.namespace AND generation=new.generation); END".into()),
    ]);
    for table in V1_SOURCES {
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            let name = format!("derived_semantic_{table}_{operation}");
            entries.push((name.clone(),format!("CREATE TRIGGER {name} AFTER {operation} ON {table} BEGIN UPDATE derived_semantic_source SET generation=lower(hex(randomblob(16))) WHERE id=1; END")));
        }
    }
    entries
}
pub(super) fn ddl() -> Vec<(String, String)> {
    let mut entries = ddl_v1();
    for (name, sql) in &mut entries {
        if name == "derived_semantic_runs" {
            *sql = sql.replace(
                "control_digest TEXT NOT NULL)",
                "control_digest TEXT NOT NULL,next_valid_from TEXT)",
            );
        }
    }
    entries.push((
        "derived_semantic_episodes_scope".into(),
        "CREATE INDEX derived_semantic_episodes_scope ON experience_episodes(namespace)".into(),
    ));
    entries.push((
        "derived_semantic_candidates_scope".into(),
        "CREATE INDEX derived_semantic_candidates_scope ON experience_candidates(namespace)".into(),
    ));
    for table in [
        "experience_episodes",
        "experience_episode_sources",
        "experience_candidates",
        "experience_candidate_versions",
        "experience_candidate_sources",
    ] {
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            let name = format!("derived_semantic_{table}_{operation}");
            entries.push((name.clone(), format!("CREATE TRIGGER {name} AFTER {operation} ON {table} BEGIN UPDATE derived_semantic_source SET generation=lower(hex(randomblob(16))) WHERE id=1; END")));
        }
    }
    entries
}
/// Optional extension: absent is valid, partial/modified/foreign objects are not.
/// Reads never install or repair anything. Bound untrusted DDL before allocation.
pub(super) async fn structure_issues(conn: &mut SqliteConnection) -> Result<Vec<String>, AppError> {
    let rows = sqlx::query("SELECT substr(name,1,128) AS name,CASE WHEN length(CAST(sql AS BLOB))<=16384 THEN sql ELSE NULL END AS sql FROM sqlite_master WHERE (name GLOB 'derived_semantic_*' OR tbl_name GLOB 'derived_semantic_*') AND name NOT LIKE 'sqlite_autoindex_%' LIMIT 65")
        .fetch_all(&mut *conn).await.map_err(|_| error())?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut expected = ddl();
    let old = ddl_v1();
    if rows.len() == old.len()
        && old.iter().all(|(name, definition)| {
            rows.iter().any(|r| {
                r.try_get::<String, _>("name").ok().as_ref() == Some(name)
                    && r.try_get::<Option<String>, _>("sql")
                        .ok()
                        .flatten()
                        .as_ref()
                        == Some(definition)
            })
        })
    {
        expected = old;
    }
    if rows.len() != expected.len() {
        return Ok(vec![
            "semantic index extension is partial or contains foreign objects".into(),
        ]);
    }
    for (name, definition) in expected {
        let actual = rows
            .iter()
            .find(|r| r.try_get::<String, _>("name").ok().as_deref() == Some(name.as_str()));
        if actual
            .and_then(|r| r.try_get::<Option<String>, _>("sql").ok())
            .flatten()
            .as_deref()
            != Some(definition.as_str())
        {
            return Ok(vec![
                "semantic index extension has noncanonical objects".into(),
            ]);
        }
    }
    Ok(Vec::new())
}
pub(super) async fn source_generation(conn: &mut SqliteConnection) -> Result<String, AppError> {
    if !structure_issues(conn).await?.is_empty() {
        return Err(error());
    }
    sqlx::query_scalar::<_,String>("SELECT generation FROM derived_semantic_source WHERE id=1 AND typeof(generation)='text' AND length(CAST(generation AS BLOB))=32").fetch_one(&mut *conn).await.map_err(|_| error())
}
async fn install(conn: &mut SqliteConnection) -> Result<(), AppError> {
    if !structure_issues(conn).await?.is_empty() {
        return Err(error());
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE name GLOB 'derived_semantic_*'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|_| error())?;
    if count != 0 {
        let has_boundary: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_table_info('derived_semantic_runs') WHERE name='next_valid_from'").fetch_one(&mut *conn).await.map_err(|_| error())?;
        if has_boundary != 1 {
            return Err(invalid(
                "semantic index v1 requires explicit index-reset then rebuild as v2",
            ));
        }
    }
    if count == 0 {
        for (_, statement) in ddl() {
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .map_err(|_| error())?;
        }
        sqlx::query("INSERT INTO derived_semantic_source VALUES(1,lower(hex(randomblob(16))))")
            .execute(&mut *conn)
            .await
            .map_err(|_| error())?;
    }
    Ok(())
}
/// Caller must own an explicit teardown transaction with foreign keys disabled,
/// then validate the remaining authoritative schema before committing.
pub(super) async fn teardown(conn: &mut SqliteConnection) -> Result<(), AppError> {
    // DROP TABLE also implicitly drops attached triggers/indexes. Refuse unknown
    // dependents before any schema write rather than accidentally deleting them.
    let known = ddl()
        .into_iter()
        .map(|(name, _)| name)
        .collect::<std::collections::BTreeSet<_>>();
    let objects: Vec<String> = sqlx::query_scalar("SELECT substr(name,1,128) FROM sqlite_master WHERE (name GLOB 'derived_semantic_*' OR tbl_name GLOB 'derived_semantic_*') AND name NOT LIKE 'sqlite_autoindex_%' LIMIT 65")
        .fetch_all(&mut *conn).await.map_err(|_|error())?;
    if objects.len() > known.len() || objects.iter().any(|name| !known.contains(name)) {
        return Err(error());
    }
    for (name, _) in ddl().into_iter().rev() {
        let kind: Option<String> =
            sqlx::query_scalar("SELECT type FROM sqlite_master WHERE name=?")
                .bind(&name)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|_| error())?;
        let Some(kind) = kind else {
            continue;
        };
        if !["table", "index", "trigger", "view"].contains(&kind.as_str()) {
            return Err(error());
        }
        sqlx::query(&format!("DROP {kind} {name}"))
            .execute(&mut *conn)
            .await
            .map_err(|_| error())?;
    }
    Ok(())
}

#[cfg(test)]
mod kind_window_tests {
    use super::*;
    #[tokio::test]
    async fn stratified_vector_windows_use_existing_unique_prefix_range_without_sort() {
        let dir = tempfile::tempdir().unwrap();
        let url = crate::support::sqlite_url::sqlite_file_url(
            &dir.path().canonicalize().unwrap().join("kind-plans.sqlite"),
        );
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let mut conn = store.pool.acquire().await.unwrap();
        install(&mut conn).await.unwrap();
        let columns =
            sqlx::query("PRAGMA index_info('sqlite_autoindex_derived_semantic_vectors_1')")
                .fetch_all(&mut *conn)
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.get::<String, _>("name"))
                .collect::<Vec<_>>();
        assert_eq!(columns, ["namespace", "generation", "reference"]);
        for kind in SemanticSourceKind::ALL {
            let (lower, upper) = kind.range();
            let plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {KIND_WINDOW_SQL}"))
                .bind("project/a")
                .bind("generation")
                .bind(lower)
                .bind(upper)
                .bind(65_i64)
                .fetch_all(&mut *conn)
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.get::<String, _>("detail"))
                .collect::<Vec<_>>();
            assert!(
                !plan
                    .iter()
                    .any(|p| p.contains("SCAN ") || p.contains("TEMP B-TREE")),
                "{plan:?}"
            );
            assert!(
                plan.iter().any(
                    |p| p.contains("sqlite_autoindex_derived_semantic_vectors_1")
                        && p.contains("namespace=? AND generation=? AND reference>?")
                        && p.contains("reference<?")
                ),
                "{plan:?}"
            );
        }
    }
}
