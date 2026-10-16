//! Read-only provenance graph. Both normalized relations and payload references
//! are edges, so a foreign row cannot hide an incoming reference behind a label.
use super::{AdoptionInventory, db};
use crate::{
    domain::{
        event::EventReference,
        experience::{ExperienceCandidate, PersistedEpisode},
        feedback_candidate::FeedbackCandidate,
        types::Namespace,
    },
    error::AppError,
};
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};
use std::collections::{BTreeMap, BTreeSet};

type Key = (String, String);
fn key(table: &str, id: &str) -> Key {
    (table.into(), id.into())
}
fn version_key(id: &str, version: i64) -> Key {
    key(
        "experience_candidate_versions",
        &format!("{}:{id}:{version}", id.len()),
    )
}
#[derive(Default)]
struct Node {
    scopes: BTreeSet<(String, String)>,
    edges: BTreeSet<Key>,
    errors: BTreeSet<String>,
    exists: bool,
}
#[derive(Default)]
struct Graph {
    nodes: BTreeMap<Key, Node>,
}
impl Graph {
    fn add(&mut self, k: &Key, namespace: Option<&str>, owner: Option<&str>) {
        let n = self.nodes.entry(k.clone()).or_default();
        n.exists = true;
        if let Some(ns) = namespace {
            n.scopes.insert((ns.into(), owner.unwrap_or("").into()));
        }
    }
    fn scoped(&mut self, k: &Key, ns: &str) {
        let owner = Namespace::parse(ns)
            .ok()
            .map(|v| match v.derived_owner() {
                crate::domain::types::Owner::World => "world",
                crate::domain::types::Owner::Self_ => "self",
                crate::domain::types::Owner::User => "user",
                _ => "unknown",
            })
            .unwrap_or("unknown");
        self.add(k, Some(ns), Some(owner));
    }
    fn edge(&mut self, a: &Key, b: &Key) {
        self.nodes
            .entry(a.clone())
            .or_default()
            .edges
            .insert(b.clone());
        self.nodes
            .entry(b.clone())
            .or_default()
            .edges
            .insert(a.clone());
    }
    fn check(&mut self, k: &Key, valid: bool, reason: &str) {
        if !valid {
            self.nodes
                .entry(k.clone())
                .or_default()
                .errors
                .insert(reason.into());
        }
    }
    fn finish(&self, ns: &str, report: &mut AdoptionInventory) {
        let mut todo: Vec<_> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.scopes.iter().any(|(s, _)| s == ns))
            .map(|(k, _)| k.clone())
            .collect();
        let mut seen = BTreeSet::new();
        while let Some(k) = todo.pop() {
            if !seen.insert(k.clone()) {
                continue;
            }
            let n = &self.nodes[&k];
            if !n.exists {
                report.block(format!("{}: missing provenance endpoint", k.0));
            }
            if n.scopes.is_empty() || n.scopes.iter().any(|(s, o)| s != ns || o != "world") {
                report.block(format!(
                    "{}: history closure crosses namespace or has unknown/non-World attribution",
                    k.0
                ));
            }
            for error in &n.errors {
                report.block(format!("{}: {error}", k.0));
            }
            todo.extend(n.edges.iter().cloned());
        }
        report.blocked_reasons.sort();
        report.blocked_reasons.dedup();
    }
}
// Full table scans are needed to prove absence of JSON-only incoming edges.
// Check limits before materializing; never truncate and then approve.
fn columns(table: &str) -> &'static str {
    match table {
        "events" => "event_id,namespace,owner",
        "claims" => "claim_id,namespace,owner",
        "reflections" => {
            "reflection_id,superseded_claim_id,replacement_claim_id,supporting_evidence_event_ids,scope_status,evidence_normalized,(requested_identity_update IS NOT NULL OR requested_commitment_updates IS NOT NULL) AS global_effect"
        }
        "reflection_scopes" => "reflection_id,role,owner,namespace",
        "reflection_evidence" => "reflection_id,event_id,position",
        "reflection_trigger_ledger" => {
            "ledger_id,trigger_type,namespace,trigger_key,status,evidence_window,reflection_id"
        }
        "experience_episodes" => "episode_id,namespace,recorded_at,payload_json",
        "experience_episode_sources" => "episode_id,event_id",
        "experience_candidates" => "candidate_id,namespace,current_version",
        "experience_candidate_versions" => {
            "candidate_id,version,recorded_at,status,kind,payload_json,change_kind,rollback_target_version"
        }
        "experience_candidate_sources" => "candidate_id,version,episode_id",
        "feedback_candidates" => {
            "candidate_id,namespace,target_claim_id,expected_target_version,state,revision,candidate_json"
        }
        _ => unreachable!("only fixed inventory tables are queried"),
    }
}
async fn rows(conn: &mut SqliteConnection, table: &str) -> Result<Vec<SqliteRow>, AppError> {
    sqlx::query(&format!("SELECT {} FROM {table}", columns(table)))
        .fetch_all(conn)
        .await
        .map_err(db)
}

