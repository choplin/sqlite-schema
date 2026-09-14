use std::fmt;

use sha2::{Digest, Sha256};

const FINGERPRINT_FORMAT: &[u8] = b"sqlite-schema:model:v1";

/// The supported subset of a SQLite schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaModel {
    sqlite_version: String,
    table: Option<Table>,
    fingerprint: SchemaFingerprint,
}

impl SchemaModel {
    pub(crate) fn new(sqlite_version: String, table: Option<Table>) -> Self {
        let fingerprint = SchemaFingerprint::for_table(table.as_ref());

        Self {
            sqlite_version,
            table,
            fingerprint,
        }
    }

    /// Returns the SQLite runtime version used for inspection.
    #[must_use]
    pub fn sqlite_version(&self) -> &str {
        &self.sqlite_version
    }

    /// Returns the single supported user table, or `None` for an empty schema.
    #[must_use]
    pub fn table(&self) -> Option<&Table> {
        self.table.as_ref()
    }

    /// Returns the deterministic digest of the supported schema content.
    #[must_use]
    pub fn fingerprint(&self) -> SchemaFingerprint {
        self.fingerprint
    }
}

/// One ordinary SQLite table supported by the initial schema model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Table {
    pub(crate) name: String,
    pub(crate) create_sql: String,
    pub(crate) columns: Vec<Column>,
}

impl Table {
    /// Returns the table name as reported by SQLite.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns stored `CREATE TABLE` SQL as reported by `sqlite_schema`.
    #[must_use]
    pub fn create_sql(&self) -> &str {
        &self.create_sql
    }

    /// Returns columns in SQLite column order.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }
}

/// Column metadata exposed by `PRAGMA table_xinfo` for an ordinary column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Column {
    pub(crate) ordinal: u32,
    pub(crate) name: String,
    pub(crate) declared_type: String,
    pub(crate) not_null: bool,
    pub(crate) default_sql: Option<String>,
    pub(crate) primary_key_position: u32,
}

impl Column {
    /// Returns the zero-based SQLite column identifier.
    #[must_use]
    pub fn ordinal(&self) -> u32 {
        self.ordinal
    }

    /// Returns the column name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the declared SQLite type, which may be empty.
    #[must_use]
    pub fn declared_type(&self) -> &str {
        &self.declared_type
    }

    /// Reports whether SQLite exposes an explicit `NOT NULL` constraint.
    #[must_use]
    pub fn is_not_null(&self) -> bool {
        self.not_null
    }

    /// Returns the stored default expression, when present.
    #[must_use]
    pub fn default_sql(&self) -> Option<&str> {
        self.default_sql.as_deref()
    }

    /// Returns the one-based primary-key position, or zero when not in the key.
    #[must_use]
    pub fn primary_key_position(&self) -> u32 {
        self.primary_key_position
    }
}

/// A stable SHA-256 digest of the supported schema representation.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct SchemaFingerprint([u8; 32]);

impl SchemaFingerprint {
    fn for_table(table: Option<&Table>) -> Self {
        let mut hasher = Sha256::new();
        add_bytes(&mut hasher, FINGERPRINT_FORMAT);

        match table {
            None => hasher.update([0]),
            Some(table) => {
                hasher.update([1]);
                add_string(&mut hasher, &table.name);
                add_string(&mut hasher, &table.create_sql);
                add_u64(&mut hasher, table.columns.len() as u64);

                for column in &table.columns {
                    add_u64(&mut hasher, u64::from(column.ordinal));
                    add_string(&mut hasher, &column.name);
                    add_string(&mut hasher, &column.declared_type);
                    hasher.update([u8::from(column.not_null)]);
                    add_optional_string(&mut hasher, column.default_sql.as_deref());
                    add_u64(&mut hasher, u64::from(column.primary_key_position));
                }
            }
        }

        Self(hasher.finalize().into())
    }

    /// Returns the fingerprint bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SchemaFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SchemaFingerprint({self})")
    }
}

impl fmt::Display for SchemaFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn add_optional_string(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        None => hasher.update([0]),
        Some(value) => {
            hasher.update([1]);
            add_string(hasher, value);
        }
    }
}

fn add_string(hasher: &mut Sha256, value: &str) {
    add_bytes(hasher, value.as_bytes());
}

fn add_bytes(hasher: &mut Sha256, value: &[u8]) {
    add_u64(hasher, value.len() as u64);
    hasher.update(value);
}

fn add_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_be_bytes());
}
