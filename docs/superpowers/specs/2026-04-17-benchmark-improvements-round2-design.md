# Design: Benchmark Improvements Round 2

**Date:** 2026-04-17
**Source:** VIRGIL_IMPROVEMENTS.md benchmark report (April 2026, second run)
**Scope:** Remaining detection gaps and false-positive fixes after the metric pipeline start_line fix

---

## Problem

After the April 17 start_line indexing fix, three metric pipelines (`deep_nesting`, `function_length`, `cyclomatic_complexity`) are now working for 7 of 9 languages. JavaScript and C++ still miss all their manifest entries. Beyond that, several new patterns are absent entirely, and `argument_mutation` still has a high false-positive rate.

**Remaining gaps from benchmark report:**

| Pattern | Languages still missing |
|---|---|
| `deep_nesting` | JavaScript, C++ |
| `function_length` | JavaScript, C# (some entries) |
| `cyclomatic_complexity` | C++ |
| `excessive_public_api` | Go (threshold too high) |
| `high_coupling` | Java (absent), PHP (wrong lines) |
| `dead_export` | JavaScript CommonJS modules |
| `hardcoded_secrets` | Python (new pattern) |
| `print_instead_of_logging` | Python (new pattern) |

**False positive still unresolved:**
- `argument_mutation` (JavaScript): fires on locally-constructed objects, not just parameter mutations

---

## What Is Not Changing

