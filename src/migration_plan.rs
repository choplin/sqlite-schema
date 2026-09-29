use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

use crate::{SchemaDiff, SchemaModel};

/// Current machine-readable plan format.
pub const PLAN_FORMAT_VERSION: u32 = 1;

/// A complete, ordered migration plan that can be reviewed and saved.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MigrationPlan {
    format_version: u32,
    sqlite_version: String,
    source_fingerprint: String,
    desired_fingerprint: String,
    operations: Vec<PlannedOperation>,
}

impl MigrationPlan {
    /// Returns the plan format version.
    #[must_use]
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the bundled SQLite version used to inspect both inputs.
    #[must_use]
    pub fn sqlite_version(&self) -> &str {
        &self.sqlite_version
    }

    /// Returns the source schema fingerprint.
    #[must_use]
    pub fn source_fingerprint(&self) -> &str {
        &self.source_fingerprint
    }

    /// Returns the desired schema fingerprint.
    #[must_use]
    pub fn desired_fingerprint(&self) -> &str {
        &self.desired_fingerprint
    }

    /// Returns operations in execution order.
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
}

/// One executable migration operation and its independent classifications.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlannedOperation {
    operation: OperationKind,
    table: String,
    data_effect: DataEffect,
    structural_cost: StructuralCost,
    data_dependent: bool,
    sql: String,
}

impl PlannedOperation {
    /// Returns the kind of migration operation.
    #[must_use]
    pub fn operation(&self) -> OperationKind {
        self.operation
    }

    /// Returns the affected table name.
    #[must_use]
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Returns the operation's data-effect classification.
    #[must_use]
    pub fn data_effect(&self) -> DataEffect {
        self.data_effect
    }

    /// Returns the operation's structural-cost classification.
    #[must_use]
    pub fn structural_cost(&self) -> StructuralCost {
        self.structural_cost
    }

    /// Reports whether success depends on existing application values.
    #[must_use]
    pub fn is_data_dependent(&self) -> bool {
        self.data_dependent
    }

    /// Returns executable SQLite SQL for the operation.
    #[must_use]
    pub fn sql(&self) -> &str {
        &self.sql
    }
}

/// Executable operation selected by the planner.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Create one ordinary table directly.
    CreateTable,
}

impl OperationKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CreateTable => "create_table",
        }
    }
}

/// Whether an operation preserves or intentionally changes schema-owned data.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataEffect {
    /// Every existing value has a deterministic preserving mapping.
    Preserving,
}

impl DataEffect {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Preserving => "preserving",
        }
    }
}

/// Structural work SQLite must perform for an operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralCost {
    /// SQLite can execute the change without rebuilding an existing table.
    Direct,
}

impl StructuralCost {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
        }
    }
}

/// Lowers one supported semantic difference into an executable plan.
pub fn plan_migration(
    current: &SchemaModel,
    desired: &SchemaModel,
    difference: SchemaDiff<'_>,
) -> Result<MigrationPlan, PlanError> {
    if current.sqlite_version() != desired.sqlite_version() {
        return Err(PlanError::SqliteVersionMismatch {
            current: current.sqlite_version().to_owned(),
            desired: desired.sqlite_version().to_owned(),
        });
    }

    let operation = match difference {
        SchemaDiff::CreateTable { table }
            if current.table().is_none() && desired.table() == Some(table) =>
        {
            PlannedOperation {
                operation: OperationKind::CreateTable,
                table: table.name().to_owned(),
                data_effect: DataEffect::Preserving,
                structural_cost: StructuralCost::Direct,
                data_dependent: false,
                sql: terminate_statement(table.canonical_sql()),
            }
        }
        SchemaDiff::CreateTable { .. } => return Err(PlanError::DifferenceModelMismatch),
    };

    Ok(MigrationPlan {
        format_version: PLAN_FORMAT_VERSION,
        sqlite_version: current.sqlite_version().to_owned(),
        source_fingerprint: current.fingerprint().to_string(),
        desired_fingerprint: desired.fingerprint().to_string(),
        operations: vec![operation],
    })
}

fn terminate_statement(sql: &str) -> String {
    let sql = sql.trim_end();
    if sql.ends_with(';') {
        sql.to_owned()
    } else {
        format!("{sql};")
    }
}

