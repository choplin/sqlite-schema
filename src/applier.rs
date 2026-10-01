use std::{error::Error, fmt, path::Path};

use rusqlite::TransactionBehavior;

use crate::{
    InspectError, MigrationPlan, SchemaDatabase, SchemaDatabaseError, SchemaSource, inspect_schema,
    inspector::inspect_connection, migration_plan::PLAN_FORMAT_VERSION,
};

/// Applies a validated migration plan to one existing SQLite database.
pub fn apply_migration(path: &Path, plan: &MigrationPlan) -> Result<(), ApplyError> {
    if plan.format_version() != PLAN_FORMAT_VERSION {
        return Err(ApplyError::InvalidPlan {
            reason: format!(
                "unsupported format version {}; expected {PLAN_FORMAT_VERSION}",
                plan.format_version()
            ),
        });
    }
    if plan.sqlite_version() != rusqlite::version() {
        return Err(ApplyError::SqliteVersionMismatch {
            planned: plan.sqlite_version().to_owned(),
            runtime: rusqlite::version().to_owned(),
        });
    }
    validate_plan(plan)?;

    let mut database =
        SchemaDatabase::open_writable(path).map_err(|source| ApplyError::TargetDatabase {
            source: Box::new(source),
        })?;

    apply_to_database(&mut database, plan)
}

fn apply_to_database(
    database: &mut SchemaDatabase,
    plan: &MigrationPlan,
) -> Result<(), ApplyError> {
    let transaction = database
        .connection_mut()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| ApplyError::Sqlite {
            context: "failed to begin migration transaction",
            source,
        })?;
    let current = inspect_connection(&transaction).map_err(|source| ApplyError::Inspection {
        phase: "before applying the migration",
        source: Box::new(source),
    })?;
    let actual_source = current.fingerprint().to_string();
    if actual_source != plan.source_fingerprint() {
        return Err(ApplyError::SchemaDrift {
            expected: plan.source_fingerprint().to_owned(),
            actual: actual_source,
        });
    }

    for operation in plan.operations() {
        transaction
            .execute(operation.sql(), [])
            .map_err(|source| ApplyError::Sqlite {
                context: "failed to execute migration operation",
                source,
            })?;
    }

    let result = inspect_connection(&transaction).map_err(|source| ApplyError::Inspection {
        phase: "after applying the migration",
        source: Box::new(source),
    })?;
    let actual_desired = result.fingerprint().to_string();
    if actual_desired != plan.desired_fingerprint() {
        return Err(ApplyError::Postcondition {
            expected: plan.desired_fingerprint().to_owned(),
            actual: actual_desired,
        });
    }

    transaction.commit().map_err(|source| ApplyError::Sqlite {
        context: "failed to commit migration transaction",
        source,
    })
}

fn validate_plan(plan: &MigrationPlan) -> Result<(), ApplyError> {
    let [operation] = plan.operations() else {
        return Err(ApplyError::InvalidPlan {
            reason: "format version 1 requires exactly one operation".to_owned(),
        });
    };
    let desired = inspect_schema(SchemaSource::Sql(operation.sql())).map_err(|source| {
        ApplyError::InvalidPlanSql {
            source: Box::new(source),
        }
    })?;
    let Some(table) = desired.table() else {
        return Err(ApplyError::InvalidPlan {
            reason: "create_table operation did not produce a table".to_owned(),
        });
    };
    if table.name() != operation.table() {
        return Err(ApplyError::InvalidPlan {
            reason: format!(
                "operation names table {}, but its SQL creates table {}",
                operation.table(),
                table.name()
            ),
        });
    }

    let actual_desired = desired.fingerprint().to_string();
    if actual_desired != plan.desired_fingerprint() {
        return Err(ApplyError::InvalidPlan {
            reason: format!(
                "desired fingerprint is {}, but operation SQL produces {}",
                plan.desired_fingerprint(),
                actual_desired
            ),
        });
    }

    Ok(())
}

