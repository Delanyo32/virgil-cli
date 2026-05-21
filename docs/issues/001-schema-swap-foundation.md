# Schema swap to new Datalog model (foundation, empty extraction)

**Label:** enhancement
**Type:** AFK

## What to build

Replace every Cozo relation defined in `src/cozo/schema.rs` with the new Datalog-model schema in `docs/virgil-datalog-schema.md`. Bump `SCHEMA_VERSION` so persistent caches auto-wipe on next open. Rewrite `CozoWriter` push methods + `flush` SQL to match the new relation shapes (String IDs, `match_index` keyed `references`, `field_type`, etc.). Rewrite `from_code_graph::populate` and `wipe_workspace_relations` against the new shapes. Delete all built-in `*.cozoql` templates and the three Rust handlers (`complexity_hotspots`, `taint_paths`, `unreleased_resources`) — they will be rebuilt under issue #17.

This slice is foundation-only: no language extractor is updated yet, so every new relation will be empty after a query run. A smoke test confirms the schema applies cleanly and basic queries against empty relations return no rows without error.

## Acceptance criteria

- [ ] `src/cozo/schema.rs` mirrors every relation in `docs/virgil-datalog-schema.md` (including `match_index` key on `references` and the new `field_type` relation)
- [ ] `SCHEMA_VERSION` bumped; persistent caches from prior versions auto-wipe on open
- [ ] `CozoWriter` interface updated for String IDs across every push method
- [ ] All 7 built-in `*.cozoql` templates deleted; `rust_templates.rs` reduced to a stub that returns "no built-in templates available during migration"
- [ ] `cargo test` passes (existing tests adapted to assert empty new-shape relations rather than removed; obsolete assertions deleted)
- [ ] `cargo run -- projects query <any benchmark> --cozoscript '?[p] := *file{path: p}'` runs without error and returns zero rows (extractors not yet wired)

## Blocked by

None - can start immediately.
