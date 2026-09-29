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
#[derive(Clone, Debug)]
pub struct Table {
    pub(crate) name: String,
    pub(crate) create_sql: String,
    canonical_sql: String,
    pub(crate) columns: Vec<Column>,
}

impl Table {
    pub(crate) fn new(name: String, create_sql: String, columns: Vec<Column>) -> Self {
        let canonical_sql = canonicalize_create_sql(&create_sql, &name, &columns);
        Self {
            name,
            create_sql,
            canonical_sql,
            columns,
        }
    }

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

    pub(crate) fn canonical_sql(&self) -> &str {
        &self.canonical_sql
    }

    /// Returns columns in SQLite column order.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }
}

impl PartialEq for Table {
    fn eq(&self, other: &Self) -> bool {
        self.name.eq_ignore_ascii_case(&other.name) && self.canonical_sql == other.canonical_sql
    }
}

impl Eq for Table {}

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
                add_string(&mut hasher, &table.name.to_ascii_lowercase());
                add_string(&mut hasher, &table.canonical_sql);
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

fn canonicalize_create_sql(sql: &str, table_name: &str, columns: &[Column]) -> String {
    SqlTokens::new(sql, table_name, columns)
        .collect::<Vec<_>>()
        .join(" ")
}

struct SqlTokens<'a> {
    remaining: &'a str,
    identifiers: Vec<&'a str>,
}

impl<'a> SqlTokens<'a> {
    fn new(sql: &'a str, table_name: &'a str, columns: &'a [Column]) -> Self {
        let mut identifiers = Vec::with_capacity(columns.len() + 1);
        identifiers.push(table_name);
        identifiers.extend(columns.iter().map(|column| column.name.as_str()));
        Self {
            remaining: sql,
            identifiers,
        }
    }

    fn consume_while(&mut self, predicate: impl Fn(char) -> bool) -> &'a str {
        let end = self
            .remaining
            .char_indices()
            .find_map(|(index, character)| (!predicate(character)).then_some(index))
            .unwrap_or(self.remaining.len());
        let (token, remaining) = self.remaining.split_at(end);
        self.remaining = remaining;
        token
    }

    fn consume_quoted(&mut self, opening: char) -> &'a str {
        let closing = if opening == '[' { ']' } else { opening };
        let mut characters = self.remaining.char_indices();
        let _ = characters.next();
        let mut end = self.remaining.len();

        while let Some((index, character)) = characters.next() {
            if character != closing {
                continue;
            }
            if opening != '['
                && characters
                    .clone()
                    .next()
                    .is_some_and(|(_, next)| next == closing)
            {
                let _ = characters.next();
                continue;
            }
            end = index + character.len_utf8();
            break;
        }

        let (token, remaining) = self.remaining.split_at(end);
        self.remaining = remaining;
        token
    }

    fn consume_numeric(&mut self) -> &'a str {
        let bytes = self.remaining.as_bytes();
        let mut end = 0;

        if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
            end = 2;
            while bytes
                .get(end)
                .is_some_and(|byte| byte.is_ascii_hexdigit() || *byte == b'_')
            {
                end += 1;
            }
        } else {
            while bytes
                .get(end)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
            {
                end += 1;
            }
            if bytes.get(end) == Some(&b'.') {
                end += 1;
                while bytes
                    .get(end)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
                {
                    end += 1;
                }
            }
            if bytes
                .get(end)
                .is_some_and(|byte| matches!(byte, b'e' | b'E'))
            {
                end += 1;
                if bytes
                    .get(end)
                    .is_some_and(|byte| matches!(byte, b'+' | b'-'))
                {
                    end += 1;
                }
                while bytes
                    .get(end)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
                {
                    end += 1;
                }
            }
        }

        let (token, remaining) = self.remaining.split_at(end);
        self.remaining = remaining;
        token
    }

    fn normalize_quoted_identifier(&self, token: &str) -> Option<String> {
        let opening = token.chars().next()?;
        if !matches!(opening, '"' | '`' | '[') {
            return None;
        }
        let closing = if opening == '[' { ']' } else { opening };
        let inner = token.strip_prefix(opening)?.strip_suffix(closing)?;
        let unescaped = if opening == '[' {
            inner.to_owned()
        } else {
            inner.replace(&format!("{closing}{closing}"), &closing.to_string())
        };
        self.identifiers
            .iter()
            .any(|identifier| identifier.eq_ignore_ascii_case(&unescaped))
            .then(|| unescaped.to_ascii_lowercase())
    }
}

