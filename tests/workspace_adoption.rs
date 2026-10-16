#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;
#[cfg(any(unix, windows))]
use agent_llm_mm::support::workspace_registry::{adopt_workspace, resolve_workspace};
// Synthetic databases only; adoption must never rewrite historical ledger data.
use agent_llm_mm::{
    adapters::sqlite::initialize_database,
    support::workspace_registry::{WorkspaceRegistry, preview_adoption, register_workspace},
};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{fs, path::PathBuf};
struct Fixture {
    _dir: tempfile::TempDir,
    registry: PathBuf,
    root: PathBuf,
    db: PathBuf,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        windows_private::private_directory(dir.path());
        let base = dir.path().canonicalize().unwrap();
        let root = base.join("project");
        fs::create_dir(&root).unwrap();
        let db = base.join("synthetic.sqlite");
        initialize_database(&agent_llm_mm::support::sqlite_url::sqlite_file_url(&db))
            .await
            .unwrap();
        let result = Self {
            _dir: dir,
            registry: base.join("registry.json"),
            root,
            db,
        };
        result.sql(r#"INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('e','2026-10-01T00:00:00Z','world','project/legacy','conversation','old fact'); INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES('c','world','project/legacy','s','p','o','said','active'); INSERT INTO evidence_links VALUES('c','e'); INSERT INTO episode_events VALUES('episode:old','e'); INSERT INTO operation_log(operation_id,occurred_at,namespace,actor_kind,actor_id,entrypoint,operation_kind,status,request_summary_json,response_summary_json) VALUES('old-receipt','2026-10-01T00:00:00Z','project/legacy','system','durable_write_receipt_v1','ingest_interaction','tool','ok','{"request_hash":"original"}','{"event_id":"e","opaque":"retained"}');"#).await;
        result
    }
    async fn connection(&self) -> SqliteConnection {
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&self.db))
            .await
            .unwrap()
    }
    async fn sql(&self, sql: &str) {
        let mut c = self.connection().await;
        sqlx::raw_sql(sql).execute(&mut c).await.unwrap();
        c.close().await.unwrap();
    }
    async fn preview(&self) -> agent_llm_mm::support::workspace_registry::AdoptionReport {
        preview_adoption(&self.registry, &self.root, "project/legacy", &self.db)
            .await
            .unwrap()
    }
}
#[tokio::test]
async fn preview_is_read_only_and_reports_supported_counts() {
    let f = Fixture::new().await;
    let bytes = fs::read(&f.db).unwrap();
    let before = fs::read_dir(f.db.parent().unwrap()).unwrap().count();
    let report = f.preview().await;
    assert!(
        report.inventory.supported,
        "{:?}",
        report.inventory.blocked_reasons
    );
    assert_eq!(report.inventory.namespace_counts["events"], 1);
    assert_eq!(report.inventory.namespace_counts["claims"], 1);
    assert_eq!(
        report.inventory.namespace_counts["durable_write_receipts"],
        1
    );
    assert!(!f.registry.exists());
    assert_eq!(fs::read(&f.db).unwrap(), bytes);
    assert_eq!(
        fs::read_dir(f.db.parent().unwrap()).unwrap().count(),
        before
    );
}
#[cfg(any(unix, windows))]
#[tokio::test]
async fn adoption_adds_v2_mapping_without_rewriting_facts_or_receipts() {
    let f = Fixture::new().await;
    let other = f.root.with_file_name("other");
    fs::create_dir(&other).unwrap();
    let original = register_workspace(&f.registry, &other).unwrap();
    let bytes = fs::read(&f.db).unwrap();
    let report = adopt_workspace(&f.registry, &f.root, "project/legacy", &f.db)
        .await
        .unwrap();
    let workspace = report.workspace.unwrap();
    assert_ne!(workspace.id, original.id);
    assert_eq!(workspace.project_namespace, "project/legacy");
    assert_eq!(
        resolve_workspace(&f.registry, &f.root)
            .unwrap()
            .project_namespace,
        "project/legacy"
    );
    let registry = WorkspaceRegistry::load(&f.registry).unwrap();
    assert_eq!(registry.version, 2);
    assert_eq!(registry.workspaces[0], original);
    assert_eq!(fs::read(&f.db).unwrap(), bytes);
    let registry_bytes = fs::read(&f.registry).unwrap();
    assert!(
        adopt_workspace(&f.registry, &f.root, "project/legacy", &f.db)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&f.registry).unwrap(), registry_bytes);
    assert!(
        adopt_workspace(&f.registry, &other, "project/legacy", &f.db)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn unsupported_histories_and_unprovable_incoming_references_are_explicit() {
    let f = Fixture::new().await;
    f.sql("INSERT INTO reflections(reflection_id,recorded_at,summary) VALUES('foreign-history','2026-10-01T00:00:00Z','unscoped history')").await;
    let report = f.preview().await;
    assert!(!report.inventory.supported);
    assert!(
        report
            .inventory
            .blocked_reasons
            .iter()
            .any(|r| r.contains("unsupported history reflections") && r.contains("incoming"))
    );
    assert_eq!(
        report
            .inventory
            .database_counts
            .iter()
            .find(|r| r.table == "reflections")
            .unwrap()
            .rows,
        1
    );
    assert!(!f.registry.exists());
}
#[tokio::test]
async fn unknown_owner_and_cross_namespace_sources_fail_closed() {
    for sql in [
        "INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('x','2026-10-01T00:00:00Z','unknown','project/legacy','conversation','unknown')",
        "INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('x','2026-10-01T00:00:00Z','world','project/foreign','conversation','foreign'); INSERT INTO evidence_links VALUES('c','x')",
        "INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES('x','world','project/foreign','s','p','o','said','active'); INSERT INTO evidence_links VALUES('x','e')",
        "INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('x','2026-10-01T00:00:00Z','world','project/foreign','conversation','foreign'); INSERT INTO episode_events VALUES('episode:old','x')",
    ] {
        let f = Fixture::new().await;
        f.sql(sql).await;
        assert!(!f.preview().await.inventory.supported, "accepted {sql}");
    }
}
#[tokio::test]
async fn feedback_provenance_is_checked_in_both_directions_and_missing_sources_block() {
    for (origin, target) in [
        ("project/legacy", "event:missing"),
        ("project/foreign", "event:e"),
    ] {
        let f = Fixture::new().await;
        let metadata = serde_json::json!({"source_kind":"caller_reported","producer":"synthetic","observed_target":target,"expected":"a","actual":"b","verification_method":"test","verification_result":"inconclusive","limitations":[],"evidence_refs":[]});
        let mut c = f.connection().await;
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary,feedback_json) VALUES('feedback','2026-10-01T00:00:00Z','world',?,'conversation','feedback',?)").bind(origin).bind(metadata.to_string()).execute(&mut c).await.unwrap();
        c.close().await.unwrap();
        assert!(!f.preview().await.inventory.supported);
    }
}
#[tokio::test]
async fn namespace_validation_and_registry_v2_uniqueness_fail_closed() {
    let f = Fixture::new().await;
    for ns in [
        "world",
        "self",
        "user/personal",
        "project/",
        "project/\nlegacy",
        "project/legacy name",
        "project/ legacy",
        "project/legacy\u{2003}name",
        "project/00000000-0000-0000-0000-000000000001",
    ] {
        assert!(
            preview_adoption(&f.registry, &f.root, ns, &f.db)
                .await
                .is_err()
        );
    }
    assert!(
        !preview_adoption(&f.registry, &f.root, "project/empty", &f.db)
            .await
            .unwrap()
            .inventory
            .supported
    );
    let w = register_workspace(&f.registry, &f.root).unwrap();
    let mut registry = WorkspaceRegistry::load(&f.registry).unwrap();
    registry.version = 2;
    registry.workspaces[0].project_namespace = "project/legacy".into();
    let mut duplicate = w;
    duplicate.id = uuid::Uuid::new_v4().to_string();
    duplicate.roots.clear();
    duplicate.project_namespace = "project/legacy".into();
    registry.workspaces.push(duplicate);
    assert!(registry.validate().is_err());
    registry.workspaces.pop();
    registry.version = 1;
    assert!(registry.validate().is_err());
    registry.version = 2;
    assert!(registry.validate().is_ok());
}
#[cfg(any(unix, windows))]
#[tokio::test]
async fn execution_rechecks_preview_and_refuses_core_or_sqlite_writer_lock() {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let f = Fixture::new().await;
    assert!(f.preview().await.inventory.supported);
    let path = PathBuf::from(format!("{}.local-core.lock", f.db.display()));
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options.open(&path).unwrap();
    lock.try_lock().unwrap();
    assert!(
        adopt_workspace(&f.registry, &f.root, "project/legacy", &f.db)
            .await
            .is_err()
    );
    assert!(!f.registry.exists());
    lock.unlock().unwrap();
    drop(lock);
    let mut c = f.connection().await;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut c)
        .await
        .unwrap();
    assert!(
        adopt_workspace(&f.registry, &f.root, "project/legacy", &f.db)
            .await
            .is_err()
    );
    assert!(!f.registry.exists());
    sqlx::query("ROLLBACK").execute(&mut c).await.unwrap();
    c.close().await.unwrap();
    f.sql("INSERT INTO reflections(reflection_id,recorded_at,summary) VALUES('late','2026-10-01T00:00:00Z','changed after preview')").await;
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
async fn preview_reports_schema_errors_and_registry_conflicts_without_modifying_either() {
    let f = Fixture::new().await;
    register_workspace(&f.registry, &f.root).unwrap();
    let registry = fs::read(&f.registry).unwrap();
    let preview = f.preview().await;
    assert!(!preview.inventory.supported);
    assert!(
        preview
            .inventory
            .blocked_reasons
            .iter()
            .any(|r| r.contains("already registered"))
    );
    assert_eq!(fs::read(&f.registry).unwrap(), registry);
    f.sql("PRAGMA user_version=999").await;
    let bytes = fs::read(&f.db).unwrap();
    let preview = f.preview().await;
    assert!(!preview.inventory.supported);
    assert_eq!(preview.inventory.schema_status, "unsupported_newer_schema");
    assert_eq!(fs::read(&f.db).unwrap(), bytes);
}