/// A migration plan could not be applied without violating its safety contract.
#[derive(Debug)]
pub enum ApplyError {
    /// The target database could not be opened for writing.
    TargetDatabase {
        /// Contextual database-opening failure.
        source: Box<SchemaDatabaseError>,
    },
    /// The saved plan does not satisfy the invariants of its format version.
    InvalidPlan {
        /// Invariant that the plan violated.
        reason: String,
    },
    /// The operation SQL cannot reproduce a supported desired schema safely.
    InvalidPlanSql {
        /// Inspection failure from evaluating the operation SQL in isolation.
        source: Box<InspectError>,
    },
    /// The bundled runtime differs from the one that created the plan.
    SqliteVersionMismatch {
        /// SQLite version recorded by the plan.
        planned: String,
        /// SQLite version used by this executable.
        runtime: String,
    },
    /// The target schema changed after the plan was created.
    SchemaDrift {
        /// Source fingerprint recorded by the plan.
        expected: String,
        /// Current target fingerprint.
        actual: String,
    },
    /// Inspecting the target failed during apply.
    Inspection {
        /// Point in the apply flow where inspection failed.
        phase: &'static str,
        /// Underlying inspection failure.
        source: Box<InspectError>,
    },
    /// SQLite could not execute or commit the migration transaction.
    Sqlite {
        /// Action that failed.
        context: &'static str,
        /// Original SQLite failure.
        source: rusqlite::Error,
    },
    /// The committed result would not match the reviewed desired schema.
    Postcondition {
        /// Desired fingerprint recorded by the plan.
        expected: String,
        /// Fingerprint produced by executing the plan.
        actual: String,
    },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetDatabase { source } => write!(formatter, "{source}"),
            Self::InvalidPlan { reason } => write!(formatter, "invalid migration plan: {reason}"),
            Self::InvalidPlanSql { source } => {
                write!(formatter, "invalid migration operation SQL: {source}")
            }
            Self::SqliteVersionMismatch { planned, runtime } => write!(
                formatter,
                "migration plan uses SQLite {planned}, but this executable uses SQLite {runtime}"
            ),
            Self::SchemaDrift { expected, actual } => write!(
                formatter,
                "target schema drifted after planning: expected {expected}, found {actual}"
            ),
            Self::Inspection { phase, source } => {
                write!(formatter, "failed to inspect target {phase}: {source}")
            }
            Self::Sqlite { context, source } => write!(formatter, "{context}: {source}"),
            Self::Postcondition { expected, actual } => write!(
                formatter,
                "migration result did not match the reviewed schema: expected {expected}, found {actual}"
            ),
        }
    }
}

impl Error for ApplyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TargetDatabase { source } => Some(source.as_ref()),
            Self::InvalidPlanSql { source } | Self::Inspection { source, .. } => {
                Some(source.as_ref())
            }
            Self::Sqlite { source, .. } => Some(source),
            Self::InvalidPlan { .. }
            | Self::SqliteVersionMismatch { .. }
            | Self::SchemaDrift { .. }
            | Self::Postcondition { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

    use super::{ApplyError, apply_to_database};
    use crate::{
        MigrationPlan, SchemaDatabase, SchemaSource, diff_schemas, inspect_schema,
        inspector::inspect_connection, parse_plan_json, plan_migration, render_plan_json,
    };

    #[test]
    fn rolls_back_when_operation_execution_fails() {
        let plan = create_table_plan();
        let mut database = SchemaDatabase::from_sql("").expect("empty target should open");
        database
            .connection_mut()
            .authorizer(Some(deny_create_table));

        let error = apply_to_database(&mut database, &plan)
            .expect_err("denied CREATE TABLE should fail during execution");

        assert!(matches!(
            error,
            ApplyError::Sqlite {
                context: "failed to execute migration operation",
                ..
            }
        ));
        database
            .connection_mut()
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        let result = inspect_connection(database.connection())
            .expect("target should remain inspectable after rollback");
        assert!(result.table().is_none());
    }

    #[test]
    fn rolls_back_when_postcondition_does_not_match() {
        let plan = create_table_plan();
        let mut value = serde_json::to_value(&plan).expect("plan should serialize");
        value["desired_fingerprint"] = serde_json::Value::String("0".repeat(64));
        let contents = serde_json::to_vec(&value).expect("modified plan should serialize");
        let mismatched_plan =
            parse_plan_json(&contents).expect("modified plan should retain its typed shape");
        let mut database = SchemaDatabase::from_sql("").expect("empty target should open");

        let error = apply_to_database(&mut database, &mismatched_plan)
            .expect_err("postcondition mismatch should fail after execution");

        assert!(matches!(error, ApplyError::Postcondition { .. }));
        let result = inspect_connection(database.connection())
            .expect("target should remain inspectable after rollback");
        assert!(result.table().is_none());
    }

    fn create_table_plan() -> MigrationPlan {
        let current = inspect_schema(SchemaSource::Sql("")).expect("empty schema should inspect");
        let desired = inspect_schema(SchemaSource::Sql("CREATE TABLE users (id INTEGER);"))
            .expect("desired schema should inspect");
        let difference = diff_schemas(&current, &desired).expect("difference should be supported");
        let plan = plan_migration(&current, &desired, difference).expect("plan should build");
        let contents = render_plan_json(&plan).expect("plan should serialize");
        parse_plan_json(&contents).expect("serialized plan should parse")
    }

    fn deny_create_table(context: AuthContext<'_>) -> Authorization {
        match context.action {
            AuthAction::CreateTable { .. } => Authorization::Deny,
            _ => Authorization::Allow,
        }
    }
}
