use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use sqlx::sqlite::SqlitePool;
use tempfile::tempdir;

#[tokio::test]
async fn backup_restore_to_new_path_roundtrips_sqlite_contents() {
    if !command_exists("bash") {
        return;
    }

    let temp_dir = tempdir().expect("temp dir");
    let db_path = temp_dir.path().join("live").join("agent.sqlite");
    let backup_dir = temp_dir.path().join("backups");
    let restored_path = temp_dir.path().join("restored").join("agent.sqlite");
    create_sqlite_database(&db_path).await;

    let backup_output = run_script(&[
        "scripts/backup-sqlite.sh",
        &sqlite_url(&db_path),
        backup_dir.to_str().expect("utf-8 backup dir"),
    ]);
    assert_success(&backup_output, "backup should succeed");
    let backup_path = output_path(&backup_output, "backup: ");
    assert!(
        backup_path.is_file(),
        "backup output should point to an existing file: {}",
        backup_path.display()
    );

    let restore_output = run_script(&[
        "scripts/restore-sqlite.sh",
        backup_path.to_str().expect("utf-8 backup path"),
        &sqlite_url(&restored_path),
    ]);
    assert_success(&restore_output, "restore should succeed");

    assert_eq!(query_item_count(&restored_path).await, 2);
    assert_eq!(query_item_name(&restored_path, 1).await, "alpha");
    assert_eq!(query_item_name(&restored_path, 2).await, "beta");
}

#[test]
fn restore_refuses_to_overwrite_existing_target() {
    if !command_exists("bash") {
        return;
    }

    let temp_dir = tempdir().expect("temp dir");
    let backup_path = temp_dir.path().join("backup.sqlite.bak");
    let target_path = temp_dir.path().join("target.sqlite");
    create_placeholder_file(&backup_path);
    fs::write(&target_path, b"keep me").expect("existing target");

    let output = run_script(&[
        "scripts/restore-sqlite.sh",
        backup_path.to_str().expect("utf-8 backup path"),
        target_path.to_str().expect("utf-8 target path"),
    ]);

    assert_failure(&output, "restore should reject existing target");
    assert!(
        stderr(&output).contains("refusing to overwrite"),
        "stderr should explain overwrite refusal, got: {}",
        stderr(&output)
    );
    assert_eq!(
        fs::read(&target_path).expect("target contents"),
        b"keep me",
        "restore must not modify an existing target"
    );
}

#[test]
fn backup_refuses_live_database_subdirectory() {
    if !command_exists("bash") {
        return;
    }

    let temp_dir = tempdir().expect("temp dir");
    let db_path = temp_dir.path().join("live").join("agent.sqlite");
    let backup_dir = temp_dir.path().join("live").join("backups");
    create_placeholder_file(&db_path);

    let output = run_script(&[
        "scripts/backup-sqlite.sh",
        db_path.to_str().expect("utf-8 db path"),
        backup_dir.to_str().expect("utf-8 backup dir"),
    ]);

    assert_failure(&output, "backup should reject live db subdirectory");
    assert!(
        stderr(&output).contains("outside the live database directory tree"),
        "stderr should explain live-directory refusal, got: {}",
        stderr(&output)
    );
}

#[test]
fn backup_and_restore_reject_in_memory_database() {
    if !command_exists("bash") {
        return;
    }

    let temp_dir = tempdir().expect("temp dir");
    let backup_path = temp_dir.path().join("backup.sqlite.bak");
    create_placeholder_file(&backup_path);

    for input in [":memory:", "sqlite::memory:"] {
        let backup_output = run_script(&[
            "scripts/backup-sqlite.sh",
            input,
            temp_dir.path().to_str().expect("utf-8 temp dir"),
        ]);

        assert_failure(&backup_output, "backup should reject in-memory database");
        assert!(
            stderr(&backup_output)
                .contains("in-memory SQLite databases cannot be backed up as files"),
            "stderr should explain memory db refusal for {input}, got: {}",
            stderr(&backup_output)
        );

        let restore_output = run_script(&[
            "scripts/restore-sqlite.sh",
            backup_path.to_str().expect("utf-8 backup path"),
            input,
        ]);

        assert_failure(&restore_output, "restore should reject in-memory database");
        assert!(
            stderr(&restore_output)
                .contains("in-memory SQLite databases cannot be restored as files"),
            "stderr should explain memory db refusal for {input}, got: {}",
            stderr(&restore_output)
        );
    }
}

