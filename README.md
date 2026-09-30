# sqlite-schema

`sqlite-schema` is a planned declarative schema migration tool for SQLite.
It will compare two schema inputs, supplied as SQL or existing database files, show a reviewable migration plan, and apply that plan with explicit safety checks.

> [!IMPORTANT]
> The project is in its design phase.
> No working CLI has been released yet.

## Why sqlite-schema?

- **Use SQLite as the source of truth.** Desired SQL will be loaded into a temporary SQLite database instead of being accepted or rejected by a partial, multi-dialect SQL parser.
- **Treat table rebuilds as normal migrations.** SQLite changes that require creating a replacement table and copying data will be planned as first-class operations.
- **Separate safety from cost.** A deterministic, data-preserving rebuild is different from both a cheap direct alteration and a destructive change.
- **Review before changing data.** The planned workflow separates `plan` from `apply` and verifies that the target schema has not drifted before applying a saved plan.

## Planned workflow

The intended interface follows a state-based workflow:

```console
$ sqlite-schema dump app.db > schema.sql
$ $EDITOR schema.sql
$ sqlite-schema plan app.db --file schema.sql
$ sqlite-schema apply app.db --plan plan.json
```

The exact command-line interface is not implemented and may change during the first development milestone.

## Design

The architecture follows the inspector-only idea used by [pgschema](https://github.com/pgplex/pgschema), adapted for SQLite:

```text
current SchemaSource ── inspect ──> SchemaModel ──┐
                                                 ├─ diff ─> SchemaDiff ─> MigrationPlan ─> DDL
desired SchemaSource ── inspect ──> SchemaModel ──┘
```

SQL and database files are input formats, not comparison roles. Either the current or desired source may use either format.
The roles begin only when two schema models are passed to the differ: the first is the current baseline and the second is the desired result.
The resulting diff describes what changed; planning decides how to realize it safely, and only then can supported operations be rendered as DDL.

Read the [architecture overview](docs/architecture.md) for the system boundaries and the [schema planning design](docs/design/schema-planning.md) for the current design contract.

## Scope

The project targets SQLite specifically rather than sharing a parser or migration generator with other database engines.
The intended scope includes schema objects used by real SQLite applications, including tables, columns, constraints, indexes, triggers, and views.

The initial implementation will establish one end-to-end vertical slice before expanding object coverage.
SQLite extensions, virtual tables, application-defined functions, application-defined collations, and exact version support remain open design questions.

## Development

Implementation has started with a Rust 2024 package that loads schema SQL into an isolated database using bundled SQLite.
The first slice will continue to prove the architectural path rather than maximize syntax coverage:

1. Load schema SQL into an isolated SQLite database. (Implemented.)
2. Inspect SQL or database-file inputs into the same intermediate representation.
3. Plan a simple `CREATE TABLE` change.
4. Serialize a human-readable and machine-readable plan.
5. Apply the saved plan after verifying the source schema fingerprint.

Development requires Rust 1.85 or newer.

Run `nix develop` to enter the pinned Rust development environment, or use
`direnv allow` when direnv is configured with Nix support.

Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` before submitting changes.

## Project documentation

- [Documentation policy](docs/README.md)
- [Architecture](docs/architecture.md)
- [Schema modeling and migration planning](docs/design/schema-planning.md)
- [Decision log](docs/decision-log.md)
