# Benchmark Detection Improvements Design

**Date:** 2026-04-17
**Status:** Approved

## Problem

Cross-language benchmarks against 10 intentional-debt codebases show detection rates of 5–27%, with "Extra" (false positive) counts ranging from 1,263 to 17,801 per benchmark. Two distinct problems drive this:

1. **Missed patterns** — `function_length` and `cyclomatic_complexity` pipelines exist but use an invalid category string (`"code-quality"`), so they never run. `deep_nesting` has a Python-only pipeline using a brittle hardcoded S-expression. No `nesting_depth` metric exists in the engine.
2. **False positives** — Six existing pipelines fire too broadly, drowning signal in noise: `buffer_overflow_risk` fires on every C++ call expression, `any_annotation` fires on all predefined types (not just `any`), `anemic_class` fires on every class declaration, `callback_hell` fires on `.map()`/`.filter()` callbacks, `argument_mutation` fires on local variable assignments.

Scope: AST-based patterns only. Ecosystem patterns (`outdated_dependency`, `deprecated_api_usage`, `eol_runtime`) are excluded.

---

## Approach

Impact-ordered single wave following three tiers:

1. **Category fixes** — one-line JSON changes, instant coverage gain across 7–8 languages
2. **New `nesting_depth` metric** — one new Rust function + executor wiring, enables 9 per-language JSON pipelines
3. **False positive fixes** — tree-sitter predicate rewrites and threshold adjustments, all JSON-only except `argument_mutation` (scoped as follow-up)

---

## Section 1: Category Fixes

**Files:** `src/audit/builtin/function_length.json`, `src/audit/builtin/cyclomatic_complexity.json`

**Change:** `"category": "code-quality"` → `"category": "complexity"`

Both pipelines are fully correct — metric computation, thresholds, and flag logic are all sound. The category string is the only thing preventing them from running. Valid categories are: `security`, `architecture`, `code_style`, `complexity`, `scalability`, `tech_debt`.

No other changes to these files.

---

## Section 2: `nesting_depth` Metric + `deep_nesting` Pipelines

### 2a: New Rust metric

**File:** `src/graph/metrics.rs`

Add `compute_nesting_depth(body: Node, config: &ControlFlowConfig) -> usize`.

Algorithm: stack-based walk (matching the pattern used by `compute_cognitive`) that tracks current nesting depth as it descends into nodes whose kind is in `config.nesting_increments`. Returns the maximum depth reached. Uses `nesting_increments` which already has the correct per-language node kinds for all 10 languages (`if_expression`, `for_expression`, `match_expression`, `closure_expression` for Rust; `if_statement`, `for_statement`, `while_statement`, `switch_statement`, `catch_clause` for TS/JS/Java/etc.).

No new config fields needed — `nesting_increments` already covers the right AST node kinds.

### 2b: Executor wiring

**File:** `src/pipeline/executor.rs`

In `execute_compute_metric`, add a match arm for `"nesting_depth"` that calls `compute_nesting_depth(body, &config)` and stores the result as `metrics["nesting_depth"]`. Follows the same pattern as the existing `"cyclomatic_complexity"` and `"function_length"` arms.

### 2c: JSON pipelines

**Replace:** `src/audit/builtin/deep_nesting_python.json` (brittle 5-nested-if S-expression → proper `compute_metric` approach)

**Add 8 new files:**
- `deep_nesting_rust.json`
- `deep_nesting_typescript.json`
- `deep_nesting_javascript.json`
- `deep_nesting_go.json`
- `deep_nesting_java.json`
- `deep_nesting_c.json`
- `deep_nesting_cpp.json`
- `deep_nesting_csharp.json`

All 9 pipelines follow this structure (language-scoped via `"languages"`):

```json
{
  "pipeline": "deep_nesting_<lang>",
  "category": "complexity",
  "languages": ["<ext>"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          {
            "when": { "metrics": { "nesting_depth": { "gte": 6 } } },
            "severity": "error"
          },
          {
            "when": { "metrics": { "nesting_depth": { "gte": 4 } } },
            "severity": "warning"
          }
        ]
      }
    }
  ]
}
```

Thresholds: warning ≥ 4, error ≥ 6. The benchmark examples show 6-level nesting as the canonical bad case; 4 is the widely-cited readability threshold.

---

## Section 3: False Positive Fixes

All fixes below are JSON-only. Tree-sitter predicates (`#match?`, `#not-match?`, `#eq?`) already work in `match_pattern` stages — the executor passes the full query string to `tree_sitter::Query::new` and `QueryCursor::matches` which respects predicates natively.

### 3a: `cpp_buffer_overflow_cpp.json`

**Root cause:** `(call_expression function: (_) @fn_name ...)` matches every C++ function call.

**Fix:** Rewrite the S-expression to only match calls to known unsafe C string functions:

