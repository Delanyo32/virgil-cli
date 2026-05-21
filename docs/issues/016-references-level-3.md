# References (Level 3): per-language fact emitters + Cozoscript resolver

**Label:** enhancement
**Type:** AFK
**Revised:** Per [ADR-0005](../adr/0005-datalog-resolution.md), this slice splits into fact emission (per language) and resolution (one Cozoscript rule set). See subtasks below.

## What to build

Populate the `references` relation by separating fact emission from resolution:

- **Per-language extractors** (9 languages, parallel) emit `occurrence` / `scope` / `binding` rows per the updated [contract template](../contract-template.md) and the rewritten `docs/references-<lang>.md` contracts.
- **A single Cozoscript resolver** (`docs/resolution.md`) consumes those facts plus the existing `imports` relation and materialises `references` rows during `populate()`. The resolver handles scope walking, shadowing, transitive re-exports, alias following, and wildcard imports — uniformly across all 9 languages.

The Level-3 commitment from ADR-0003 is preserved (full lexical scope, transitive re-exports, aliases, wildcards). What changes is *where* the resolution algorithm lives.

## Subtasks

### 16a — Per-language fact emitters

Per language, extend the extractor to emit `occurrence`, `scope`, and `binding` rows. Dispatch one subagent per language using the rewritten `docs/references-<lang>.md` as the contract.

### 16b — Cozoscript resolver

Implement the resolver rules per `docs/resolution.md` in `src/queries/resolution/` (folder TBD). Wire them into `populate()` so `references` rows materialise after fact emission completes.

### 16c — Resolver test suite

Synthetic-factbase tests under `tests/resolution/`: each test fixes minimal `occurrence` / `scope` / `binding` / `imports` rows and asserts the exact `references` rows the resolver produces. Cover shadowing, transitive re-exports, aliases, wildcards, overload disambiguation, and unresolved-occurrence-emits-null.

## Acceptance criteria

- [ ] `occurrence` populated for every language (call, read, write, type_use, import_use)
- [ ] `scope` populated covering file/module/namespace/class/function/block as applicable per language
- [ ] `binding` populated for definition, parameter, import, import_alias, wildcard_import
- [ ] Cozoscript resolver rules in `src/queries/resolution/` produce `references` rows that match `docs/resolution.md`'s spec
- [ ] `populate()` runs the resolver after fact emission; `references` is non-empty for non-trivial benchmarks
- [ ] Per-language snapshot tests at `tests/snapshots/<lang>/references.cozoql` validate expected rows for a fixture exercising shadowing + a write to a non-local
- [ ] Resolver tests at `tests/resolution/*.rs` pass against synthetic factbases
- [ ] `cargo test` passes

## Blocked by

- #11 (parameter/local symbols feed `binding` rows of kind `parameter`)
- #13 (signatures provide `type_use` occurrences in parameter/return positions)
