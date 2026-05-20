# Built-in templates dark during schema migration

The 7 pure-Cozoscript templates in `src/queries/builtin/` and the 3 Rust handlers (`complexity_hotspots`, `taint_paths`, `unreleased_resources`) all bind to current relation names. Rather than rewriting them at every phase that touches the schema, we delete them at Phase 1 and rebuild the set in a dedicated Phase 6 once the new schema has stabilized. Verification during phases 1–5 comes entirely from new snapshot tests committed alongside `../virgil-skills/benchmarks/<lang>/`; old tests are removed, not migrated. A future reader looking at git history will see built-in templates vanish at Phase 1 and reappear at Phase 6 — this ADR exists so the gap is not mistaken for a broken merge.

## Considered options

- **Rewrite templates at every phase touch** — kept templates working continuously, but each phase paid template-rewrite cost on top of schema work, and the templates couldn't fully exercise schema fields that didn't exist yet.
- **Rewrite once at Phase 1, leave them alone after** — relied on Phase 1 templates being a stable target through Phases 2–5; fragile when later phases add columns or relations the templates would naturally use.

## Consequences

- The CLI's `--template` flag effectively returns "unknown template" for every name during phases 1–5. Users who depend on built-in templates must pin to the pre-migration version or wait for Phase 6.
- Snapshot tests become the only signal that the schema is being populated correctly. They must cover the same surface area as the old templates plus the new fields.
- Phase 6 is sized for a clean-slate template authoring effort, not a port — new templates can exploit the full schema (`extends`/`implements`, `references`, `*_attrs`) rather than mirroring the old surface.