```
(call_expression
  function: [
    (identifier) @fn_name
    (field_expression field: (field_identifier) @fn_name)
    (qualified_identifier name: (identifier) @fn_name)
  ])
(#match? @fn_name "^(strcpy|strcat|sprintf|vsprintf|gets|memcpy|memmove|wcscpy|wcscat|scanf|strdup)$")
```

Keep severity `"error"`. Update description to remove the "Simplified" caveat.

### 3b: `callback_hell_javascript.json`

**Root cause:** `(arguments [(arrow_function) (function_expression)] @callback)` matches callbacks passed to `.map()`, `.filter()`, `.reduce()`, `.forEach()` — idiomatic functional array operations, not callback hell.

**Fix:** Rewrite to capture the parent method name and exclude functional array methods:

```
(call_expression
  function: (member_expression
    property: (property_identifier) @method_name)
  arguments: (arguments
    [(arrow_function) (function_expression)] @callback))
(#not-match? @method_name "^(map|filter|reduce|forEach|find|findIndex|some|every|flatMap|then|catch|finally)$")
```

Keep severity `"info"`. Update description.

### 3c: `anemic_domain_model_csharp.json`

**Root cause:** Flags every `class_declaration` — controllers, repositories, services, middleware all trigger.

**Fix:** Add a `#not-match?` predicate on the class name to exclude infrastructure classes:

```
(class_declaration name: (identifier) @class_name)
(#not-match? @class_name "(?i)(Controller|Repository|Middleware|Handler|Service|Factory|Validator|Filter|Converter|Builder|Provider|Manager|Context|Config|Options)$")
```

Reduce to single capture (remove `@class_decl`) to avoid duplicate findings. Keep severity `"info"`.

### 3d: `any_escape_hatch_typescript.json`

**Root cause:** `(predefined_type) @ty` matches all TypeScript predefined types (`string`, `number`, `boolean`, `void`, `never`, `any`). This produces findings on every type annotation in the file, most at wrong lines relative to actual `any` usages.

**Fix:** Add `#eq?` predicate to restrict matches to nodes whose text is literally `"any"`:

```
((predefined_type) @ty (#eq? @ty "any"))
```

This also fixes the line number problem — the reported line is now always the exact position of the `any` keyword in source. Keep severity `"warning"`.

### 3e: `api_surface_area_*.json` (9 files)

**Root cause:** `count >= 10` threshold flags standard service and repository classes (8–12 public methods is normal).

**Fix:** Raise the `count` threshold from `"gte": 10` to `"gte": 20` in all 9 per-language files:
- `api_surface_area_rust.json`
- `api_surface_area_typescript.json` (if exists, else skip)
- `api_surface_area_javascript.json`
- `api_surface_area_python.json`
- `api_surface_area_go.json`
- `api_surface_area_java.json`
- `api_surface_area_c.json`
- `api_surface_area_cpp.json`
- `api_surface_area_csharp.json`
- `api_surface_area_php.json`

Keep the ratio threshold (`>= 0.8`) unchanged — it correctly requires most symbols to be public, not just count alone.

### 3f: `argument_mutation_javascript.json` (follow-up)

**Root cause:** Tree-sitter cannot determine whether the LHS object identifier is a function parameter vs a locally-declared variable — this requires scope analysis.

**Immediate fix (this plan):** Lower severity from `"warning"` to `"info"` and update the message to explicitly note that manual verification is required (local variable assignments will appear alongside genuine parameter mutations).

**Follow-up primitive:** Design and implement `lhs_is_parameter` as a new DSL `where` clause predicate that checks whether the matched LHS identifier name appears in the enclosing function's formal parameter list. This is out of scope for this plan.

---

## Changes Summary

| Type | Count | Files |
|---|---|---|
| Rust (new metric) | 1 function | `src/graph/metrics.rs` |
| Rust (executor wiring) | 1 match arm | `src/pipeline/executor.rs` |
| JSON (category fix) | 2 files | `function_length.json`, `cyclomatic_complexity.json` |
| JSON (replace) | 1 file | `deep_nesting_python.json` |
| JSON (new pipelines) | 8 files | `deep_nesting_{rust,typescript,javascript,go,java,c,cpp,csharp}.json` |
| JSON (FP fixes) | 13 files | 4 pipeline rewrites + 9 `api_surface_area_*` threshold bumps |

**Total: 2 Rust changes, 24 JSON file changes/additions.**

---

## Out of Scope

- `argument_mutation` `lhs_is_parameter` primitive (follow-up)
- Ecosystem patterns (`outdated_dependency`, `deprecated_api_usage`, `eol_runtime`, `abandoned_library`, `version_drift`)
- `code_injection_call` taint-flow rewrite (the taint stages exist; a proper rewrite is a separate project)
- `print_instead_of_logging` and `hardcoded_secrets` (AST-based versions are feasible but lower priority than the items above)
