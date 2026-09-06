# Schema modeling and migration planning

## Summary

`sqlite-schema` derives both current and desired state from databases inspected by the same SQLite-specific inspector.
It then converts semantic differences into migration operations that expose data safety, structural cost, and runtime-data dependencies separately.

## Background

SQLite accepts schema definitions that include expressions, generated clauses, collations, partial indexes, triggers, recursive common table expressions, and other SQLite-specific syntax.
A migration tool that reparses all of this SQL through an incomplete multi-dialect grammar can reject SQL that SQLite accepts.

SQLite also supports only a limited set of direct `ALTER TABLE` operations.
Other deterministic changes remain possible through SQLite's [documented generalized schema-change procedure](https://www.sqlite.org/lang_altertable.html#making_other_kinds_of_table_schema_changes): create a replacement table, copy rows, replace the old table, and recreate dependent schema objects.

These two properties require a SQLite-specific approach to both schema modeling and migration planning.

## Problem

SQLite users cannot reliably keep a complete SQL schema as the desired state when the migration tool accepts less SQLite syntax than SQLite itself or silently omits changes that require table reconstruction.
They must either reduce their schema to the tool's subset or maintain hand-written migrations alongside the declarative schema.

## Target users

The primary users are developers who own SQLite databases, keep their schema in version control as SQL, and want reviewable state-based migrations without adopting an ORM-specific schema format.

## Goals

- A schema accepted by the supported SQLite runtime can be represented without requiring the tool to parse every SQL expression into its own AST.
- Current and desired schemas can be compared through one canonical model.
- Deterministic, data-preserving changes remain supported when they require a table rebuild.
- Plans expose destructive effects, ambiguity, structural execution cost, and data-dependent preconditions before apply.
- A saved plan cannot apply to a target whose schema changed after planning.
- Planning remains useful without scanning application rows or estimating runtime resource consumption.

## Non-goals

- Supporting PostgreSQL, MySQL, or another SQL dialect through shared parser and planner abstractions.
- Replacing migration planning with an ordered migration-history table.
- Predicting execution time, copied bytes, or temporary disk usage from schema information alone.
- Automatically inventing data transformations or resolving ambiguous renames.
- Defining implementation packages, public APIs, or a plugin system in this design document.

## Desired-state construction

The desired-state loader creates an isolated SQLite database and executes the supplied schema SQL with SQLite itself.
The inspector then obtains the desired schema model from that database.

This rule makes the SQLite runtime the syntax authority.
The tool may still need a statement boundary mechanism for execution and may parse small expressions when comparison requires structure, but such parsing must not become an independent acceptance gate for otherwise valid SQLite schema SQL.

The isolated database must use an explicit SQLite version and controlled connection settings.
If desired SQL depends on application-defined functions, collations, virtual-table modules, or extensions, the loader must either provide them through an established extension mechanism or report an explicit unsupported dependency.

## Canonical schema model

The inspector combines `sqlite_schema` SQL with semantic metadata exposed by SQLite PRAGMAs.
The model must preserve at least:

- tables and table options;
- ordered columns, declared types, nullability, defaults, generated expressions, and primary-key properties;
- primary-key, unique, check, and foreign-key constraints;
- indexes, including uniqueness, expressions, sort properties, and partial predicates;
- triggers, including timing, events, conditions, and stored SQL;
- views and their stored SQL;
- dependencies required to order replacement and recreation.

Raw stored SQL can remain part of the model when SQLite does not expose a safer structural representation.
Normalization must be semantic and narrow.
Formatting differences alone must not create perpetual diffs, while unknown clauses must not disappear during normalization.

## Difference and operation separation

The differ reports what changed.
The planner decides how SQLite can realize that change.

For example, a changed column constraint is one semantic difference.
Depending on the complete table definition and supported SQLite version, the planner may realize it as a direct operation, a table rebuild, or a blocked operation.

Keeping these stages separate prevents SQLite execution limitations from distorting the schema model.

## Operation classification

Each planned operation carries independent classifications.

### Data effect