fn string(row: &SqliteRow, column: &str) -> String {
    row.get(column)
}
fn optional(row: &SqliteRow, column: &str) -> Option<String> {
    row.get(column)
}
fn strings(raw: &str) -> Option<Vec<String>> {
    serde_json::from_str(raw).ok()
}
fn set(values: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    values.into_iter().collect()
}
// Reject unrecognized fields even for older persisted types that deserialize
// permissively: otherwise an unknown provenance field could be silently lost.
fn known_fields(input: &serde_json::Value, output: &serde_json::Value) -> bool {
    match (input, output) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => a
            .iter()
            .all(|(k, v)| b.get(k).is_some_and(|w| known_fields(v, w))),
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(v, w)| known_fields(v, w))
        }
        _ => true,
    }
}
fn decode<T: DeserializeOwned + Serialize>(raw: &str) -> Option<T> {
    let input: serde_json::Value = serde_json::from_str(raw).ok()?;
    let value: T = serde_json::from_str(raw).ok()?;
    known_fields(&input, &serde_json::to_value(&value).ok()?).then_some(value)
}
fn bad_json(report: &mut AdoptionInventory, table: &str) {
    report.block(format!("{table}: malformed or unsupported provenance payload; absence of incoming references cannot be proven"));
}

pub(super) async fn validate(
    conn: &mut SqliteConnection,
    ns: &str,
    report: &mut AdoptionInventory,
) -> Result<(), AppError> {
    let tables = [
        "events",
        "claims",
        "reflections",
        "reflection_scopes",
        "reflection_evidence",
        "reflection_trigger_ledger",
        "feedback_candidates",
        "experience_episodes",
        "experience_episode_sources",
        "experience_candidates",
        "experience_candidate_versions",
        "experience_candidate_sources",
    ];
    let total: i64 = report
        .database_counts
        .iter()
        .filter(|c| tables.contains(&c.table.as_str()))
        .map(|c| c.rows)
        .sum();
    if total > 100_000 {
        report.block("history provenance exceeds bounded inventory limit (100000 rows)");
        return Ok(());
    }
    let mut bytes = 0.0;
    for table in tables {
        let invalid_types = columns(table)
            .split(',')
            .map(|c| {
                let (expr, name) = c.split_once(" AS ").unwrap_or((c, c));
                let integer = matches!(
                    name,
                    "version"
                        | "current_version"
                        | "position"
                        | "revision"
                        | "evidence_normalized"
                        | "global_effect"
                        | "rollback_target_version"
                );
                let nullable = matches!(
                    name,
                    "superseded_claim_id" | "replacement_claim_id" | "rollback_target_version"
                ) || (table == "reflection_trigger_ledger"
                    && name == "reflection_id");
                format!(
                    "typeof({expr}) NOT IN ('{}'{})",
                    if integer { "integer" } else { "text" },
                    if nullable { ",'null'" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        let invalid: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} WHERE {invalid_types}"
        ))
        .fetch_one(&mut *conn)
        .await
        .map_err(db)?;
        if invalid > 0 {
            report.block(format!(
                "{table}: unsupported provenance storage type or null identifier"
            ));
            return Ok(());
        }
        // Account for every field we will load, including identifiers, before
        // any materialization. Mutation payloads are represented by a boolean.
        let sizes = columns(table)
            .split(',')
            .map(|c| {
                let c = c.split(" AS ").next().unwrap();
                format!("COALESCE(length(CAST({c} AS BLOB)),0)")
            })
            .collect::<Vec<_>>()
            .join("+");
        bytes += sqlx::query_scalar::<_, f64>(&format!("SELECT total({sizes}) FROM {table}"))
            .fetch_one(&mut *conn)
            .await
            .map_err(db)?;
    }
    if bytes > 16_777_216.0 {
        report.block(
            "history provenance exceeds bounded inventory limit (16 MiB of projected fields)",
        );
        return Ok(());
    }
    let mut g = Graph::default();
    for (table, id) in [("events", "event_id"), ("claims", "claim_id")] {
        // Do not materialize arbitrary fact text in the graph.
        for r in sqlx::query(&format!("SELECT {id}, namespace, owner FROM {table}"))
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
        {
            g.add(
                &key(table, &string(&r, id)),
                Some(&string(&r, "namespace")),
                Some(&string(&r, "owner")),
            );
        }
    }
    reflections(conn, &mut g, report).await?;
    experience(conn, &mut g, report).await?;
    feedback(conn, &mut g, report).await?;
    triggers(conn, &mut g, report).await?;
    g.finish(ns, report);
    Ok(())
}

async fn reflections(
    conn: &mut SqliteConnection,
    g: &mut Graph,
    report: &mut AdoptionInventory,
) -> Result<(), AppError> {
    let mut scopes: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    for r in rows(conn, "reflection_scopes").await? {
        let id = string(&r, "reflection_id");
        let ns = string(&r, "namespace");
        let owner = string(&r, "owner");
        // Scope rows are not proof that the reflection itself exists.
        g.nodes
            .entry(key("reflections", &id))
            .or_default()
            .scopes
            .insert((ns.clone(), owner.clone()));
        scopes
            .entry(id)
            .or_default()
            .push((string(&r, "role"), ns, owner));
    }
    let mut normalized: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
    for r in rows(conn, "reflection_evidence").await? {
        let id = string(&r, "reflection_id");
        let event = string(&r, "event_id");
        g.edge(&key("reflections", &id), &key("events", &event));
        normalized
            .entry(id)
            .or_default()
            .push((r.get("position"), event));
    }
    for r in rows(conn, "reflections").await? {
        let id = string(&r, "reflection_id");
        let k = key("reflections", &id);
        g.add(&k, None, None);
        let status = string(&r, "scope_status");
        if !matches!(status.as_str(), "verified" | "legacy_unambiguous") {
            report.block("unsupported history reflections: unknown attribution cannot exclude incoming references");
        }
        let declared = scopes.get(&id).cloned().unwrap_or_default();
        g.check(
            &k,
            declared
                .iter()
                .filter(|(role, _, _)| role == "origin")
                .count()
                == 1,
            "reflection must have exactly one origin scope",
        );
        let old = optional(&r, "superseded_claim_id");
        let new = optional(&r, "replacement_claim_id");
        for c in [&old, &new].into_iter().flatten() {
            g.edge(&k, &key("claims", c));
        }
        let Some(evidence) = strings(&string(&r, "supporting_evidence_event_ids")) else {
            bad_json(report, "reflections");
            continue;
        };
        for e in &evidence {
            g.edge(&k, &key("events", e));
        }
        let mut norm = normalized.remove(&id).unwrap_or_default();
        norm.sort();
        let unique: Vec<_> = evidence.iter().fold(Vec::new(), |mut a, e| {
            if !a.contains(e) {
                a.push(e.clone())
            }
            a
        });
        g.check(
            &k,
            r.get::<i64, _>("evidence_normalized") == 1
                && norm.iter().enumerate().all(|(i, (p, _))| *p == i as i64)
                && norm.into_iter().map(|(_, e)| e).collect::<Vec<_>>() == unique,
            "reflection payload and normalized evidence differ",
        );
        g.check(
            &k,
            old.is_some() || new.is_some() || !evidence.is_empty(),
            "reflection has no durable sources",
        );
        let global = r.get::<bool, _>("global_effect");
        if global {
            g.nodes
                .get_mut(&k)
                .unwrap()
                .scopes
                .insert(("self".into(), "self".into()));
        }
        // Effects must match actual mutations, not just a trusted scope label.
        let mut expected = BTreeSet::new();
        if let Some(c) = old
            && let Some(n) = g.nodes.get(&key("claims", &c))
        {
            expected.extend(n.scopes.clone());
        }
        if status == "legacy_unambiguous"
            && let Some(c) = new
            && let Some(n) = g.nodes.get(&key("claims", &c))
        {
            expected.extend(n.scopes.clone());
        }
        if global {
            expected.insert(("self".into(), "self".into()));
        }
        let actual = declared
            .iter()
            .filter(|(role, _, _)| role == "affected")
            .map(|(_, n, o)| (n.clone(), o.clone()))
            .collect();
        g.check(
            &k,
            expected == actual,
            "reflection affected scopes differ from mutations",
        );
    }
    Ok(())
}

async fn experience(
    conn: &mut SqliteConnection,
    g: &mut Graph,
    report: &mut AdoptionInventory,
) -> Result<(), AppError> {
    let mut episode_sources: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in rows(conn, "experience_episode_sources").await? {
        let id = string(&r, "episode_id");
        let event = string(&r, "event_id");
        g.edge(&key("experience_episodes", &id), &key("events", &event));
        episode_sources.entry(id).or_default().insert(event);
    }
    for r in rows(conn, "experience_episodes").await? {
        let id = string(&r, "episode_id");
        let ns = string(&r, "namespace");
        let k = key("experience_episodes", &id);
        g.scoped(&k, &ns);
        let Some(p) = decode::<PersistedEpisode>(&string(&r, "payload_json")) else {
            bad_json(report, "experience_episodes");
            continue;
        };
        g.scoped(&k, p.namespace.as_str());
        g.check(
            &k,
            p.episode_id == id
                && p.namespace.as_str() == ns
                && p.recorded_at.to_rfc3339() == string(&r, "recorded_at")
                && p.content.validate().is_ok(),
            "episode payload does not match row or content contract",
        );
        let mut refs = BTreeSet::new();
        for source in &p.content.source_event_refs {
            if let Ok(e) = EventReference::parse(source) {
                refs.insert(e.event_id().to_owned());
                g.edge(&k, &key("events", e.event_id()));
            }
        }
        g.check(
            &k,
            refs == episode_sources.remove(&id).unwrap_or_default(),
            "episode payload and normalized sources differ",
        );
    }
    let mut heads = BTreeMap::new();
    for r in rows(conn, "experience_candidates").await? {
        let id = string(&r, "candidate_id");
        let k = key("experience_candidates", &id);
        g.scoped(&k, &string(&r, "namespace"));
        let v = r.get::<i64, _>("current_version");
        heads.insert(id.clone(), v);
        g.edge(&k, &version_key(&id, v));
    }
    let mut sources: BTreeMap<Key, BTreeSet<String>> = BTreeMap::new();
    for r in rows(conn, "experience_candidate_sources").await? {
        let k = version_key(&string(&r, "candidate_id"), r.get("version"));
        let e = string(&r, "episode_id");
        g.edge(&k, &key("experience_episodes", &e));
        sources.entry(k).or_default().insert(e);
    }
    let mut versions: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    let mut snapshots = BTreeMap::new();
    for r in rows(conn, "experience_candidate_versions").await? {
        let id = string(&r, "candidate_id");
        let v = r.get::<i64, _>("version");
        let k = version_key(&id, v);
        let head = key("experience_candidates", &id);
        g.add(&k, None, None);
        g.edge(&k, &head);
        versions.entry(id.clone()).or_default().insert(v);
        let Some(p) = decode::<ExperienceCandidate>(&string(&r, "payload_json")) else {
            bad_json(report, "experience_candidate_versions");
            continue;
        };
        snapshots.insert((id.clone(), v), p.clone());
        g.scoped(&k, p.namespace.as_str());
        g.check(
            &k,
            p.candidate_id == id
                && p.version == v
                && p.status.as_str() == string(&r, "status")
                && p.content.kind.as_str() == string(&r, "kind")
                && p.change_kind.as_str() == string(&r, "change_kind")
                && p.recorded_at.to_rfc3339() == string(&r, "recorded_at")
                && p.rollback_target_version == r.get::<Option<i64>, _>("rollback_target_version")
                && p.content.validate().is_ok(),
            "candidate version payload does not match row or content contract",
        );
        g.check(
            &k,
            p.previous_version == if v > 1 { Some(v - 1) } else { None },
            "candidate previous version is not contiguous",
        );
        g.check(
            &k,
            (v == 1) == (p.change_kind.as_str() == "created"),
            "candidate creation/version mismatch",
        );
        let rollback = p.rollback_target_version;
        g.check(
            &k,
            (p.change_kind.as_str() == "rolled_back") == rollback.is_some()
                && rollback.is_none_or(|t| t > 0 && t < v - 1),
            "invalid candidate rollback history",
        );
        if let Some(t) = rollback {
            g.edge(&k, &version_key(&id, t));
        }
        for e in &p.content.source_episode_ids {
            g.edge(&k, &key("experience_episodes", e));
        }
        g.check(
            &k,
            set(p.content.source_episode_ids) == sources.remove(&k).unwrap_or_default(),
            "candidate payload and normalized sources differ",
        );
    }
    for ((id, version), snapshot) in &snapshots {
        let previous = snapshots.get(&(id.clone(), version - 1));
        let valid = match snapshot.change_kind.as_str() {
            "created" => snapshot.status.as_str() == "pending",
            "status_updated" => previous.is_some_and(|p| {
                p.content == snapshot.content
                    && p.status.validate_transition(snapshot.status).is_ok()
            }),
            "revised" => previous.is_some() && snapshot.status.as_str() == "pending",
            "rolled_back" => {
                snapshot.status.as_str() == "pending"
                    && snapshot
                        .rollback_target_version
                        .and_then(|v| snapshots.get(&(id.clone(), v)))
                        .is_some_and(|p| p.content == snapshot.content)
            }
            _ => false,
        };
        g.check(
            &version_key(id, *version),
            valid,
            "candidate version change does not match its predecessor/rollback source",
        );
    }
    for (id, head) in heads {
        let vs = versions.remove(&id).unwrap_or_default();
        g.check(
            &key("experience_candidates", &id),
            vs.len() as i64 == head && vs.first() == Some(&1) && vs.last() == Some(&head),
            "candidate history is incomplete or current version is not its latest version",
        );
    }
    Ok(())
}

async fn feedback(
    conn: &mut SqliteConnection,
    g: &mut Graph,
    report: &mut AdoptionInventory,
) -> Result<(), AppError> {
    for r in rows(conn, "feedback_candidates").await? {
        let id = string(&r, "candidate_id");
        let ns = string(&r, "namespace");
        let k = key("feedback_candidates", &id);
        g.scoped(&k, &ns);
        g.edge(&k, &key("claims", &string(&r, "target_claim_id")));
        let Some(p) = decode::<FeedbackCandidate>(&string(&r, "candidate_json")) else {
            bad_json(report, "feedback_candidates");
            continue;
        };
        g.scoped(&k, p.proposal.namespace.as_str());
        g.add(
            &k,
            Some(p.proposal.replacement_claim.namespace().as_str()),
            Some(match p.proposal.replacement_claim.owner() {
                crate::domain::types::Owner::World => "world",
                crate::domain::types::Owner::Self_ => "self",
                crate::domain::types::Owner::User => "user",
                _ => "unknown",
            }),
        );
        g.check(
            &k,
            p.candidate_id == id
                && p.proposal.namespace.as_str() == ns
                && p.proposal.target_claim_reference.claim_id() == string(&r, "target_claim_id")
                && p.proposal.expected_target_version == string(&r, "expected_target_version")
                && p.state.as_str() == string(&r, "state")
                && i64::try_from(p.revision).ok() == Some(r.get("revision")),
            "feedback candidate payload does not match row",
        );
        let committed = p.state.as_str() == "committed";
        let rejected = p.state.as_str() == "rejected";
        g.check(
            &k,
            if committed {
                p.revision >= 2 && p.reflection_id.is_some() && p.replacement_claim_id.is_some()
            } else {
                p.reflection_id.is_none() && p.replacement_claim_id.is_none()
            },
            "feedback lifecycle has inconsistent commit references",
        );
        g.check(
            &k,
            (p.state.as_str() == "proposed") == (p.revision == 0)
                && p.updated_at >= p.created_at
                && rejected == p.rejection_reason.is_some(),
            "feedback lifecycle has inconsistent revision, timestamp or rejection",
        );
        g.check(
            &k,
            match p.state.as_str() {
                "validated" | "committed" => p.validation.passed && p.validation.reasons.is_empty(),
                "proposed" | "blocked" => !p.validation.passed,
                _ => true,
            },
            "feedback lifecycle has inconsistent validation",
        );
        g.edge(
            &k,
            &key("claims", p.proposal.target_claim_reference.claim_id()),
        );
        for e in &p.proposal.evidence_event_ids {
            g.edge(&k, &key("events", e.event_id()));
        }
        if let Some(id) = p.reflection_id {
            g.edge(&k, &key("reflections", &id));
            let reflection=sqlx::query("SELECT superseded_claim_id,replacement_claim_id,supporting_evidence_event_ids FROM reflections WHERE reflection_id=?").bind(&id).fetch_optional(&mut *conn).await.map_err(db)?;
            g.check(
                &k,
                reflection.is_some_and(|r| {
                    optional(&r, "superseded_claim_id").as_deref()
                        == Some(p.proposal.target_claim_reference.claim_id())
                        && optional(&r, "replacement_claim_id") == p.replacement_claim_id
                        && strings(&string(&r, "supporting_evidence_event_ids")).map(set)
                            == Some(set(p
                                .proposal
                                .evidence_event_ids
                                .iter()
                                .map(|e| e.event_id().to_owned())))
                }),
                "feedback commit references differ from persisted reflection",
            );
        }
        if let Some(id) = p.replacement_claim_id {
            g.edge(&k, &key("claims", &id));
        }
    }
    Ok(())
}
async fn triggers(
    conn: &mut SqliteConnection,
    g: &mut Graph,
    report: &mut AdoptionInventory,
) -> Result<(), AppError> {
    for r in rows(conn, "reflection_trigger_ledger").await? {
        let k = key("reflection_trigger_ledger", &string(&r, "ledger_id"));
        g.scoped(&k, &string(&r, "namespace"));
        let kind = string(&r, "trigger_type");
        g.check(
            &k,
            matches!(kind.as_str(), "conflict" | "failure" | "periodic")
                && matches!(
                    string(&r, "status").as_str(),
                    "pending" | "handled" | "rejected" | "suppressed"
                )
                && string(&r, "trigger_key") == format!("{}:{kind}", string(&r, "namespace")),
            "unsupported trigger type/status or inconsistent scope key",
        );
        let Some(events) = strings(&string(&r, "evidence_window")) else {
            bad_json(report, "reflection_trigger_ledger");
            continue;
        };
        for e in events {
            g.edge(&k, &key("events", &e));
        }
        if let Some(id) = optional(&r, "reflection_id") {
            g.edge(&k, &key("reflections", &id));
        }
    }
    Ok(())
}
