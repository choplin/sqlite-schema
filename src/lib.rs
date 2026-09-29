//! Library support for constructing and inspecting SQLite schema state.

mod inspector;
mod migration_plan;
mod rendering;
mod schema;
mod schema_database;
mod schema_diff;

pub use inspector::{InspectError, SchemaSource, inspect_schema};
pub use migration_plan::{
    DataEffect, MigrationPlan, OperationKind, PlanError, PlannedOperation, StructuralCost,
    plan_migration,
};
pub use rendering::{render_plan_json, render_plan_summary};
pub use schema::{Column, SchemaFingerprint, SchemaModel, Table};
pub use schema_database::{SchemaDatabase, SchemaDatabaseError};
pub use schema_diff::{DiffError, SchemaDiff, diff_schemas};
