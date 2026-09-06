# Architecture

## Summary

`sqlite-schema` is a SQLite-specific declarative schema migration tool.
It turns a desired SQL schema and a target SQLite database into a reviewable, reproducible migration plan.

The architecture uses SQLite itself to interpret SQL.
Both the current and desired schemas pass through the same inspector before they are compared.
This avoids defining a second, narrower version of SQLite syntax in the tool.

## Design priorities

The following priorities determine the system boundaries:

1. **SQLite fidelity.** Valid SQLite SQL should not fail because an independent general-purpose parser does not understand it.
2. **Data preservation.** A migration that has a deterministic, data-preserving result should be supported even when it requires a full table rebuild.
3. **Explicit risk.** The plan must distinguish destructive changes, ambiguous changes, data-dependent changes, and non-destructive changes.
4. **Explicit structural cost.** The plan must distinguish direct metadata operations from table rebuilds without pretending to know elapsed time or temporary disk usage.
5. **Reproducibility.** A saved plan must identify the current schema it was generated from and must not apply silently after schema drift.
6. **SQLite-only design.** The intermediate representation and planner may model SQLite behavior directly instead of compromising for other database dialects.

## Core concepts

**Desired schema** is the complete SQL definition that the user wants the database to have.

**Current schema** is the schema observed from the target SQLite database.

**Schema model** is the canonical intermediate representation obtained by inspecting a SQLite database.
Current and desired schemas use the same model and inspector.

**Schema diff** describes semantic differences between two schema models without yet choosing an executable migration strategy.

**Migration operation** is an executable unit selected to realize a schema difference.
An operation may be a direct SQLite DDL statement, a coordinated table rebuild, or a blocked change that requires user input.

**Plan** is an ordered set of migration operations plus classifications, warnings, and a fingerprint of the source schema.

## Component ownership

The target architecture separates the following responsibilities:

| Component | Owns |
| --- | --- |
| CLI | Command parsing, input and output selection, confirmation, and exit behavior |
| Desired-state loader | Creating an isolated SQLite database and applying the desired SQL |
| Inspector | Reading `sqlite_schema` and PRAGMA results into the canonical schema model |
| Normalizer | Removing representation differences that do not change SQLite semantics |
| Differ | Producing semantic changes between current and desired models |
| Planner | Selecting direct operations, table rebuilds, destructive operations, or blocked outcomes |
| Plan renderer | Human-readable, SQL, and machine-readable representations of one plan |
| Applier | Fingerprint verification, transaction handling, execution, and post-apply verification |

The schema model is the boundary between SQLite inspection and comparison.
The migration plan is the boundary between planning and side effects.

## Planning flow

`plan` follows one symmetric inspection path:

1. Open the target database without scanning application rows and inspect its schema.
2. Compute a deterministic fingerprint of the current schema model.
3. Create an isolated temporary SQLite database.
4. Apply the desired schema SQL with a declared SQLite version and controlled connection configuration.
5. Inspect the desired database with the same inspector used for the target.
6. Normalize both models.
7. Generate semantic differences.
8. Lower each difference into ordered migration operations.
9. Classify each operation and render the plan.

The planner does not estimate bytes, duration, or temporary disk consumption because it does not inspect application data volume.
It reports structural cost, such as whether a table rebuild and full row copy are required.

## Apply flow

`apply` consumes a saved or freshly generated plan:

1. Open the target database and inspect its current schema.
2. Compare its fingerprint with the plan's source fingerprint.
3. Stop if the schema has drifted.
4. Check the plan's approval requirements.
5. Execute each operation with the required transaction and foreign-key handling.
6. Run integrity and foreign-key checks required by rebuilt objects.
7. Inspect the resulting schema and verify the desired fingerprint.

The exact transaction boundary for table rebuilds is owned by the [schema planning design](design/schema-planning.md).

## Boundaries and invariants

- Desired SQL is validated by executing it in SQLite, not by accepting it solely through a separate SQL grammar.
- Current and desired schema models come from the same inspector.
- Unknown schema properties are not silently discarded.
- A table rebuild includes dependent indexes, triggers, views, and foreign-key considerations in one coordinated operation.
- Safety classification and cost classification remain separate.
- A data-preserving operation is not described as cheap merely because its result is deterministic.
- A plan created without scanning application rows does not claim exact runtime cost or data compatibility.
- Applying a stale plan fails before executing its migration operations.
- Unsupported or ambiguous changes produce an explicit blocked result rather than an incomplete SQL script.

## Starting points for implementation

The first vertical slice should cross every major boundary with the smallest semantic change: creating a table in an empty target database.
It should establish the schema model, isolated desired-state database, diff shape, plan serialization, fingerprint check, and apply path.

After that slice works end to end, object coverage can expand in this order:

1. Direct table and index creation and deletion.
2. Directly supported `ALTER TABLE` operations.
3. General table rebuilds with explicit column-copy mappings.
4. Constraints and foreign-key verification.
5. Triggers and views with dependency ordering.
6. SQLite-specific table forms and extension boundaries.

This order is an implementation starting point, not a promise of release scope.
