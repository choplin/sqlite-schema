use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use rusqlite::Connection;
use serde_json::Value;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn applies_a_reviewed_create_table_plan() -> rusqlite::Result<()> {
    let files = TestFiles::new("success");
    Connection::open(files.database())?;
    let plan = create_plan(&files, "CREATE TABLE users (id INTEGER PRIMARY KEY);");

    let result = run_apply(files.database(), &plan);

    assert!(
        result.status.success(),
        "apply command failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("Applied plan"));
    let database = Connection::open(files.database())?;
    let table_count: i64 = database.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'users'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(table_count, 1);

    Ok(())
}

#[test]
fn rejects_schema_drift_without_mutating_the_target() -> rusqlite::Result<()> {
    let files = TestFiles::new("drift");
    Connection::open(files.database())?;
    let plan = create_plan(&files, "CREATE TABLE users (id INTEGER);");
    let database = Connection::open(files.database())?;
    database.execute_batch(
        "CREATE TABLE accounts (id INTEGER PRIMARY KEY);
         INSERT INTO accounts (id) VALUES (7);",
    )?;
    drop(database);

    let result = run_apply(files.database(), &plan);

    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("target schema drifted"));
    let database = Connection::open(files.database())?;
    let account_id: i64 = database.query_row("SELECT id FROM accounts", [], |row| row.get(0))?;
    assert_eq!(account_id, 7);
    let users_count: i64 = database.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'users'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(users_count, 0);

    Ok(())
}

#[test]
fn rejects_an_unsupported_plan_version_without_mutating_the_target() {
    let files = TestFiles::new("version");
    Connection::open(files.database()).expect("empty target database should be created");
    let plan_path = create_plan(&files, "CREATE TABLE users (id INTEGER);");
    let mut plan: Value =
        serde_json::from_slice(&fs::read(&plan_path).expect("generated plan should be readable"))
            .expect("generated plan should be valid JSON");
    plan["format_version"] = Value::from(2);
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&plan).expect("modified plan should serialize"),
    )
    .expect("modified plan should be written");
    let before = fs::read(files.database()).expect("target should be readable");

    let result = run_apply(files.database(), &plan_path);

    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("unsupported migration plan format version 2")
    );
    assert_eq!(
        fs::read(files.database()).expect("target should remain readable"),
        before
    );
}

#[test]
fn rejects_tampered_operation_metadata_without_mutating_the_target() {
    let files = TestFiles::new("tampered");
    Connection::open(files.database()).expect("empty target database should be created");
    let plan_path = create_plan(&files, "CREATE TABLE users (id INTEGER);");
    let mut plan: Value =
        serde_json::from_slice(&fs::read(&plan_path).expect("generated plan should be readable"))
            .expect("generated plan should be valid JSON");
    plan["operations"][0]["table"] = Value::from("accounts");
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&plan).expect("modified plan should serialize"),
    )
    .expect("modified plan should be written");
    let before = fs::read(files.database()).expect("target should be readable");

    let result = run_apply(files.database(), &plan_path);

    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid migration plan"));
    assert_eq!(
        fs::read(files.database()).expect("target should remain readable"),
        before
    );
}

fn create_plan(files: &TestFiles, schema_sql: &str) -> PathBuf {
    fs::write(files.schema(), schema_sql).expect("desired schema should be written");
    let plan = files.directory().join("plan.json");
    let result = run_plan(files.database(), files.schema(), &plan);
    assert!(
        result.status.success(),
        "plan command failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    plan
}

fn run_plan(
    database: impl AsRef<Path>,
    schema: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sqlite-schema"))
        .arg("plan")
        .arg(database.as_ref())
        .arg("--file")
        .arg(schema.as_ref())
        .arg("--output")
        .arg(output.as_ref())
        .output()
        .expect("sqlite-schema plan command should run")
}

fn run_apply(database: impl AsRef<Path>, plan: impl AsRef<Path>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sqlite-schema"))
        .arg("apply")
        .arg(database.as_ref())
        .arg("--plan")
        .arg(plan.as_ref())
        .output()
        .expect("sqlite-schema apply command should run")
}

struct TestFiles {
    directory: PathBuf,
}

impl TestFiles {
    fn new(name: &str) -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "sqlite-schema-apply-{name}-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).expect("test directory should be created");
        Self { directory }
    }

    fn directory(&self) -> &Path {
        &self.directory
    }

    fn database(&self) -> PathBuf {
        self.directory.join("target.db")
    }

    fn schema(&self) -> PathBuf {
        self.directory.join("schema.sql")
    }
}

impl Drop for TestFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
