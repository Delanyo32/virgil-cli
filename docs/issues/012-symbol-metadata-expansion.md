# Symbol metadata: visibility, qualified_name, parent_id, async/static/abstract/mutable (parity across 9 languages)

**Label:** enhancement
**Type:** AFK

## What to build

Populate the symbol-relation columns that go beyond name/kind/file/lines: `visibility` (`public`/`private`/`internal`/`protected`), `qualified_name` (scope-resolved fully-qualified identifier), `parent_id` (containing symbol), and the flags `is_async`/`is_static`/`is_abstract`/`is_mutable`.

Per ADR-0003 parity-gated commitment, all 9 languages get these columns populated in lockstep. Per-language rules live in `docs/types-<lang>.md` and `docs/attrs-<lang>.md`. Dispatch one subagent per language; each implements its language's extraction against the contract doc plus the benchmark corpus.

End-to-end: a query like `?[n] := *symbol{visibility: "public", qualified_name: n, language: "<lang>"}` returns the expected exported surface for each benchmark.

## Acceptance criteria

- [ ] `visibility` populated for every symbol in every language (default `"public"` only when the language has no notion of access modifiers AND the symbol is reachable)
- [ ] `qualified_name` populated per the language's contract doc rules (Rust modules, TS namespaces/classes, Python dotted, Go package-qualified, etc.)
- [ ] `parent_id` populated for nested symbols (methods in classes, inner functions, etc.)
- [ ] `is_async`, `is_static`, `is_abstract`, `is_mutable` populated where the language supports the concept; default `false` otherwise
- [ ] Per-language snapshot tests at `tests/snapshots/<lang>/symbol-metadata.cozoql` validate expected rows
- [ ] `cargo test` passes
- [ ] No `*_attrs` column duplicates these `symbol` columns (per contract review policy 4)

## Blocked by

- #2, #3, #4, #5, #6, #7, #8, #9, #10
