# Extend all 9 extractors to emit parameter/local-variable symbols

**Label:** enhancement
**Type:** AFK

## What to build

The Java contract review (`docs/contract-review.md` item 7) surfaced that several language extractors don't currently produce `symbol` rows for parameters or local variables. Without those rows, the references walker (issue #16) can't resolve identifiers inside function bodies to anything — they'd all be `referent_id = null`.

This slice audits each `src/languages/<lang>/queries.rs`, extends the symbol extraction to emit `symbol` rows for function/method parameters and locally-declared variables (where the language supports it), and confirms via snapshot tests that resolution works end-to-end. The work is parity-gated: all 9 languages must extract these symbols before the references work lands.

Parallelizable: dispatch one subagent per language using the contract docs as the spec.

## Acceptance criteria

- [ ] Each `src/languages/<lang>/queries.rs` extracts parameter symbols with `kind = "parameter"`
- [ ] Each extractor emits local-variable symbols where the language has lexical locals (Rust `let`, Go `:=`, JS `let`/`const`/`var`, Python locals at function scope, Java/C#/C/C++/PHP local declarations)
- [ ] Languages without distinct local-declaration syntax (Python's first-assignment-defines) emit local symbols at first assignment within a function scope
- [ ] Parameter symbols populate `parent_id = <function_symbol_id>`
- [ ] Per-language snapshot tests at `tests/snapshots/<lang>/locals.cozoql` validate the expected `symbol` rows for a representative function from the benchmark corpus
- [ ] `cargo test` passes

## Blocked by

- #2, #3, #4, #5, #6, #7, #8, #9, #10