impl Iterator for SqlTokens<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.remaining = self.remaining.trim_start();
            if self.remaining.is_empty() {
                return None;
            }
            if self.remaining.starts_with("--") {
                self.remaining = self
                    .remaining
                    .find('\n')
                    .map_or("", |index| &self.remaining[index + 1..]);
                continue;
            }
            if self.remaining.starts_with("/*") {
                self.remaining = self
                    .remaining
                    .find("*/")
                    .map_or("", |index| &self.remaining[index + 2..]);
                continue;
            }
            break;
        }

        let first = self.remaining.chars().next()?;
        if matches!(first, 'x' | 'X') && self.remaining[1..].starts_with('\'') {
            self.remaining = &self.remaining[1..];
            let literal = self.consume_quoted('\'');
            return Some(format!("x{literal}"));
        }
        if matches!(first, '\'' | '"' | '`' | '[') {
            let token = self.consume_quoted(first);
            return Some(
                self.normalize_quoted_identifier(token)
                    .unwrap_or_else(|| token.to_owned()),
            );
        }
        let starts_leading_dot_number = first == '.'
            && self
                .remaining
                .as_bytes()
                .get(1)
                .is_some_and(u8::is_ascii_digit);
        if first.is_ascii_digit() || starts_leading_dot_number {
            return Some(self.consume_numeric().to_ascii_lowercase());
        }

        const OPERATORS: [&str; 12] = [
            "->>", "||", "->", "<<", ">>", "<=", ">=", "==", "!=", "<>", "::", "..",
        ];
        if let Some(operator) = OPERATORS
            .into_iter()
            .find(|operator| self.remaining.starts_with(operator))
        {
            self.remaining = &self.remaining[operator.len()..];
            return Some(operator.to_owned());
        }

        if first.is_ascii_punctuation() && first != '_' && first != '$' {
            self.remaining = &self.remaining[first.len_utf8()..];
            return Some(first.to_string());
        }

        let token = self.consume_while(|character| {
            !character.is_whitespace()
                && (!character.is_ascii_punctuation() || matches!(character, '_' | '$'))
        });
        Some(token.to_ascii_lowercase())
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

#[cfg(test)]
mod tests {
    use super::{Column, canonicalize_create_sql};

    fn column(name: &str) -> Column {
        Column {
            ordinal: 0,
            name: name.to_owned(),
            declared_type: "INTEGER".to_owned(),
            not_null: false,
            default_sql: None,
            primary_key_position: 0,
        }
    }

    #[test]
    fn canonical_sql_ignores_formatting_case_comments_and_known_identifier_quotes() {
        let columns = [column("id")];
        let first = canonicalize_create_sql("CREATE TABLE users (id INTEGER);", "users", &columns);
        let second = canonicalize_create_sql(
            "create /* formatting */ table \"users\" ( \"id\" integer ) ;",
            "users",
            &columns,
        );

        assert_eq!(first, second);
    }

    #[test]
    fn canonical_sql_preserves_literal_contents() {
        let columns = [column("value")];
        let first = canonicalize_create_sql(
            "CREATE TABLE data (value TEXT DEFAULT 'a b');",
            "data",
            &columns,
        );
        let second = canonicalize_create_sql(
            "CREATE TABLE data (value TEXT DEFAULT 'a  b');",
            "data",
            &columns,
        );

        assert_ne!(first, second);
    }

    #[test]
    fn canonical_sql_keeps_blob_literals_executable() {
        let columns = [column("value")];
        let canonical = canonicalize_create_sql(
            "CREATE TABLE data (value BLOB DEFAULT X'CAFE');",
            "data",
            &columns,
        );

        assert!(canonical.contains("x'CAFE'"));
    }

    #[test]
    fn canonical_sql_keeps_numeric_literals_indivisible() {
        let columns = [column("value")];
        let canonical = canonicalize_create_sql(
            "CREATE TABLE data (value REAL DEFAULT 1e+2, lower REAL DEFAULT .5, mask INTEGER DEFAULT 0xCAFE);",
            "data",
            &columns,
        );

        assert!(canonical.contains("1e+2"));
        assert!(canonical.contains(".5"));
        assert!(canonical.contains("0xcafe"));
    }
}
