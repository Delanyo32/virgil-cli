# Design: A1 Pattern Name Fixes (deep_nesting, function_length, cyclomatic_complexity)

**Date:** 2026-04-17
**Status:** Approved

## Problem

Three audit patterns are undetected in benchmark manifests despite having correct Rust compute
functions and working JSON pipelines. The sole cause is a mismatch between the `pattern` field
in each pipeline's `flag` stage and the name the benchmark manifests expect.

| Pipeline file(s) | Current pattern name | Expected by manifests |
|---|---|---|
| `cyclomatic_complexity.json` | `high_cyclomatic_complexity` | `cyclomatic_complexity` |
| `deep_nesting_*.json` (×10) | `excessive_nesting_depth` | `deep_nesting` |
| `function_length.json` | `function_too_long` | `function_length` |

No Rust changes are required. All three metrics are already implemented in `src/graph/metrics.rs`
and registered in `src/pipeline/executor.rs`.

## Solution

### 1. `cyclomatic_complexity.json`

Change the `flag.pattern` field from `"high_cyclomatic_complexity"` to `"cyclomatic_complexity"`.
All other fields (thresholds, message, kind filter, category) stay unchanged.

### 2. `deep_nesting` — consolidate and rename

Replace all 10 per-language `deep_nesting_*.json` files with a single cross-language
`deep_nesting.json`. The per-language files differ only in their `languages` restriction and
`kind` array. The consolidated file:

- Removes the `languages` field (applies to all registered languages via `ControlFlowConfig`
  dispatch, same as `cyclomatic_complexity.json` and `function_length.json`)
- Uses the superset `kind` array: `["function", "method", "arrow_function"]` — languages without
  `arrow_function` (C, Rust, Go, etc.) simply never produce nodes of that kind, so including it
  causes no false positives
- Changes `flag.pattern` from `"excessive_nesting_depth"` to `"deep_nesting"`
- Preserves existing thresholds: warning `>= 4`, error `>= 6`

Files to delete: `deep_nesting_c.json`, `deep_nesting_cpp.json`, `deep_nesting_csharp.json`,
`deep_nesting_go.json`, `deep_nesting_java.json`, `deep_nesting_javascript.json`,
`deep_nesting_php.json`, `deep_nesting_python.json`, `deep_nesting_rust.json`,
`deep_nesting_typescript.json`.

### 3. `function_length.json`

Change the `flag.pattern` field from `"function_too_long"` to `"function_length"`.
All other fields stay unchanged.

## Files Changed

| Action | File |
|---|---|
| Edit | `src/audit/builtin/cyclomatic_complexity.json` |
| Delete ×10 | `src/audit/builtin/deep_nesting_*.json` |
| Create | `src/audit/builtin/deep_nesting.json` |
| Edit | `src/audit/builtin/function_length.json` |

## Thresholds (unchanged)

| Pattern | Warning | Error |
|---|---|---|
| `cyclomatic_complexity` | `> 10` | `>= 20` |
| `deep_nesting` | `>= 4` | `>= 6` |
| `function_length` | `> 50 lines` | `>= 100 lines` |

## Expected Impact

These fixes unlock benchmark manifest matches that are currently missed solely due to name
mismatch. Detection rates should increase for:

- `cyclomatic_complexity`: 7 languages (c, cpp, csharp, java, javascript, rust, typescript)
- `deep_nesting`: 9 languages (c, cpp, csharp, go, java, javascript, php, rust, typescript)
- `function_length`: 8 languages (c, csharp, go, java, javascript, php, rust, typescript)

## Out of Scope

- Threshold tuning (the report examples are all well above current warning thresholds)
- New Rust metric implementations
- False positive reduction (Workstream B)
- Dependency/ecosystem patterns (A2) and code-quality additions (A3)
