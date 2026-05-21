# Datalog migration follow-ups: field symbols + references resolver + per-language references emitters

**Label:** enhancement
**Type:** AFK

## What to build

Five gaps left open by issues #13–#16. Approach for each was decided in a
grilling pass; this issue records the chosen path for each, not the
options considered.

### 1. Emit `field` symbols from the per-language symbol queries

Issue #14 emits `field_type{symbol_id, type_id}` rows whose `symbol_id`
is synthesized per ADR-0002 (`path|line|col|name|field`) but the
matching `symbol` row never gets created — so

```cozo
?[fid] := *field_type{symbol_id: fid}, *symbol{id: fid}
```

returns zero rows.

**Fix:** extend the per-language symbol query in
`src/languages/<lang>/queries.rs` to capture `field_declaration` /
`property_declaration` nodes as Symbol rows of kind `field` (or
`property` for languages that already use that — keep existing
convention). Rebake `tests/snapshots/<lang>/baseline.expected` +
`symbol-metadata.expected` counts.

No data migration needed: tests build a fresh in-memory store every
run, and the production store wipes on a schema-version mismatch.

### 2. Finish the references resolver per `docs/resolution.md`,
###    one rule at a time, each paired with its synthetic test

The current `src/cozo/resolver.rs` is two stub scripts. Ship the
remaining rules incrementally; **each rule lands together with the
test file in `tests/resolution/` that exercises it.**

Rule-by-rule, in this order:

1. **Innermost-binding pick** (replace "first match wins" with
   `max(start_byte)` over `candidate_binding`).
   Test: `tests/resolution/shadowing.rs`.
2. **Overload `match_index` numbering** (`0, 1, 2…` when multiple
   candidates resolve at the same scope).
   Test: `tests/resolution/overload_disambiguation.rs`.
3. **Wildcard import expansion** (when no scoped binding matches,
   expand `wildcard_import` bindings against `imports` + exported
   symbols).
   Test: `tests/resolution/wildcards.rs`.
4. **Aliased imports** (`import { foo as bar }` resolves `bar` to
   `foo`'s target symbol).
   Test: `tests/resolution/aliased_imports.rs`.
5. **Transitive re-exports** (alias chains across 3+ files).
   Test: `tests/resolution/transitive_reexports.rs`.
6. **Unresolved emits null** (verify the fallback: one row with
   `referent_id = null` at `match_index = 0`).
   Test: `tests/resolution/unresolved_emits_null.rs`.

Each test loads minimal `occurrence` / `scope` / `binding` / `imports`
rows directly via `CozoWriter` into a fresh store, runs the resolver,
asserts exact `references` rows. ~50 LOC each.

### 3. Replace `refs_common.rs` with 9 per-language Level-3 emitters,
###    sequentially

Currently 7 languages (python, go, java, php, c, cpp, csharp) use
`src/languages/refs_common.rs` which only emits `definition` bindings
in the file scope. Each language needs its own `references.rs`
mirroring the Rust pilot's shape (or the TypeScript reference at
`src/languages/typescript/references.rs`, ~1000 LOC).

Each per-language extractor adds, on top of file scope + definitions:

- `parameter` bindings inside the enclosing function scope
- `import` / `import_alias` / `wildcard_import` bindings in the right
  scope per the language's import syntax
- Language-specific scope kinds: C++ namespace, TS module / namespace,
  PHP namespace, etc.

**Dispatch sequentially**, one language at a time — finish, integrate,
commit before starting the next. Order: python → go → java → php → c
→ cpp → csharp. This avoids the parallel-quota wall that killed the
#16 fan-out (8 agents dispatched simultaneously all hit a session
limit). Slower wall-clock; each language survives quota resets and
the integration check after each prevents a 7-language merge bomb.

`src/languages/refs_common.rs` deletes when the last per-language
extractor lands.

### 4 + 5 — Subsumed

The grilling pass merged the original "test suite" and "fan-out
resilience" items into items 2 and 3 respectively (each rule pairs
with its test; fan-out is sequential).

## Acceptance criteria

- [ ] `?[count(s)] := *symbol{id: s, kind: "field"}, *field_type{symbol_id: s}` returns a non-zero count for every language's benchmark
- [ ] All 6 resolver rules (innermost-binding, overload numbering,
      wildcards, aliases, transitive re-exports, null fallback) implemented
      in `src/cozo/resolver.rs`
- [ ] `tests/resolution/{shadowing,overload_disambiguation,wildcards,aliased_imports,transitive_reexports,unresolved_emits_null}.rs` all pass
- [ ] Every language has a per-language `src/languages/<lang>/references.rs`
      emitting parameter / import / import_alias / wildcard_import
      bindings (verified by inline tests)
- [ ] `src/languages/refs_common.rs` is deleted
- [ ] `cargo test` passes
- [ ] Per-language snapshot counts in
      `tests/snapshots/<lang>/references.expected` updated to reflect
      richer extraction; the `references_resolved` fraction should rise
      for every language vs. the #16 baseline

## Blocked by

- #14 (field_type) — already shipping rows
- #16 (references) — fact emission + stub resolver in place

## Out of scope

- Java/C#/PHP `throws` relation wiring (5th tuple slot in `extract_types`)
- Extra `*_attrs` columns from `docs/attrs-<lang>.md` not in the schema
- 19 ignored agent self-tests from #13 + #15
- Missing `.cozoql` snapshot files for #16 (only `.expected` exist)
- Python `docstring_style` always `None`