/// A plan cannot be reproduced with one bundled SQLite runtime.
#[derive(Debug, Eq, PartialEq)]
pub enum PlanError {
    /// The supplied difference was not derived from the supplied models.
    DifferenceModelMismatch,
    /// Current and desired models were inspected by different SQLite versions.
    SqliteVersionMismatch {
        /// SQLite version used for the current model.
        current: String,
        /// SQLite version used for the desired model.
        desired: String,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DifferenceModelMismatch => write!(
                formatter,
                "schema difference does not match the supplied current and desired models"
            ),
            Self::SqliteVersionMismatch { current, desired } => write!(
                formatter,
                "current schema used SQLite {current}, but desired schema used SQLite {desired}"
            ),
        }
    }
}

impl Error for PlanError {}

#[cfg(test)]
mod tests {
    use crate::{
        DataEffect, OperationKind, PlanError, SchemaSource, StructuralCost, diff_schemas,
        inspect_schema, plan_migration,
    };

    #[test]
    fn creates_one_classified_operation_with_reproducible_context() {
        let current = inspect_schema(SchemaSource::Sql("")).expect("empty schema should inspect");
        let desired = inspect_schema(SchemaSource::Sql("CREATE TABLE users (id INTEGER);"))
            .expect("one-table schema should inspect");
        let difference = diff_schemas(&current, &desired).expect("change should be supported");

        let plan = plan_migration(&current, &desired, difference).expect("plan should build");

        assert_eq!(plan.format_version(), 1);
        assert_eq!(plan.sqlite_version(), current.sqlite_version());
        assert_eq!(plan.source_fingerprint(), current.fingerprint().to_string());
        assert_eq!(
            plan.desired_fingerprint(),
            desired.fingerprint().to_string()
        );
        let [operation] = plan.operations() else {
            panic!("plan should contain exactly one operation");
        };
        assert_eq!(operation.operation(), OperationKind::CreateTable);
        assert_eq!(operation.table(), "users");
        assert_eq!(operation.data_effect(), DataEffect::Preserving);
        assert_eq!(operation.structural_cost(), StructuralCost::Direct);
        assert!(!operation.is_data_dependent());
        assert_eq!(operation.sql(), "create table users ( id integer );");
        let rendered = inspect_schema(SchemaSource::Sql(operation.sql()))
            .expect("rendered operation SQL should remain executable");
        assert_eq!(rendered.fingerprint(), desired.fingerprint());
    }

    #[test]
    fn rejects_a_difference_from_other_models() {
        let current = inspect_schema(SchemaSource::Sql("")).expect("empty schema should inspect");
        let users = inspect_schema(SchemaSource::Sql("CREATE TABLE users (id INTEGER);"))
            .expect("users schema should inspect");
        let accounts = inspect_schema(SchemaSource::Sql("CREATE TABLE accounts (id INTEGER);"))
            .expect("accounts schema should inspect");
        let unrelated =
            diff_schemas(&current, &accounts).expect("accounts change should be supported");

        assert_eq!(
            plan_migration(&current, &users, unrelated),
            Err(PlanError::DifferenceModelMismatch)
        );
    }

    #[test]
    fn rendered_sql_round_trips_sqlite_literal_forms() {
        for schema_sql in [
            "CREATE TABLE metrics (value REAL DEFAULT 1e+2);",
            "CREATE TABLE ratios (value REAL DEFAULT .5);",
            "CREATE TABLE masks (value INTEGER DEFAULT 0xCAFE);",
            "CREATE TABLE payloads (value BLOB DEFAULT X'CAFE');",
            "CREATE TABLE messages (value TEXT DEFAULT 'a  b');",
        ] {
            let current =
                inspect_schema(SchemaSource::Sql("")).expect("empty schema should inspect");
            let desired = inspect_schema(SchemaSource::Sql(schema_sql))
                .expect("desired schema should inspect");
            let difference = diff_schemas(&current, &desired).expect("change should be supported");
            let plan =
                plan_migration(&current, &desired, difference).expect("plan should be built");
            let [operation] = plan.operations() else {
                panic!("plan should contain exactly one operation");
            };

            let rendered = inspect_schema(SchemaSource::Sql(operation.sql()))
                .expect("rendered operation SQL should remain executable");
            assert_eq!(
                rendered.fingerprint(),
                desired.fingerprint(),
                "rendered SQL changed desired schema for {schema_sql}"
            );
        }
    }
}
