//! Library support for constructing and inspecting SQLite schema state.

mod inspector;
mod schema;
mod schema_database;

pub use inspector::{InspectError, SchemaSource, inspect_schema};
pub use schema::{Column, SchemaFingerprint, SchemaModel, Table};
pub use schema_database::{SchemaDatabase, SchemaDatabaseError};
