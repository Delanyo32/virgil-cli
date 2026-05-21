# Symbol resolution moves into Cozoscript rules

Per-language Rust extractors stop computing resolution. They become **fact emitters** — emitting raw identifier occurrences, scope structure, and bindings. Resolution (turning a name occurrence into the `symbol_id` it refers to, in a particular scope, possibly through imports/re-exports/aliases) becomes Cozoscript rules over those facts. This supersedes the per-language scope-walker commitment in [ADR-0003](0003-level-3-types-and-references.md).

The accuracy gain is the load-bearing reason: transitive re-exports, multi-hop imports, aliased imports, and wildcard imports all express naturally as recursive Datalog rules. Per-language Rust resolvers would need bespoke logic for each of these in each of 9 languages. A single Cozoscript resolver expresses them once and applies uniformly.

## Considered options

- **Path A — per-language Rust resolvers (the prior commitment).** Each extractor walks scope and produces fully-resolved `references` rows. Rejected because re-export and import-alias semantics differ per language but the resolution algorithm is fundamentally the same; encoding "follow re-exports transitively" in Rust 9 times duplicates work.
- **Path B — current direction.** Extractors emit `occurrence` / `scope` / `binding` rows. Cozoscript rules resolve. One resolver, many fact emitters.
- **Path C — hybrid.** Per-language extractors produce a partial resolution + Cozoscript fills gaps. Rejected because it splits the resolution logic across two places — when an audit query returns the wrong answer, you can't easily tell which side is wrong.

## Consequences

- **Three new relations** in the schema: `occurrence`, `scope`, `binding`. The existing `references` relation becomes a derived view (Cozoscript rule output) rather than directly populated by extractors. See `docs/virgil-datalog-schema.md` for the shapes.
- **The `calls` relation also becomes derivable** from `occurrence` (with `occurrence_kind = "call"`) joined with the resolved `references` view. Phase 1's direct-populate `calls` rows continue to land for backwards compatibility; in a later cleanup phase they could be dropped in favor of the view.
- **Storage grows roughly 10×.** Every identifier occurrence is a row. On a 50k-line codebase, expect ~100k–500k occurrence rows. Indexes on `occurrence.name` and `binding.name` are load-bearing for query performance.
- **Per-language extractors get larger, not smaller.** They have to emit every occurrence, every lexical scope, and every binding. The compensating win: writing these extractors is mechanical (walk AST, emit facts) compared to writing a correct scope-aware resolver per language.
- **The Cozoscript resolver is its own engineered artifact.** Stratified recursive rules for lexical scope, transitive re-exports, alias following, wildcard expansion. Spec lives in `docs/resolution.md`; implementation lives in `src/queries/resolution/` (or similar) as a versioned set of Cozoscript rules.
- **Audit queries can override or extend resolution.** Because the resolver is a set of named Cozoscript rules, a custom query can redefine `resolve_local` or compose alongside it without forking the extractor.
- **Adding a 10th language collapses to fact emission.** No new resolver code; the existing Cozoscript rules apply automatically once the new extractor produces `occurrence` / `scope` / `binding` rows.

## When this lands

Before Issue #16 starts. Issue #16 splits into:
- **#16a** — per-language fact emitters (9 subagents, in parallel).
- **#16b** — Cozoscript resolver rules (single, in `docs/resolution.md`).

Issue #13 (Level-3 `type` relation, canonical names) becomes mostly Datalog too — `canonical_name` is computed via the same `imports` + `binding` joins instead of per-language Rust passes. Update the per-language `types-<lang>.md` contracts to drop the canonical-resolution-in-extractor language; the resolver handles it.
