use std::{error::Error, fmt, path::Path};

use rusqlite::Connection;

use crate::{Column, SchemaDatabase, SchemaDatabaseError, SchemaModel, Table};

const CATALOG_QUERY: &str = "
    SELECT 'main', type, name, sql
    FROM main.sqlite_schema
    WHERE name NOT GLOB 'sqlite_*'
    UNION ALL
    SELECT 'temp', type, name, sql
    FROM temp.sqlite_schema
    WHERE name NOT GLOB 'sqlite_*'
    ORDER BY 1, 2, 3
";

/// Input used to construct a schema model, independent of its comparison role.
#[derive(Clone, Copy, Debug)]
pub enum SchemaSource<'a> {
    /// Schema SQL interpreted in an isolated database by bundled SQLite.
    Sql(&'a str),
    /// An existing SQLite database opened without write or create access.
    Database(&'a Path),
}

/// Inspects a schema supplied as SQL or as an existing SQLite database.
pub fn inspect_schema(source: SchemaSource<'_>) -> Result<SchemaModel, InspectError> {
    match source {
        SchemaSource::Sql(schema_sql) => {
            let database = SchemaDatabase::from_sql(schema_sql)
                .map_err(|source| InspectError::Input { source })?;
            inspect_connection(database.connection())
        }
        SchemaSource::Database(path) => {
            let database =
                SchemaDatabase::open(path).map_err(|source| InspectError::Input { source })?;
            inspect_connection(database.connection())
        }
    }
}

fn inspect_connection(connection: &Connection) -> Result<SchemaModel, InspectError> {
    let sqlite_version = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .map_err(|source| InspectError::sqlite("failed to read SQLite runtime version", source))?;
    reject_attached_databases(connection)?;
    let objects = read_catalog(connection)?;

    if objects.is_empty() {
        return Ok(SchemaModel::new(sqlite_version, None));
    }

    if objects.len() != 1 {
        let descriptions = objects
            .iter()
            .map(CatalogObject::description)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(InspectError::unsupported(format!(
            "expected at most one ordinary user table, found: {descriptions}"
        )));
    }

    let object = &objects[0];
    if object.schema != "main" || object.kind != "table" {
        return Err(InspectError::unsupported(format!(
            "unsupported schema object {}",
            object.description()
        )));
    }

    let create_sql = object.sql.clone().ok_or_else(|| {
        InspectError::unsupported(format!(
            "table main.{} has no stored CREATE TABLE SQL",
            object.name
        ))
    })?;
    validate_table_shape(connection, &object.name)?;
    let columns = read_columns(connection, &object.name)?;

    Ok(SchemaModel::new(
        sqlite_version,
        Some(Table::new(object.name.clone(), create_sql, columns)),
    ))
}

fn reject_attached_databases(connection: &Connection) -> Result<(), InspectError> {
    let attached = connection
        .prepare("PRAGMA database_list")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|source| InspectError::sqlite("failed to inspect attached databases", source))?
        .into_iter()
        .filter(|name| name != "main" && name != "temp")
        .collect::<Vec<_>>();

    if attached.is_empty() {
        Ok(())
    } else {
        Err(InspectError::unsupported(format!(
            "attached databases are not supported: {}",
            attached.join(", ")
        )))
    }
}

fn read_catalog(connection: &Connection) -> Result<Vec<CatalogObject>, InspectError> {
    connection
        .prepare(CATALOG_QUERY)
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok(CatalogObject {
                        schema: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        sql: row.get(3)?,
                    })
                })?
                .collect()
        })
        .map_err(|source| InspectError::sqlite("failed to inspect SQLite schema catalog", source))
}

fn validate_table_shape(connection: &Connection, table_name: &str) -> Result<(), InspectError> {
    let (kind, without_rowid, strict): (String, bool, bool) = connection
        .query_row(
            "SELECT type, wr, strict
             FROM pragma_table_list
             WHERE schema = 'main' AND name = ?1",
            [table_name],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|source| InspectError::sqlite("failed to inspect SQLite table flags", source))?;

    if kind != "table" || without_rowid || strict {
        return Err(InspectError::unsupported(format!(
            "table main.{table_name} is not an ordinary rowid table"
        )));
    }

    Ok(())
}

