use std::{
    fs,
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use sqlite_schema::SchemaDatabase;

#[test]
fn materializes_multi_statement_schema_with_bundled_sqlite() -> rusqlite::Result<()> {
    let database = SchemaDatabase::from_sql(
        "
        CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            email TEXT NOT NULL UNIQUE
        );
        CREATE TABLE audit (user_id INTEGER NOT NULL);
        CREATE INDEX users_email_index ON users (email);
        CREATE TRIGGER users_audit AFTER INSERT ON users
        BEGIN
            INSERT INTO audit (user_id) VALUES (NEW.id);
        END;
        CREATE VIEW user_emails AS SELECT email FROM users;
        ",
    )
    .expect("valid SQLite schema should load");

    let objects = database
        .connection()
        .prepare(
            "SELECT type, name FROM sqlite_schema
             WHERE name IN ('audit', 'user_emails', 'users', 'users_audit', 'users_email_index')
             ORDER BY name",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    assert_eq!(
        objects,
        [
            ("table".to_owned(), "audit".to_owned()),
            ("view".to_owned(), "user_emails".to_owned()),
            ("table".to_owned(), "users".to_owned()),
            ("trigger".to_owned(), "users_audit".to_owned()),
            ("index".to_owned(), "users_email_index".to_owned()),
        ]
    );

    let runtime_version: String =
        database
            .connection()
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    assert_eq!(database.sqlite_version(), runtime_version);

    Ok(())
}

#[test]
fn applies_explicit_connection_configuration() -> rusqlite::Result<()> {
    let database = SchemaDatabase::from_sql("").expect("empty schema should load");

    let foreign_keys: bool =
        database
            .connection()
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    let trusted_schema: bool =
        database
            .connection()
            .pragma_query_value(None, "trusted_schema", |row| row.get(0))?;

    assert!(foreign_keys);
    assert!(!trusted_schema);

    Ok(())
}

#[test]
fn opens_existing_database_without_write_access() -> rusqlite::Result<()> {
    let source = TestDatabase::new("read-only");
    Connection::open(source.path())?.execute_batch("CREATE TABLE users (id INTEGER);")?;

    let database = SchemaDatabase::open(source.path()).expect("existing database should open");
    let error = database
        .connection()
        .execute_batch("CREATE TABLE forbidden (id INTEGER);")
        .expect_err("database input must remain read-only");

    assert!(error.to_string().contains("readonly"));
    Ok(())
}

#[test]
fn invalid_sql_has_actionable_context() {
    let error = match SchemaDatabase::from_sql("CREATE TABL broken (id INTEGER);") {
        Err(error) => error,
        Ok(_) => panic!("invalid SQLite syntax should fail"),
    };

    let message = error.to_string();
    assert!(message.contains("failed to apply schema SQL"));
    assert!(message.contains("using bundled SQLite"));
    assert!(message.contains("syntax error"));
}

#[test]
fn cannot_attach_and_change_a_target_database() -> rusqlite::Result<()> {
    let target = TestDatabase::new("isolation");
    let connection = Connection::open(target.path())?;
    connection.execute_batch("CREATE TABLE existing (id INTEGER PRIMARY KEY);")?;
    drop(connection);

    let target_path = target.path().to_string_lossy().replace('\'', "''");
    let schema_sql = format!(
        "ATTACH DATABASE '{target_path}' AS target;
         DROP TABLE target.existing;
         CREATE TABL broken (id INTEGER);"
    );
    let error = match SchemaDatabase::from_sql(&schema_sql) {
        Err(error) => error,
        Ok(_) => panic!("schema SQL must not attach a target database"),
    };
    let message = error.to_string();
    assert!(
        message.contains("not authorized") || message.contains("authorization denied"),
        "unexpected isolation error: {message}"
    );

    let target = Connection::open(target.path())?;
    let existing_count: i64 = target.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'existing'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(existing_count, 1);

    Ok(())
}

#[test]
fn cannot_write_a_vacuum_output_database() {
    let output = TestDatabase::new("vacuum-output");
    let output_path = output.path().to_string_lossy().replace('\'', "''");
    let schema_sql = format!("VACUUM INTO '{output_path}';");

    let error = match SchemaDatabase::from_sql(&schema_sql) {
        Err(error) => error,
        Ok(_) => panic!("schema SQL must not write a VACUUM output database"),
    };

    let message = error.to_string();
    assert!(
        message.contains("not authorized") || message.contains("authorization denied"),
        "unexpected isolation error: {message}"
    );
    assert!(!output.path().exists());
}

#[test]
fn cannot_override_settings_or_execute_non_schema_statements() {
    for schema_sql in [
        "PRAGMA foreign_keys = OFF;",
        "PRAGMA trusted_schema = ON;",
        "CREATE TABLE data (value BLOB); INSERT INTO data VALUES (zeroblob(1024));",
        "CREATE TABLE populated AS SELECT 1 AS value;",
        "WITH RECURSIVE values_table(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM values_table WHERE value < 10) SELECT * FROM values_table;",
    ] {
        assert!(
            SchemaDatabase::from_sql(schema_sql).is_err(),
            "schema construction should reject non-schema side effects: {schema_sql}"
        );
    }
}

struct TestDatabase {
    path: PathBuf,
}

impl TestDatabase {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sqlite-schema-{name}-{}-{}.db",
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
