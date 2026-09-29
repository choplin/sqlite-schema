use std::{error::Error, fmt};

use crate::{SchemaModel, Table};

/// A semantic difference between two supported schema models.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchemaDiff<'a> {
    /// One ordinary table is present only in the desired schema.
    CreateTable {
        /// Desired table to create.
        table: &'a Table,
    },
}

/// Compares current and desired schema models without selecting execution SQL.
pub fn diff_schemas<'a>(
    current: &'a SchemaModel,
    desired: &'a SchemaModel,
) -> Result<SchemaDiff<'a>, DiffError> {
    match (current.table(), desired.table()) {
        (None, Some(table)) => Ok(SchemaDiff::CreateTable { table }),
        (None, None) | (Some(_), Some(_))
            if current.fingerprint() == desired.fingerprint() =>
        {
            Err(DiffError::NoChange)
        }
        (Some(_), None) => Err(DiffError::Unsupported {
            reason: "removing a table is outside the supported empty-to-one-table change"
                .to_owned(),
        }),
        (Some(_), Some(_)) => Err(DiffError::Unsupported {
            reason: "changing or replacing an existing table is outside the supported empty-to-one-table change"
                .to_owned(),
        }),
        (None, None) => unreachable!("equal empty schemas are handled above"),
    }
}

/// A no-op comparison or a difference outside the executable MVP subset.
#[derive(Debug, Eq, PartialEq)]
pub enum DiffError {
    /// Current and desired schemas are equivalent.
    NoChange,
    /// The semantic difference is not supported by this planner.
    Unsupported {
        /// Human-readable unsupported boundary.
        reason: String,
    },
}

impl fmt::Display for DiffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoChange => write!(formatter, "current and desired schemas are identical"),
            Self::Unsupported { reason } => {
                write!(formatter, "unsupported schema difference: {reason}")
            }
        }
    }
}

impl Error for DiffError {}

#[cfg(test)]
mod tests {
    use crate::{DiffError, SchemaDiff, SchemaSource, diff_schemas, inspect_schema};

    #[test]
    fn identifies_only_empty_to_one_table_as_executable() {
        let current = inspect_schema(SchemaSource::Sql("")).expect("empty schema should inspect");
        let desired = inspect_schema(SchemaSource::Sql("CREATE TABLE users (id INTEGER);"))
            .expect("one-table schema should inspect");

        let difference = diff_schemas(&current, &desired).expect("change should be supported");
        let SchemaDiff::CreateTable { table } = difference;
        assert_eq!(table.name(), "users");
    }

    #[test]
    fn distinguishes_no_change_from_unsupported_change() {
        let empty = inspect_schema(SchemaSource::Sql("")).expect("empty schema should inspect");
        let users = inspect_schema(SchemaSource::Sql("CREATE TABLE users (id INTEGER);"))
            .expect("users schema should inspect");
        let accounts = inspect_schema(SchemaSource::Sql("CREATE TABLE accounts (id INTEGER);"))
            .expect("accounts schema should inspect");

        assert_eq!(diff_schemas(&empty, &empty), Err(DiffError::NoChange));
        assert!(matches!(
            diff_schemas(&users, &empty),
            Err(DiffError::Unsupported { .. })
        ));
        assert!(matches!(
            diff_schemas(&users, &accounts),
            Err(DiffError::Unsupported { .. })
        ));
    }
}