| Value | Meaning |
| --- | --- |
| `preserving` | The operation has a deterministic mapping for every existing value and does not intentionally discard schema-owned data. |
| `destructive` | The operation intentionally removes stored data or an object that may contain data. |
| `requires-input` | The desired result does not determine a unique mapping, such as an ambiguous rename or a required transformation expression. |

### Structural cost

| Value | Meaning |
| --- | --- |
| `direct` | SQLite can perform the change without copying the table's rows into a replacement table. |
| `rebuild` | The operation recreates a table and copies its rows, so cost scales with that table and its dependent objects. |

### Runtime-data dependency

An operation may also be marked `data-dependent` when its success depends on existing values even though the plan does not scan them.
Examples include adding a constraint that existing rows may violate or converting values through an explicit expression that can fail.

These dimensions produce the user-facing distinctions established for the project:

1. Deterministic and data-preserving direct changes.
2. Deterministic and data-preserving rebuilds, shown separately because they can be expensive.
3. Destructive changes that require explicit approval.
4. Changes that require user input and cannot be applied as generated.

A plan must not convert structural cost into an unsupported numeric estimate.
It may say that a table is rebuilt and all of its rows are copied, but it must not claim a byte count, duration, or disk requirement without measuring the relevant data.

## Table rebuild contract

A table rebuild is one coordinated migration operation rather than an incidental sequence of unrelated SQL statements.
Its plan representation includes:

- the source and replacement table definitions;
- an explicit destination-to-source expression for every copied column;
- the ordered replacement steps;
- indexes and triggers to recreate;
- views or other objects that constrain ordering;
- foreign-key handling and verification;
- integrity checks required after copying;
- transaction requirements and cleanup behavior on failure.

The default copy mapping uses the same column only when the mapping is unambiguous and preserves values.
The planner does not infer a rename from similar names alone.

The implementation must follow SQLite's safe generalized schema-change procedure rather than editing `sqlite_schema` directly.
Exact handling of `PRAGMA foreign_keys`, deferred constraints, and transaction boundaries must be verified against each supported SQLite version before it becomes an executable contract.

## Plan artifact and apply

A machine-readable plan contains:

- a plan format version;
- the source schema fingerprint;
- the desired schema fingerprint;
- ordered operations and their SQL or structured execution instructions;
- data-effect, structural-cost, and data-dependency classifications;
- approval requirements and blocked reasons;
- SQLite compatibility information needed to reproduce the decision.

`apply` inspects the target again and compares its fingerprint with the source fingerprint before making changes.
It stops on drift rather than regenerating or partially adapting the saved plan.

After execution, `apply` inspects the database again and compares it with the desired fingerprint.
Rebuild operations also run the required SQLite integrity and foreign-key checks.

## Alternatives considered

### Extend a shared multi-dialect parser

This approach keeps one parser and abstract syntax tree across database engines.
It was rejected because accepting SQLite schema SQL would remain limited by the shared grammar, including expression and trigger syntax that SQLite already knows how to validate.

### Patch sqlite3def incrementally

Adding individual grammar productions can fix isolated failures.
It was rejected as the project direction because it does not provide a general table-rebuild planner and keeps valid SQLite syntax dependent on a second parser.

### Require hand-written migrations for rebuilds

This approach could keep the declarative tool small.
It was rejected because deterministic, data-preserving rebuilds are a central SQLite migration case rather than an exceptional escape hatch.

### Inspect desired SQL directly without executing it

This approach avoids creating a temporary database.
It was rejected because the tool would again need to reproduce SQLite's schema grammar and normalization behavior.

## Open questions

- Which implementation language and SQLite binding should the first version use?
- What is the minimum supported SQLite version, and can a plan target a different runtime version from the one used to generate it?
- How should users register application-defined functions, collations, virtual-table modules, and extensions needed by desired SQL?
- What explicit interface should provide rename and data-transformation mappings?
- Which changes require optional row validation during planning, if row inspection is ever introduced as an opt-in mode?
- Which plan operations may be resumed safely after process interruption?