#[test]
fn adoption_cli_has_an_explicit_read_only_preview_and_rejects_extra_flags() {
    use agent_llm_mm::interfaces::local_core::cli::{LocalCommand, WorkspaceAction, parse_args};
    // Parsing performs no I/O; derive platform-native absolute fixture paths.
    let directory = std::env::current_dir()
        .unwrap()
        .join("synthetic-adoption-cli");
    let registry = directory.join("registry.json").to_str().unwrap().to_owned();
    let root = directory.join("root").to_str().unwrap().to_owned();
    let database = directory.join("db.sqlite").to_str().unwrap().to_owned();
    let base = [
        "agent_llm_mm",
        "workspace",
        "--registry",
        &registry,
        "adopt-preview",
        "--root",
        &root,
        "--namespace",
        "project/legacy",
        "--database",
        &database,
    ];
    let parsed = parse_args(base.iter().map(|s| s.to_string()))
        .unwrap()
        .unwrap();
    assert!(matches!(
        parsed,
        LocalCommand::Workspace {
            action: WorkspaceAction::Adopt { preview: true, .. },
            ..
        }
    ));
    for index in [3, 6, 10] {
        let mut relative = base.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        relative[index] = "relative-path".into();
        let error = parse_args(relative).unwrap_err();
        assert!(error.to_string().contains("paths must be absolute"));
    }
    let mut bad = base.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    bad.extend(["--id".into(), uuid::Uuid::new_v4().to_string()]);
    assert!(parse_args(bad).is_err());
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn failed_registry_publication_keeps_the_database_and_existing_registry_unchanged() {
    let f = Fixture::new().await;
    let other = f.root.with_file_name("other");
    fs::create_dir(&other).unwrap();
    register_workspace(&f.registry, &other).unwrap();
    let before = fs::read(&f.db).unwrap();
    let registry = fs::read(&f.registry).unwrap();
    let lock = PathBuf::from(format!("{}.lock", f.registry.display()));
    fs::write(&lock, "synthetic lock owned by another operation").unwrap();
    assert!(
        adopt_workspace(&f.registry, &f.root, "project/legacy", &f.db)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&f.db).unwrap(), before);
    assert_eq!(fs::read(&f.registry).unwrap(), registry);
    assert!(lock.exists());
    let mut c = f.connection().await;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut c)
        .await
        .unwrap();
    sqlx::query("ROLLBACK").execute(&mut c).await.unwrap();
}

