# Decision log

This log records changes to established project direction.
The linked documents own the current design rules.

| Date | Decision | Current design |
| --- | --- | --- |
| 2026-09-06 | Name the project and CLI `sqlite-schema`. | [README](../README.md) |
| 2026-09-06 | Build a SQLite-specific declarative migration tool with a `dump`, `plan`, and `apply` workflow. | [Architecture](architecture.md) |
| 2026-09-06 | Construct desired state by applying SQL to an isolated SQLite database, then inspect current and desired databases through the same path. | [Schema modeling and migration planning](design/schema-planning.md) |
| 2026-09-09 | Accept SQL or an existing database file independently for either side of a schema comparison. | [Schema modeling and migration planning](design/schema-planning.md#schema-input-construction) |
| 2026-09-06 | Treat deterministic, data-preserving table rebuilds as supported but structurally expensive changes. | [Schema modeling and migration planning](design/schema-planning.md#operation-classification) |
| 2026-09-06 | Keep safety, structural cost, and runtime-data dependency as separate plan dimensions. | [Schema modeling and migration planning](design/schema-planning.md#operation-classification) |
| 2026-09-06 | Do not estimate disk usage, copied bytes, or duration from a schema-only plan. | [Schema modeling and migration planning](design/schema-planning.md#operation-classification) |
