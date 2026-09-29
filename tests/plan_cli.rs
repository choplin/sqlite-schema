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
fn creates_a_reviewable_reproducible_plan_without_mutating_the_target() {
    let files = TestFiles::new("success");
    Connection::open(files.database()).expect("empty target database should be created");
    fs::write(
        files.schema(),
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);\n",
    )
    .expect("desired schema should be written");
    let original_target = fs::read(files.database()).expect("target should be readable");

    let first = run_plan(
        files.database(),
        files.schema(),
        files.directory().join("plan.json"),
    );
    assert!(
        first.status.success(),
        "plan command failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let first_plan_path = files.directory().join("plan.json");
    let first_json = fs::read(&first_plan_path).expect("plan should be saved");
    let plan: Value = serde_json::from_slice(&first_json).expect("plan should be valid JSON");
    assert_eq!(plan["format_version"], 1);
    assert!(
        plan["sqlite_version"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(plan["source_fingerprint"].as_str().map(str::len), Some(64));
    assert_eq!(plan["desired_fingerprint"].as_str().map(str::len), Some(64));
    assert_eq!(plan["operations"].as_array().map(Vec::len), Some(1));
    let operation = &plan["operations"][0];
    assert_eq!(operation["operation"], "create_table");
    assert_eq!(operation["table"], "users");
    assert_eq!(operation["data_effect"], "preserving");
    assert_eq!(operation["structural_cost"], "direct");
    assert_eq!(operation["data_dependent"], false);
    assert_eq!(
        operation["sql"],
        "create table users ( id integer primary key , email text not null );"
    );

    let stdout = String::from_utf8(first.stdout).expect("summary should be UTF-8");
    assert!(stdout.contains("create_table table users"));
    assert!(stdout.contains("data effect: preserving"));
    assert!(stdout.contains("structural cost: direct"));
    assert!(stdout.contains("data dependent: false"));
    assert!(stdout.contains(&format!("Saved plan: {}", first_plan_path.display())));
    assert_eq!(
        fs::read(files.database()).expect("target should remain readable"),
        original_target,
        "planning must not mutate the target database"
    );

    fs::write(
        files.schema(),
        "create /* formatting */ table \"users\" (\n  \"id\" integer primary key,\n  email text not null\n);",
    )
    .expect("equivalent desired schema should be written");
    let second_plan_path = files.directory().join("second-plan.json");
    let second = run_plan(files.database(), files.schema(), &second_plan_path);
    assert!(second.status.success());
    assert_eq!(
        fs::read(second_plan_path).expect("second plan should be saved"),
        first_json,
        "equivalent inputs should produce identical plan artifacts"
    );
}

#[test]
fn rejects_output_aliases_without_mutating_the_target() {
    let direct = TestFiles::new("direct-output-alias");
    Connection::open(direct.database()).expect("empty target database should be created");
    fs::write(direct.schema(), "CREATE TABLE users (id INTEGER);")
        .expect("desired schema should be written");
    let original = fs::read(direct.database()).expect("target should be readable");

    let result = run_plan(direct.database(), direct.schema(), direct.database());

    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("refusing to overwrite existing plan destination")
    );
    assert_eq!(
        fs::read(direct.database()).expect("target should remain readable"),
        original
    );

    let hard_link = TestFiles::new("hard-link-output-alias");
    Connection::open(hard_link.database()).expect("empty target database should be created");
    fs::write(hard_link.schema(), "CREATE TABLE users (id INTEGER);")
        .expect("desired schema should be written");
    let linked_output = hard_link.directory().join("linked-plan.json");
    fs::hard_link(hard_link.database(), &linked_output).expect("hard link should be created");
    let original = fs::read(hard_link.database()).expect("target should be readable");

    let result = run_plan(hard_link.database(), hard_link.schema(), &linked_output);

    assert!(!result.status.success());
    assert_eq!(
        fs::read(hard_link.database()).expect("target should remain readable"),
        original
    );
}

#[cfg(unix)]
#[test]
fn rejects_a_symbolic_link_output_alias_without_mutating_the_target() {
    use std::os::unix::fs::symlink;

    let files = TestFiles::new("symbolic-link-output-alias");
    Connection::open(files.database()).expect("empty target database should be created");
    fs::write(files.schema(), "CREATE TABLE users (id INTEGER);")
        .expect("desired schema should be written");
    let linked_output = files.directory().join("linked-plan.json");
    symlink(files.database(), &linked_output).expect("symbolic link should be created");
    let original = fs::read(files.database()).expect("target should be readable");

    let result = run_plan(files.database(), files.schema(), &linked_output);

    assert!(!result.status.success());
    assert_eq!(
        fs::read(files.database()).expect("target should remain readable"),
        original
    );
}

#[test]
fn rejects_no_op_and_unsupported_differences_without_writing_a_plan() {
    let no_op = TestFiles::new("no-op");
    let schema_sql = "CREATE TABLE users (id INTEGER);";
    Connection::open(no_op.database())
        .and_then(|connection| connection.execute_batch(schema_sql))
        .expect("target schema should be created");
    fs::write(
        no_op.schema(),
        "create /* same schema */ table \"users\" ( \"id\" integer );",
    )
    .expect("equivalent desired schema should be written");
    let no_op_output = no_op.directory().join("plan.json");

    let result = run_plan(no_op.database(), no_op.schema(), &no_op_output);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("current and desired schemas are identical")
    );
    assert!(!no_op_output.exists());

    let replacement = TestFiles::new("replacement");
    Connection::open(replacement.database())
        .and_then(|connection| connection.execute_batch(schema_sql))
        .expect("target schema should be created");
    fs::write(replacement.schema(), "CREATE TABLE accounts (id INTEGER);")
        .expect("desired schema should be written");
    let replacement_output = replacement.directory().join("plan.json");

    let result = run_plan(
        replacement.database(),
        replacement.schema(),
        &replacement_output,
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported schema difference"));
    assert!(!replacement_output.exists());
}

#[test]
fn rejects_unsupported_desired_shapes_without_writing_a_plan() {
    let files = TestFiles::new("unsupported-shape");
    Connection::open(files.database()).expect("empty target database should be created");
    fs::write(
        files.schema(),
        "CREATE TABLE users (id INTEGER); CREATE TABLE accounts (id INTEGER);",
    )
    .expect("desired schema should be written");
    let output = files.directory().join("plan.json");

    let result = run_plan(files.database(), files.schema(), &output);

    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported schema"));
    assert!(!output.exists());
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
        .expect("sqlite-schema command should run")
}

struct TestFiles {
    directory: PathBuf,
}

impl TestFiles {
    fn new(name: &str) -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "sqlite-schema-plan-{name}-{}-{sequence}",
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