#[tokio::test]
async fn wal_preview_reads_uncheckpointed_facts_without_mutating_ledger_or_registry() {
    let f = Fixture::new().await;
    let mut writer = f.connection().await;
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&mut writer)
        .await
        .unwrap();
    sqlx::query("PRAGMA wal_autocheckpoint=0")
        .execute(&mut writer)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('wal-event','2026-10-01T00:00:00Z','world','project/legacy','conversation','uncheckpointed')").execute(&mut writer).await.unwrap();
    let wal = PathBuf::from(format!("{}-wal", f.db.display()));
    let db_bytes = fs::read(&f.db).unwrap();
    let wal_bytes = fs::read(&wal).unwrap();
    assert!(!wal_bytes.is_empty());
    let report = f.preview().await;
    assert!(
        report.inventory.supported,
        "{:?}",
        report.inventory.blocked_reasons
    );
    assert_eq!(report.inventory.namespace_counts["events"], 2);
    assert_eq!(fs::read(&f.db).unwrap(), db_bytes);
    assert_eq!(fs::read(&wal).unwrap(), wal_bytes);
    assert!(!f.registry.exists());
    assert!(!PathBuf::from(format!("{}.local-core.lock", f.db.display())).exists());
    writer.close().await.unwrap();
    // A read-only SQLite open may create WAL/SHM sidecars after a clean close.
    // The promise concerns ledger records and registry, not directory entry count.
    let bytes = fs::read(&f.db).unwrap();
    assert_eq!(f.preview().await.inventory.namespace_counts["events"], 2);
    assert_eq!(fs::read(&f.db).unwrap(), bytes);
    assert!(!f.registry.exists());
}

#[path = "workspace_adoption/history.rs"]
mod history;

#[tokio::test]
async fn task_history_is_explicitly_blocked_even_when_not_in_requested_legacy_namespace() {
    let f = Fixture::new().await;
    let store = agent_llm_mm::adapters::sqlite::open_current_database(
        &agent_llm_mm::support::sqlite_url::sqlite_file_url(&f.db),
    )
    .await
    .unwrap();
    let request=serde_json::from_value(serde_json::json!({
        "namespace":"project/other","task_id":"task-one","request_id":"task-create","title":"Synthetic task","reason":"Test unsupported task adoption",
        "source":{"kind":"caller_reported","reference":null},
        "requirements":[{"requirement_id":"goal","kind":"goal","text":"Retain task history","state":"reported","source":{"kind":"caller_reported","reference":null},"replaces":null}]
    })).unwrap();
    agent_llm_mm::application::task::create_task(&store, request)
        .await
        .unwrap();
    let report = f.preview().await;
    assert!(!report.inventory.supported);
    assert!(
        report
            .inventory
            .blocked_reasons
            .iter()
            .any(|r| r.contains("workspace task history"))
    );
    assert!(!f.registry.exists());
}