fn read_columns(connection: &Connection, table_name: &str) -> Result<Vec<Column>, InspectError> {
    let rows = connection
        .prepare(
            "SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden
             FROM pragma_table_xinfo(?1)
             ORDER BY cid",
        )
        .and_then(|mut statement| {
            statement
                .query_map([table_name], |row| {
                    Ok(RawColumn {
                        ordinal: row.get(0)?,
                        name: row.get(1)?,
                        declared_type: row.get(2)?,
                        not_null: row.get(3)?,
                        default_sql: row.get(4)?,
                        primary_key_position: row.get(5)?,
                        hidden: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|source| InspectError::sqlite("failed to inspect SQLite table columns", source))?;

    rows.into_iter()
        .map(|column| column.into_supported(table_name))
        .collect()
}

struct CatalogObject {
    schema: String,
    kind: String,
    name: String,
    sql: Option<String>,
}

impl CatalogObject {
    fn description(&self) -> String {
        format!("{}.{} ({})", self.schema, self.name, self.kind)
    }
}

struct RawColumn {
    ordinal: i64,
    name: String,
    declared_type: String,
    not_null: i64,
    default_sql: Option<String>,
    primary_key_position: i64,
    hidden: i64,
}

impl RawColumn {
    fn into_supported(self, table_name: &str) -> Result<Column, InspectError> {
        if self.hidden != 0 {
            return Err(InspectError::unsupported(format!(
                "table main.{table_name} contains generated or hidden column {}",
                self.name
            )));
        }

        let ordinal = u32::try_from(self.ordinal).map_err(|_| {
            InspectError::unsupported(format!(
                "table main.{table_name} has invalid column ordinal {}",
                self.ordinal
            ))
        })?;
        let primary_key_position = u32::try_from(self.primary_key_position).map_err(|_| {
            InspectError::unsupported(format!(
                "table main.{table_name} has invalid primary-key position {}",
                self.primary_key_position
            ))
        })?;
        let not_null = match self.not_null {
            0 => false,
            1 => true,
            value => {
                return Err(InspectError::unsupported(format!(
                    "table main.{table_name} has invalid NOT NULL flag {value}"
                )));
            }
        };

        Ok(Column {
            ordinal,
            name: self.name,
            declared_type: self.declared_type,
            not_null,
            default_sql: self.default_sql,
            primary_key_position,
        })
    }
}

/// A database access failure or an explicitly unsupported schema shape.
#[derive(Debug)]
pub enum InspectError {
    /// Schema SQL could not be loaded into an isolated database.
    Input {
        /// Contextual failure from materializing the schema input.
        source: SchemaDatabaseError,
    },
    /// SQLite could not complete an inspection operation.
    Sqlite {
        /// Action that failed.
        context: String,
        /// Original SQLite error.
        source: rusqlite::Error,
    },
    /// The database contains a shape outside the initial supported subset.
    Unsupported {
        /// Human-readable unsupported boundary.
        reason: String,
    },
}

impl InspectError {
    fn sqlite(context: &'static str, source: rusqlite::Error) -> Self {
        Self::Sqlite {
            context: context.to_owned(),
            source,
        }
    }

    fn unsupported(reason: String) -> Self {
        Self::Unsupported { reason }
    }
}

impl fmt::Display for InspectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input { source } => write!(formatter, "failed to inspect schema input: {source}"),
            Self::Sqlite { context, source } => write!(formatter, "{context}: {source}"),
            Self::Unsupported { reason } => write!(formatter, "unsupported schema: {reason}"),
        }
    }
}

impl Error for InspectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Input { source } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
            Self::Unsupported { .. } => None,
        }
    }
}
