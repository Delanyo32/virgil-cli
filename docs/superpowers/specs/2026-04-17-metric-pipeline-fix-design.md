# Design: Fix Metric Pipeline Detection (deep_nesting, function_length, cyclomatic_complexity)

**Date:** 2026-04-17  
**Source:** VIRGIL_IMPROVEMENTS.md benchmark report (April 2026)  
**Scope:** Bucket 1 (broken metric pipelines) + Bucket 2 (false positive audit)

---

## Problem

Three high-value audit pipelines — `deep_nesting` (9 languages), `function_length` (8 languages), and `cyclomatic_complexity` (7 languages) — produce zero findings on real codebases despite having correct JSON pipeline definitions and working metric computation functions.

**Root cause:** All 10 language parsers store line numbers using `start_position().row as u32`, which is 0-indexed (first line of file = row 0). The `execute_compute_metric` function in `src/pipeline/executor.rs` treats `node.line` as 1-indexed and subtracts 1 before calling `find_function_body_at_line`. For any function not at row 0, the lookup searches at `target_row = actual_row - 1`, misses the function body, emits a silent warning, and pushes the node with no metrics attached. The `flag` stage's `severity_map` then never fires because no metric value exists.

`match_pattern` nodes are already correct — line 818 of `executor.rs` adds `+1` when constructing match results. The inconsistency is purely in the language parsers.

---

## What Is Not Changing

All other false positives from the benchmark report are already resolved:

| Pattern | Resolution |
|---|---|
| `code_injection_*` / `command_injection_*` | Rewritten as taint-flow pipelines |
| `buffer_overflow_risk` | Scoped to known unsafe C/C++ functions by name |
| `nested_callback` | Excludes `.map/.filter/.reduce/.forEach/then/catch/finally` |
| `any_annotation` | Uses `#eq? @ty "any"` — precise predicate, correct line numbers |
| `anemic_class` | Excludes Controller/Repository/Service/Handler/Factory/etc. |
| `excessive_public_api` | Already at `count ≥ 20 AND ratio ≥ 80%` threshold |

`argument_mutation` remains high-FP at `info` severity. A proper fix requires a `lhs_is_parameter` executor primitive (scope analysis) and is explicitly deferred.

---

## Fix

### Language Parsers — Add `+1` to All Row Conversions

In every language parser, change:
```rust
start_position().row as u32
end_position().row as u32
```
to:
```rust
start_position().row as u32 + 1
end_position().row as u32 + 1
```

This applies to `start_line`, `end_line`, and any standalone `line` field on import/callsite nodes. `end_line` is also 0-indexed in all parsers and is used by `query_engine.rs` for line-range reads and the `inside` containment filter — fixing it in the same pass prevents a class of off-by-one errors in those query paths.

**Files:**
- `src/languages/rust_lang.rs` — 2 symbol sites (lines 125, 329), 1 import site (line 213)
- `src/languages/typescript.rs` — ~6 sites (symbols, imports, re-exports, dynamic imports, callsites)
- `src/languages/c_lang.rs` — symbol and include sites
- `src/languages/cpp.rs` — symbol and include sites
- `src/languages/java.rs` — symbol and import sites
- `src/languages/go.rs` — symbol and import sites
- `src/languages/python.rs` — symbol and import sites
- `src/languages/csharp.rs` — symbol sites
- `src/languages/php.rs` — symbol and import sites

No changes to `src/pipeline/executor.rs`. The subtraction in `execute_compute_metric` is correct — it converts from 1-indexed to the 0-indexed row expected by tree-sitter's `start_position().row`.

### Unit Tests

The TypeScript parser unit test asserts `syms[0].start_line == 0` for a function on the first line of a file. After the fix, `start_line` will be `1` for the first line. Update this assertion and any other parser tests that hardcode line numbers derived from parser output.

The `compute_metric` executor tests at lines 1846 and 1928 manually construct graph nodes with `start_line: 1` (for a first-line function) — these remain valid because the fix is in the parsers, not the executor.

---

## Data Flow After Fix

```
Parser reads AST node at row R (0-indexed)
  → stores start_line = R + 1  (1-indexed)

execute_compute_metric reads node.line = R + 1
  → target_line = (R + 1) - 1 = R  (back to 0-indexed for tree-sitter)

find_function_body_at_line searches for node where start_position().row == R
  → MATCH ✓ → returns body node → metric computed → flag fires
```

---

## Acceptance Criteria

1. `cargo test` passes with no regressions.
2. Running `virgil audit --dir <any-rust-codebase> --pipeline deep_nesting_rust` produces findings with correct 1-indexed line numbers.
3. Same for `function_length` and `cyclomatic_complexity` against their respective language benchmarks.
4. Audit findings for `match_pattern`-based pipelines (e.g. `panic_prone_calls`) continue to report the same line numbers as before (no regression).

---

## Out of Scope

- `argument_mutation` scope analysis (`lhs_is_parameter` primitive)
- New patterns: `hardcoded_secrets`, `print_instead_of_logging`, `deprecated_api_usage`, `outdated_dependency` (Bucket 3 from the benchmark report)
- Benchmark harness integration or detection rate measurement
