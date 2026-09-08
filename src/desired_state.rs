use std::{error::Error, fmt};

use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};

const CONNECTION_CONFIGURATION: &str = "
    PRAGMA foreign_keys = ON;
    PRAGMA trusted_schema = OFF;
";

/// An isolated database containing the schema described by user-supplied SQL.
pub struct DesiredState {
    connection: Connection,
    sqlite_version: String,
}

impl DesiredState {
    /// Constructs desired state by asking the bundled SQLite runtime to execute `schema_sql`.
    pub fn load(schema_sql: &str) -> Result<Self, DesiredStateError> {
        let sqlite_version = rusqlite::version().to_owned();
        let connection = Connection::open_in_memory().map_err(|source| {
            DesiredStateError::new(
                "failed to open the isolated desired-state database",
                sqlite_version.clone(),
                source,
            )
        })?;

        connection
            .execute_batch(CONNECTION_CONFIGURATION)
            .map_err(|source| {
                DesiredStateError::new(
                    "failed to configure the isolated desired-state database",
                    sqlite_version.clone(),
                    source,
                )
            })?;

        connection.authorizer(Some(authorize_schema_sql));

        connection.execute_batch(schema_sql).map_err(|source| {
            DesiredStateError::new(
                "failed to apply desired schema SQL",
                sqlite_version.clone(),
                source,
            )
        })?;
        connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);

        Ok(Self {
            connection,
            sqlite_version,
        })
    }

    /// Returns the isolated database for schema inspection.
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Returns the version of the bundled SQLite runtime that interpreted the schema.
    #[must_use]
    pub fn sqlite_version(&self) -> &str {
        &self.sqlite_version
    }
}

fn authorize_schema_sql(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        AuthAction::Pragma {
            pragma_value: Some(_),
            ..
        } => Authorization::Deny,
        AuthAction::Select if context.accessor.is_none() => Authorization::Deny,
        AuthAction::Insert { table_name }
        | AuthAction::Delete { table_name }
        | AuthAction::Update { table_name, .. }
            if context.accessor.is_none() && !is_schema_catalog(table_name) =>
        {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }
}

fn is_schema_catalog(table_name: &str) -> bool {
    matches!(
        table_name,
        "sqlite_master" | "sqlite_schema" | "sqlite_temp_master"
    )
}

/// A contextual failure while constructing desired state.
#[derive(Debug)]
pub struct DesiredStateError {
    context: &'static str,
    sqlite_version: String,
    source: rusqlite::Error,
}

impl DesiredStateError {
    fn new(context: &'static str, sqlite_version: String, source: rusqlite::Error) -> Self {
        Self {
            context,
            sqlite_version,
            source,
        }
    }

    /// Returns the bundled SQLite version associated with the failure.
    #[must_use]
    pub fn sqlite_version(&self) -> &str {
        &self.sqlite_version
    }
}

impl fmt::Display for DesiredStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} using bundled SQLite {}: {}",
            self.context, self.sqlite_version, self.source
        )
    }
}

impl Error for DesiredStateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}