#[test]
fn sqlite_file_urls_reject_invalid_percent_encoding() {
    if !command_exists("bash") {
        return;
    }

    let temp_dir = tempdir().expect("temp dir");
    let backup_path = temp_dir.path().join("backup.sqlite.bak");
    create_placeholder_file(&backup_path);

    let backup_output = run_script(&[
        "scripts/backup-sqlite.sh",
        "sqlite:///tmp/agent%ZZ.sqlite",
        temp_dir.path().to_str().expect("utf-8 temp dir"),
    ]);
    assert_failure(
        &backup_output,
        "backup should reject invalid percent encoding",
    );
    assert!(
        stderr(&backup_output).contains("invalid percent-encoding"),
        "backup stderr should explain invalid percent encoding, got: {}",
        stderr(&backup_output)
    );

    let restore_output = run_script(&[
        "scripts/restore-sqlite.sh",
        backup_path.to_str().expect("utf-8 backup path"),
        "sqlite:///tmp/agent%ZZ.sqlite",
    ]);
    assert_failure(
        &restore_output,
        "restore should reject invalid percent encoding",
    );
    assert!(
        stderr(&restore_output).contains("invalid percent-encoding"),
        "restore stderr should explain invalid percent encoding, got: {}",
        stderr(&restore_output)
    );
}

#[test]
fn restore_rejects_parent_directory_components_in_target_path() {
    if !command_exists("bash") {
        return;
    }

    let temp_dir = tempdir().expect("temp dir");
    let backup_path = temp_dir.path().join("backup.sqlite.bak");
    // Preserve the lexical traversal even if the temp path has a verbatim prefix.
    let mut target_path = temp_dir.path().as_os_str().to_os_string();
    let separator = std::path::MAIN_SEPARATOR;
    target_path.push(format!(
        "{separator}restore{separator}..{separator}target.sqlite"
    ));
    let target_path = PathBuf::from(target_path);
    assert!(
        target_path
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    );
    create_placeholder_file(&backup_path);

    let output = run_script(&[
        "scripts/restore-sqlite.sh",
        backup_path.to_str().expect("utf-8 backup path"),
        target_path.to_str().expect("utf-8 target path"),
    ]);

    assert_failure(
        &output,
        "restore should reject parent-directory target components",
    );
    assert!(
        stderr(&output).contains("must not contain '..' path components"),
        "stderr should explain parent-directory refusal, got: {}",
        stderr(&output)
    );
}

fn run_script(args: &[&str]) -> Output {
    let (script, rest) = args.split_first().expect("script arg");
    Command::new("bash")
        .arg(script)
        .args(rest)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("failed to run {script}: {error}"))
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

async fn create_sqlite_database(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create db parent");
    }
    fs::File::create(path).expect("create sqlite file");

    let database_url = sqlite_url(path);
    let pool = SqlitePool::connect(&database_url)
        .await
        .expect("connect sqlite fixture");
    sqlx::query("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create fixture table");
    sqlx::query("INSERT INTO items (name) VALUES ('alpha'), ('beta')")
        .execute(&pool)
        .await
        .expect("insert fixture rows");
    pool.close().await;
}

fn create_placeholder_file(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create file parent");
    }
    fs::write(path, b"placeholder").expect("create placeholder file");
}

async fn query_item_count(path: &Path) -> i64 {
    let database_url = sqlite_url(path);
    let pool = SqlitePool::connect(&database_url)
        .await
        .expect("connect restored sqlite");
    let value = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM items")
        .fetch_one(&pool)
        .await
        .expect("query restored sqlite count");
    pool.close().await;
    value
}

async fn query_item_name(path: &Path, id: i64) -> String {
    query_scalar(path, &format!("SELECT name FROM items WHERE id = {id}")).await
}

async fn query_scalar(path: &Path, sql: &str) -> String {
    let database_url = sqlite_url(path);
    let pool = SqlitePool::connect(&database_url)
        .await
        .expect("connect restored sqlite");
    let value = sqlx::query_scalar::<_, String>(sql)
        .fetch_one(&pool)
        .await
        .expect("query restored sqlite");
    pool.close().await;
    value
}

fn output_path(output: &Output, prefix: &str) -> PathBuf {
    stdout(output)
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("missing {prefix:?} line in stdout: {}", stdout(output)))
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}; status: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout(output),
        stderr(output)
    );
}

fn assert_failure(output: &Output, context: &str) {
    assert!(
        !output.status.success(),
        "{context}; command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
