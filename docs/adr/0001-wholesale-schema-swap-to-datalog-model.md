# Wholesale schema swap to the Datalog model

`docs/virgil-datalog-schema.md` defines a richer Cozo schema (typed symbol/file/span/calls/references/type/comment relations + per-language attribute tables) than the current one in `src/cozo/schema.rs`. We're replacing the existing relations wholesale in a single `SCHEMA_VERSION` bump rather than running the old and new shapes side-by-side. The CLI is pre-1.0 (v0.5.0) with no published-API contract on relation names, the cache-wipe machinery already handles version mismatches, and carrying two schemas would double maintenance and the surface area for queries to bind against. Users with custom `*.cozoql` files lose compatibility at the version bump; this is called out in release notes.

## Considered options

- **Additive evolution** — populate both old and new relations during a transition window. Rejected: doubled writer cost, two sources of truth, no clean cutover signal.
- **Versioned namespaces** (`v2_symbol`, etc.) — kept the old names available. Rejected: same maintenance cost as additive evolution with worse ergonomics.
