use std::{error::Error, fmt, io, path::Path};

use rusqlite::{
    Connection, OpenFlags,
    hooks::{AuthAction, AuthContext, Authorization},
};

const CONNECTION_CONFIGURATION: &str = "
    PRAGMA foreign_keys = ON;
    PRAGMA trusted_schema = OFF;
";

/// A SQLite database materialized from one schema input.
pub struct SchemaDatabase {
    connection: Connection,
    sqlite_version: String,
}

impl SchemaDatabase {
    /// Constructs an isolated database by asking bundled SQLite to execute `schema_sql`.
    pub fn from_sql(schema_sql: &str) -> Result<Self, SchemaDatabaseError> {
        let sqlite_version = rusqlite::version().to_owned();
        let connection = Connection::open_in_memory().map_err(|source| {
            SchemaDatabaseError::sqlite(
                "failed to open the isolated schema database",
                sqlite_version.clone(),
                source,
            )
        })?;

        connection
            .execute_batch(CONNECTION_CONFIGURATION)
            .map_err(|source| {
                SchemaDatabaseError::sqlite(
                    "failed to configure the isolated schema database",
                    sqlite_version.clone(),
                    source,
                )
            })?;

        connection.authorizer(Some(authorize_schema_sql));

        connection.execute_batch(schema_sql).map_err(|source| {
            SchemaDatabaseError::sqlite(
                "failed to apply schema SQL",
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

    /// Opens an existing SQLite database read-only, without creating a missing path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SchemaDatabaseError> {
        let path = path.as_ref();
        let sqlite_version = rusqlite::version().to_owned();
        let resolved_path = path
            .canonicalize()
            .map_err(|source| SchemaDatabaseError::Path {
                context: format!(
                    "failed to resolve existing schema database {}",
                    path.display()
                ),
                sqlite_version: sqlite_version.clone(),
                source,
            })?;
        let connection =
            Connection::open_with_flags(&resolved_path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
                |source| {
                    SchemaDatabaseError::sqlite(
                        format!(
                            "failed to open schema database {} read-only",
                            resolved_path.display()
                        ),
                        sqlite_version.clone(),
                        source,
                    )
                },
            )?;

        Ok(Self {
            connection,
            sqlite_version,
        })
    }

    /// Returns the materialized database connection for schema inspection.
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Returns the bundled SQLite runtime version used for this database.
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

/// A contextual failure while materializing a schema input.
#[derive(Debug)]
pub enum SchemaDatabaseError {
    /// The database path could not be resolved to an existing filesystem entry.
    Path {
        /// Action that failed.
        context: String,
        /// Bundled SQLite version associated with the attempted input.
        sqlite_version: String,
        /// Original filesystem error.
        source: io::Error,
    },
    /// SQLite could not materialize the schema database.
    Sqlite {
        /// Action that failed.
        context: String,
        /// Bundled SQLite version used for the operation.
        sqlite_version: String,
        /// Original SQLite error.
        source: rusqlite::Error,
    },
}

impl SchemaDatabaseError {
    fn sqlite(context: impl Into<String>, sqlite_version: String, source: rusqlite::Error) -> Self {
        Self::Sqlite {
            context: context.into(),
            sqlite_version,
            source,
        }
    }

    /// Returns the bundled SQLite version associated with the failure.
    #[must_use]
    pub fn sqlite_version(&self) -> &str {
        match self {
            Self::Path { sqlite_version, .. } | Self::Sqlite { sqlite_version, .. } => {
                sqlite_version
            }
        }
    }
}

impl fmt::Display for SchemaDatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path {
                context,
                sqlite_version,
                source,
            } => write!(
                formatter,
                "{context} using bundled SQLite {sqlite_version}: {source}"
            ),
            Self::Sqlite {
                context,
                sqlite_version,
                source,
            } => write!(
                formatter,
                "{context} using bundled SQLite {sqlite_version}: {source}"
            ),
        }
    }
}

impl Error for SchemaDatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Path { source, .. } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
        }
    }
}
