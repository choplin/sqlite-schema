use std::{
    fs,
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use sqlite_schema::{InspectError, SchemaSource, inspect_schema};

const SUPPORTED_SCHEMA: &str = "
    CREATE TABLE users (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        email TEXT NOT NULL UNIQUE,
        enabled INTEGER NOT NULL DEFAULT 1
    );
";

#[test]
fn inspects_database_and_sql_sources_with_one_model() -> rusqlite::Result<()> {
    let target_file = TestDatabase::new("symmetric");
    let writable_target = Connection::open(target_file.path())?;
    writable_target.execute_batch(SUPPORTED_SCHEMA)?;
    writable_target.execute("INSERT INTO users (email) VALUES ('user@example.com')", [])?;
    drop(writable_target);

    let current = inspect_schema(SchemaSource::Database(target_file.path()))
        .expect("supported database schema should inspect");
    let desired = inspect_schema(SchemaSource::Sql(SUPPORTED_SCHEMA))
        .expect("supported SQL schema should inspect");

    assert_eq!(current, desired);
    for (current_source, desired_source) in [
        (
            SchemaSource::Sql(SUPPORTED_SCHEMA),
            SchemaSource::Sql(SUPPORTED_SCHEMA),
        ),
        (
            SchemaSource::Sql(SUPPORTED_SCHEMA),
            SchemaSource::Database(target_file.path()),
        ),
        (
            SchemaSource::Database(target_file.path()),
            SchemaSource::Sql(SUPPORTED_SCHEMA),
        ),
        (
            SchemaSource::Database(target_file.path()),
            SchemaSource::Database(target_file.path()),
        ),
    ] {
        let current = inspect_schema(current_source).expect("current source should inspect");
        let desired = inspect_schema(desired_source).expect("desired source should inspect");
        assert_eq!(current, desired);
    }
    assert_eq!(current.fingerprint().to_string().len(), 64);

    let table = current
        .table()
        .expect("one user table should be represented");
    assert_eq!(table.name(), "users");
    assert_eq!(
        table.create_sql(),
        SUPPORTED_SCHEMA.trim().trim_end_matches(';')
    );
    assert_eq!(table.columns().len(), 3);
    assert_eq!(table.columns()[0].name(), "id");
    assert_eq!(table.columns()[0].declared_type(), "INTEGER");
    assert_eq!(table.columns()[0].primary_key_position(), 1);
    assert_eq!(table.columns()[1].name(), "email");
    assert!(table.columns()[1].is_not_null());
    assert_eq!(table.columns()[2].default_sql(), Some("1"));

    Ok(())
}

#[test]
fn fingerprints_are_stable_across_independent_databases() {
    let first =
        inspect_schema(SchemaSource::Sql(SUPPORTED_SCHEMA)).expect("first schema should inspect");
    let second =
        inspect_schema(SchemaSource::Sql(SUPPORTED_SCHEMA)).expect("second schema should inspect");

    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(
        first.fingerprint().as_bytes(),
        second.fingerprint().as_bytes()
    );
}

#[test]
fn represents_empty_schemas_deterministically() {
    let first = inspect_schema(SchemaSource::Sql("")).expect("first empty schema should inspect");
    let second = inspect_schema(SchemaSource::Sql("")).expect("second empty schema should inspect");

    assert!(first.table().is_none());
    assert_eq!(first, second);
}

#[test]
fn does_not_confuse_user_names_with_sqlite_internal_names() {
    let model = inspect_schema(SchemaSource::Sql("CREATE TABLE sqliteXusers (id INTEGER);"))
        .expect("user table should inspect");

    assert_eq!(
        model.table().map(|table| table.name()),
        Some("sqliteXusers")
    );
}

#[test]
fn rejects_attached_databases_in_schema_sql() {
    for schema_sql in [
        "ATTACH ':memory:' AS aux;",
        "ATTACH ':memory:' AS aux; CREATE TABLE aux.hidden (id INTEGER);",
    ] {
        let error = inspect_schema(SchemaSource::Sql(schema_sql))
            .expect_err("attached database should be rejected explicitly");
        assert!(matches!(error, InspectError::Input { .. }));
        assert!(error.to_string().contains("not authorized"));
    }
}

#[test]
fn rejects_every_schema_shape_outside_the_minimal_model() {
    for schema_sql in [
        "CREATE TABLE first (id INTEGER); CREATE TABLE second (id INTEGER);",
        "CREATE TABLE users (id INTEGER); CREATE INDEX users_id ON users (id);",
        "CREATE VIEW users AS SELECT 1 AS id;",
        "CREATE TEMP TABLE users (id INTEGER);",
        "CREATE TEMP TABLE sqliteXusers (id INTEGER);",
        "CREATE TABLE users (id INTEGER) STRICT;",
        "CREATE TABLE users (id INTEGER PRIMARY KEY) WITHOUT ROWID;",
        "CREATE TABLE users (id INTEGER, doubled INTEGER GENERATED ALWAYS AS (id * 2));",
    ] {
        let error = inspect_schema(SchemaSource::Sql(schema_sql))
            .expect_err("unsupported schema shape should be rejected");
        assert!(
            matches!(error, InspectError::Unsupported { .. }),
            "unexpected inspection error for {schema_sql}: {error}"
        );
    }
}

#[test]
fn missing_database_source_is_not_created() {
    let database = TestDatabase::new("missing");
    assert!(!database.path().exists());

    let error = inspect_schema(SchemaSource::Database(database.path()))
        .expect_err("missing database source must not be created");

    assert!(matches!(error, InspectError::Input { .. }));
    assert!(!database.path().exists());
}

#[test]
fn sqlite_special_names_are_not_accepted_as_database_files() {
    for path in [Path::new(""), Path::new(":memory:")] {
        let error = inspect_schema(SchemaSource::Database(path))
            .expect_err("database input must resolve to an existing file");

        assert!(matches!(error, InspectError::Input { .. }));
    }
}

struct TestDatabase {
    path: PathBuf,
}

impl TestDatabase {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sqlite-schema-inspector-{name}-{}-{}.db",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("db-journal"));

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(self.path.with_extension("db-journal"));
    }
}