- Build-file patterns (`outdated_dependency`, `deprecated_api_usage`, `eol_runtime`, `abandoned_library`, `version_drift`) — deferred, require external data sources
- `legacy_pattern` — deferred
- Metric pipelines for languages already working (C, Java, Rust, TypeScript, Go, PHP, C#) — no regressions expected

---

## Approach: Option A (Rust infrastructure first, then JSON pipelines)

All Rust-level changes are grouped in Part 1. JSON pipeline additions follow in Part 2. This avoids context-switching between infrastructure and authoring work.

---

## Part 1: Rust Infrastructure

### 1A. JavaScript and C++ Metric Pipeline Gaps

**Root cause (JavaScript):** Arrow functions captured via `lexical_declaration` (e.g. `const foo = () => {}`) have their symbol's `start_line` pointing to the `lexical_declaration` node. `find_function_body_at_line` in `executor.rs:1059` searches for an `arrow_function` node at that row and calls `child_by_field_name("body")`. The JavaScript tree-sitter grammar may not expose a `"body"` field on `arrow_function` nodes the same way TypeScript does — needs a targeted test to confirm and fix.

**Root cause (C++):** `function_definition` nodes with qualified names (e.g. `MyClass::method`) have an additional `declarator` layer. The CFG builder's `find_compound_statement` traverses past this layer using custom logic, but `find_function_body_at_line` uses `child_by_field_name("body")` directly, which may return `None` for qualified definitions. Forward declarations (`declaration` nodes) are also symbolized as `Function` kind, creating spurious no-body lookups.

**Fix:**
1. Write a targeted failing unit test for each: a JS arrow function and a C++ qualified method that should produce a `nesting_depth` metric.
2. Fix `body_field_for_language` and/or `find_function_body_at_line` to handle cases where the body is not a direct `"body"`-named field:
   - For JS `arrow_function`: traverse to the `arrow_function` child of `variable_declarator` before calling `child_by_field_name("body")`, or add `arrow_function` to the walk with a fallback to its expression body.
   - For C++: use the same compound-statement traversal already in `CppCfgBuilder::find_compound_statement`, or unify the two paths.
3. In the C++ parser, skip `declaration` nodes (forward declarations) when creating `Function` kind symbols — they have no body and produce silent `compute_metric` warnings.

**Files:**
- `src/pipeline/executor.rs` — `find_function_body_at_line`
- `src/graph/metrics.rs` — `function_node_kinds_for_language`, `body_field_for_language`
- `src/languages/cpp.rs` — skip forward declarations in symbol extraction
- Tests: new unit tests in `src/pipeline/executor.rs` for JS arrow function and C++ qualified method metrics

### 1B. `lhs_is_parameter` Predicate + Pipeline Audit

**Current state:** `argument_mutation_javascript.json` fires on every `obj.field = value` inside a function body regardless of whether `obj` is a named parameter. It is marked `info` severity with a "manually verify" note.

**Fix:** Add a new `WhereClause` boolean predicate `"lhs_is_parameter": true`. In the `match_pattern` stage of the executor, after finding an `assignment_expression` with a `member_expression` LHS, walk up the AST to the nearest enclosing function node and check whether the object identifier of the `member_expression` appears in that function's parameter list. Only nodes where this check passes are retained.

**Pipeline audit:** After implementing the primitive, search all built-in JSON pipelines for `match_pattern` stages targeting `assignment_expression`, `augmented_assignment_expression`, or `member_expression` in JavaScript/TypeScript files. For any that currently over-flag (documented as high-FP in the benchmark or carrying a "manually verify" note), evaluate whether `lhs_is_parameter: true` tightens them to an acceptable FP rate. Update those pipelines accordingly — raise severity on ones that become precise enough to warrant it.

**`argument_mutation_javascript.json` specifically:**
- Add `{"when": {"lhs_is_parameter": true}}` condition to the flag stage
- Promote severity from `info` to `warning`
- Remove the "manually verify" caveat from the message

**Files:**
- `src/pipeline/dsl.rs` — add `lhs_is_parameter: Option<bool>` to `WhereClause`
- `src/pipeline/executor.rs` — implement predicate evaluation in `match_pattern` stage
- `src/audit/builtin/argument_mutation_javascript.json` — add predicate, raise severity
- Any other pipelines identified in the audit

---

## Part 2: JSON Pipeline Additions

### 2A. `excessive_public_api` for Go — Threshold Adjustment

**Problem:** `api_surface_area_go.json` requires `count >= 20`. The benchmark Go files (`defaults.go`: 13 exported constants, `order.go`/`priority.go`: 11–12 exported symbols) all have >80% exported ratio but fall below the count gate.

**Fix:** Lower `count >= 20` to `count >= 10` in `api_surface_area_go.json`. The 80% ratio condition is kept — Go's uppercase export convention means most files have a high export ratio, and the ratio gate filters out files that simply have few symbols overall.

**File:** `src/audit/builtin/api_surface_area_go.json`

### 2B. `print_instead_of_logging` — New Python Pipeline

**New file:** `src/audit/builtin/print_in_production_python.json`

```json
{
  "pipeline": "print_in_production",
  "category": "code_style",
  "description": "Detects print() calls in production Python code. In codebases that use logging, print() loses severity levels, context, and log routing.",
  "languages": ["python"],
  "graph": [
    {
      "match_pattern": "(call function: (identifier) @fn (#eq? @fn \"print\")) @call",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "print_instead_of_logging",
        "message": "print() call in production code — use the logging module instead",
        "severity": "info"
      }
    }
  ]
}
```

No dependency-detection gate. Flagging `print()` in any non-test Python module is high-signal enough given the benchmark codebase already declares `logging`/`structlog` as dependencies.

### 2C. `hardcoded_secrets` — New Python Pipeline

**New file:** `src/audit/builtin/hardcoded_secrets_python.json`

Pattern matches assignments where the LHS identifier name suggests a secret and the RHS is a string literal:

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects hardcoded secrets assigned to variables with secret-suggesting names in Python.",
  "languages": ["python"],
  "graph": [
    {
      "match_pattern": "(assignment left: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") right: (string) @val) @assign",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable or secrets manager",
        "severity": "error"
      }
    }
  ]
}
```

Entropy-based detection (for high-entropy strings not named obviously) is deferred — name-matching covers the benchmark's `config.py` examples cleanly and has low FP rate.

### 2D. `high_coupling` for Java — Pipeline Rewrite

**Problem:** `coupling_java.json` uses `match_pattern` on every `import_declaration` node, emitting `excessive_imports` for each individual import line. This produces noise at `info` severity and the pattern name doesn't match the benchmark's `high_coupling` expectation.

**Fix:** Rewrite `coupling_java.json` to use file-level `efferent_coupling` metric (consistent with `coupling.json`):

```json
{
  "pipeline": "coupling",
  "category": "code-quality",
  "description": "Detects Java files with high efferent coupling (excessive unique imports).",
  "languages": ["java"],
  "graph": [
    {
      "select": "file",
      "exclude": {
        "or": [{ "is_test_file": true }, { "is_generated": true }]
      }
    },
    { "compute_metric": "efferent_coupling" },
    {
      "flag": {
        "pattern": "high_coupling",
        "message": "{{file}} imports from {{efferent_coupling}} modules — high fan-out coupling",
        "severity_map": [
          { "when": { "metrics": { "efferent_coupling": { "gte": 15 } } }, "severity": "error" },
          { "when": { "metrics": { "efferent_coupling": { "gte": 8 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

Thresholds (warning ≥ 8, error ≥ 15) match `coupling.json` for consistency.

**File:** `src/audit/builtin/coupling_java.json` (rewrite)

### 2E. CommonJS `dead_export` — JavaScript Parser Extension

**Problem:** The JavaScript parser only marks ES module exports (`export` keyword, `export { }`, `export default`) as `is_exported: true`. CommonJS `module.exports.foo = value` and `exports.foo = value` assignments are invisible to the symbol graph.

**Fix:** Extend the JavaScript parser (`src/languages/typescript.rs`, JS path) to detect CommonJS export assignments as exported symbols:
- Pattern 1: `module.exports.NAME = value` → create or update symbol with name `NAME`, `is_exported: true`
- Pattern 2: `exports.NAME = value` → same
- Pattern 3: `module.exports = { NAME: value }` → each key becomes an exported symbol

Once these symbols are in the graph with `is_exported: true`, the existing `dead_exports.json` pipeline picks them up via `exported: true` + `unreferenced: true` with no new pipeline file needed.

**Files:**
- `src/languages/typescript.rs` — add CommonJS export detection to the JS extraction path
- Tests: new unit tests verifying CommonJS exports appear as `is_exported: true` symbols

### 2F. PHP `high_coupling` Line Fix

**Problem:** The benchmark reports `coupling_php.json` fires at class-declaration lines rather than `use` statement lines.

**Fix:** Parse a PHP file with `use` statements using tree-sitter directly (in a test or via `cargo run -- projects query`) to confirm what row `namespace_use_declaration` nodes report. If they point to the class declaration rather than the `use` block, adjust the `match_pattern` to target the correct child node. If the off-by-one is the same start_line indexing issue already fixed in other parsers, verify the PHP parser's `+1` correction was applied to callsite/import nodes as well as symbol nodes.

**File:** `src/audit/builtin/coupling_php.json`

---

## Acceptance Criteria

1. `cargo test` passes with no regressions.
2. `deep_nesting`, `function_length`, `cyclomatic_complexity` produce findings for JavaScript arrow functions and C++ qualified methods in test cases.
3. `argument_mutation` produces zero findings for locally-constructed objects (`const filter = {}; filter.role = role`) and fires on genuine parameter mutations.
4. All built-in pipelines audited for `lhs_is_parameter` applicability; audit findings documented.
5. `api_surface_area_go.json` fires on files with ≥ 10 exported symbols at ≥ 80% export ratio.
6. `print_instead_of_logging` fires on `print()` calls in non-test Python files.
7. `hardcoded_secrets` fires on assignments like `SECRET_KEY = "abc123"` in Python.
8. `coupling_java.json` fires `high_coupling` on Java files with ≥ 8 unique imports (not on every individual import line).
9. CommonJS `module.exports.foo` and `exports.foo` symbols appear as `is_exported: true` and are caught by `dead_exports.json`.
10. PHP `high_coupling` findings point to import lines, not class declaration lines.

---

## Out of Scope

- Build-file patterns: `outdated_dependency`, `deprecated_api_usage`, `eol_runtime`, `abandoned_library`, `version_drift`, `legacy_pattern`
- `argument_mutation` for languages other than JavaScript
- Entropy-based `hardcoded_secrets` detection
- `hardcoded_secrets` for languages other than Python (can be added later as separate pipelines)
- C# `function_length` remaining misses (lower priority, same fix path as JS/C++)
