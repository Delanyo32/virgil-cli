# Phase 3: Tech Debt + Scalability JSON Migration - Research

**Researched:** 2026-04-16
**Domain:** JSON audit pipeline migration — compute_metric stage, match_pattern stage, WhereClause DSL
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**D-01:** TECH-02 (per-language tech-debt pipeline migrations) is deferred out of Phase 3. Phase 3 covers only the 6 shared pipelines. TECH-02 will be addressed in Phase 4+.

**D-02:** The 4 complexity pipelines ship as 4 cross-language JSON files — one per pipeline, no `"languages"` filter. `compute_metric` operates on graph symbol nodes that already carry language information; the stage itself is language-aware. No per-language splitting needed.
- `cyclomatic_complexity.json`
- `function_length.json`
- `cognitive_complexity.json`
- `comment_to_code_ratio.json`

**D-03:** Each complexity JSON pipeline must explicitly filter by symbol kind before calling `compute_metric`. Use `{"select": "symbol", "kind": ["function", "method", "arrow_function"]}` as the first stage.

**D-04:** Thresholds follow `audit_plans/` specifications (authoritative). Current Rust defaults (CC > 10, function length > 50 lines or > 20 stmts, cognitive > 15, ratio < 0.05 or > 0.60) serve as fallback.

**D-05:** Migrate `n_plus_one_queries` to a single cross-language JSON file using `match_pattern`. Accept precision delta (no hardcoded DB method name filters). Document in JSON `"description"` field.

**D-06:** The match_pattern query for n_plus_one_queries should target the broadest structural pattern: a call expression (method call or function call) inside a loop body.

**D-07:** Migrate `sync_blocking_in_async` as per-language JSON files. One per language group.

**D-08:** Naming follows Phase 1 convention: `sync_blocking_in_async_{lang}.json` for each language group with async support.

**D-09:** Delete Rust pipeline files in the same batch as the JSON replacement — not in a separate cleanup step.

**D-10:** Minimum 1 positive + 1 negative case per pipeline in `tests/audit_json_integration.rs`.

### Claude's Discretion

- Which language groups get `sync_blocking_in_async` JSON files: start with languages that have clear async constructs (TypeScript, Python, Rust, Go). C, C++, PHP, Java files can be minimal/omitted if async detection isn't meaningful — planner's judgment.
- Exact `match_pattern` S-expression queries for n_plus_one_queries and sync_blocking_in_async: derive from existing Rust queries in those pipeline files as starting templates.
- Whether `comment_to_code_ratio` needs a `select` kind filter or operates at file level: check the current Rust helper (resolved below).

### Deferred Ideas (OUT OF SCOPE)

- TECH-02: Per-language tech-debt migrations (any_escape_hatch, type_assertions, etc.)
- SCAL-02: Per-language scalability pipelines
- Threshold boundary tests
- match_pattern text predicates (#match?, #eq? for filtering by identifier name)
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| TECH-01 | Migrate cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio to JSON | compute_metric stage is implemented; WhereClause metric predicate gap identified — see Open Question 1 |
| TECH-03 | Delete all replaced Rust tech debt pipeline files | mod.rs update pattern documented; 9 language × 4 pipeline = up to 36 files, but these 4 share cross-language JSON so deletions span all 9 language mod.rs files |
| SCAL-01 | Migrate n_plus_one_queries and sync_blocking_in_async to JSON | match_pattern stage implemented; per-language sync query patterns documented |
| TEST-01 | 1 positive + 1 negative case per pipeline in audit_json_integration.rs | Existing test infrastructure in tests/audit_json_integration.rs; pattern confirmed |
| TEST-02 | cargo test passes with zero failures at every phase boundary | Standard; run after each JSON file + Rust deletion batch |
</phase_requirements>

---

## Summary

Phase 3 migrates 6 shared audit pipelines from Rust to declarative JSON: the 4 cross-language complexity pipelines (`cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`) and 2 scalability pipelines (`n_plus_one_queries`, `sync_blocking_in_async`). Phase 2 has already implemented both executor stages (`match_pattern` and `compute_metric`) in `src/graph/executor.rs`, so this phase is purely JSON file authoring, Rust pipeline deletion, and test writing.

The dominant implementation challenge is that `WhereClause` (used in `severity_map` `when` clauses) only supports a fixed set of named metric predicates: `count`, `cycle_size`, `depth`, `edge_count`, `ratio`. It does NOT support named metric predicates for `cyclomatic_complexity`, `function_length`, or `cognitive_complexity`. This means the JSON pipeline cannot filter nodes whose CC > 10 using a `when` clause — the executor computes the metric value but cannot apply threshold filtering via the existing `WhereClause.eval_metrics`. A `WhereClause` extension or a pre-filter `select` with a `kind` filter before `flag` is needed. This is the primary open question blocking the JSON schema design.

The `comment_to_code_ratio` pipeline is confirmed to be per-file (not per-function) based on the Rust implementation. The executor's `execute_compute_metric` handles this special case: when `metric_name == "comment_to_code_ratio"`, it operates on the whole file tree rather than a function body. This means `comment_to_code_ratio.json` should use `select: "file"` (not symbol) as its first stage — a direct contradiction of D-03's uniform symbol approach for complexity pipelines.

**Primary recommendation:** Extend `WhereClause` with a generic named-metric predicate (e.g., `metrics: HashMap<String, NumericPredicate>`) before writing the JSON pipelines. Without this, the JSON complexity pipelines cannot express threshold filtering in `severity_map` `when` clauses.

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| compute_metric execution | Graph executor (`src/graph/executor.rs`) | Workspace (file re-parsing) | Executor re-parses file via workspace to locate function body at node line |
| match_pattern execution | Graph executor (`src/graph/executor.rs`) | Workspace (language-filtered file iteration) | Source stage: iterates workspace files, compiles query per language |
| JSON pipeline discovery | `src/audit/json_audit.rs` via `include_dir!` | None | Automatic at compile time — no registration needed |
| Rust pipeline suppression | AuditEngine (`src/audit/engine.rs` line 111) | None | `json_pipeline_names` HashSet suppresses Rust pipelines with same name |
| Complexity metric computation | `src/graph/metrics.rs` | `src/audit/pipelines/helpers.rs` (re-exports) | metrics.rs has all 4 compute functions; helpers.rs re-exports for backward compat |
| Threshold filtering | WhereClause (`src/graph/pipeline.rs`) | Flag stage severity_map | WhereClause.eval_metrics only handles fixed fields — custom metrics not filterable |
| Test execution | `tests/audit_json_integration.rs` | Rust unit tests in pipeline files | Integration tests use AuditEngine end-to-end |

---

## Standard Stack

### Core (Phase-Specific — What Executes the JSON)
| Component | Location | Purpose | Status |
|-----------|----------|---------|--------|
| `execute_compute_metric` | `src/graph/executor.rs:789` | Transforms symbol nodes with metric values | VERIFIED: implemented in Phase 2 |
| `execute_match_pattern` | `src/graph/executor.rs:714` | Source stage: pattern-matches files | VERIFIED: implemented in Phase 2 |
| `compute_cyclomatic` | `src/graph/metrics.rs:38` | CC algorithm (decision points + logical ops) | VERIFIED |
| `compute_cognitive` | `src/graph/metrics.rs:76` | Cognitive complexity (nesting penalty) | VERIFIED |
| `count_function_lines` | `src/graph/metrics.rs:120` | Returns (total_lines, statement_count) | VERIFIED |
| `compute_comment_ratio` | `src/graph/metrics.rs:157` | Returns (comment_lines, code_lines) for whole file | VERIFIED |
| `control_flow_config_for_language` | `src/graph/metrics.rs:202` | Per-language ControlFlowConfig dispatcher | VERIFIED: covers all 10 languages |
| `include_dir!` macro | `src/audit/json_audit.rs` | Auto-discovers JSON files from `src/audit/builtin/` | VERIFIED: Phase 1 implementation |

### Pipeline Format
JSON files go in `src/audit/builtin/`. Format:
```json
{
  "pipeline": "<pipeline_name>",
  "category": "<category>",
  "description": "<description>",
  "languages": ["<optional filter>"],
  "graph": [ ...stages... ]
}
```

### Stage DSL
| Stage | JSON Shape | Implemented | Notes |
|-------|-----------|-------------|-------|
| `select` | `{"select": "symbol", "where": {...}, "exclude": {...}}` | YES | `kind` filter not a WhereClause field — see below |
| `compute_metric` | `{"compute_metric": "cyclomatic_complexity"}` | YES | Supported values: cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio |
| `match_pattern` | `{"match_pattern": "(tree_sitter_s_expr)"}` | YES | Source stage; emits one PipelineNode per AST match |
| `flag` | `{"flag": {"pattern": "...", "message": "...", "severity_map": [...]}}` | YES | `{{metric_name}}` interpolation works for any metric key |

### WhereClause Supported Predicates (CRITICAL GAP)

[VERIFIED: `src/graph/pipeline.rs:107-143`] The `WhereClause` struct currently supports these metric predicates in `eval_metrics`:
- `count` — for Count stage output
- `cycle_size` — for FindCycles output
- `depth` — for MaxDepth output
- `edge_count` — for edge count output
- `ratio` — for Ratio stage output

**Missing predicates** (blocking JSON threshold filtering for complexity pipelines):
- `cyclomatic_complexity` — NOT in WhereClause
- `function_length` — NOT in WhereClause
- `cognitive_complexity` — NOT in WhereClause

**Message interpolation works** for all metrics (via `interpolate_message` loop over `node.metrics`), but `severity_map.when` clauses CANNOT filter by these values.

**Workaround options:**
1. Add `metrics: Option<HashMap<String, NumericPredicate>>` to `WhereClause` — enables `{"when": {"metrics": {"cyclomatic_complexity": {"gt": 10}}}}`
2. Rely on threshold-free flag (flag ALL nodes that reach the stage, use `severity_map` based on message text only — not useful)
3. Add individual named fields to WhereClause for each metric (verbose but simple: `cyclomatic_complexity: Option<NumericPredicate>`)

**Recommended:** Option 3 (individual fields) matches the existing WhereClause pattern for `count`, `depth`, etc. Requires 3 new fields + deserialization. This is a small Rust change that unblocks all 3 compute_metric pipelines.

---

## Architecture Patterns

### System Architecture Diagram

```
JSON file in src/audit/builtin/
         |
         v
json_audit.rs (include_dir! auto-discovery)
         |
         v
AuditEngine::run()
   |-- Suppresses Rust pipeline with same name (line 111)
   |-- Executes JSON pipelines via run_pipeline()
         |
         v
executor::run_pipeline()
         |
         |-- Stage: select(symbol) ---> graph.symbol_nodes
         |         WhereClause filter (kind, exported, is_test_file)
         |
         |-- Stage: compute_metric("cyclomatic_complexity")
         |         workspace.read_file() -> parse -> find_body_at_line -> compute
         |
         |-- Stage: flag
               severity_map when: {cyclomatic_complexity: {gt: 10}}  <-- NEEDS EXTENSION
               message: "CC={{cyclomatic_complexity}} in {{name}}"
```

```
JSON file (match_pattern pipeline)
         |
         v
executor::run_pipeline()
         |
         |-- Stage: match_pattern("(ts_sexp @cap)")
         |         Iterates workspace files, filtered by pipeline.languages
         |         Compiles query per-language (skips incompatible grammars)
         |         Emits PipelineNode per AST match
         |
         |-- Stage: flag
               pattern: "query_in_loop"
               severity: "warning"
```

### select Stage — Kind Filter Note

[VERIFIED: `src/graph/pipeline.rs:453-487`] The `GraphStage::Select` variant's `filter` field is a `WhereClause`, and `WhereClause` does NOT have a `kind` field. D-03 specifies `{"select": "symbol", "kind": ["function", "method", "arrow_function"]}` but this syntax is not currently supported in the JSON DSL.

**Resolution needed:** Add `kind: Option<Vec<String>>` to `WhereClause`, OR use `select: "symbol"` and accept that all symbol kinds pass through (and let `find_function_body_at_line` gracefully skip non-function nodes with an eprintln warning). The latter is the minimal-change path — `execute_compute_metric` already handles "no body found at line" by logging a warning and passing the node through unchanged.

### Recommended JSON File Naming
```
src/audit/builtin/
├── cyclomatic_complexity.json          (no language filter — cross-language)
├── function_length.json                (no language filter — cross-language)
├── cognitive_complexity.json           (no language filter — cross-language)
├── comment_to_code_ratio.json          (no language filter — file-level select)
├── n_plus_one_queries.json             (no language filter — broadest match_pattern)
├── sync_blocking_in_async_typescript.json
├── sync_blocking_in_async_javascript.json
├── sync_blocking_in_async_python.json
├── sync_blocking_in_async_rust.json
└── sync_blocking_in_async_go.json      (if Go goroutine pattern covered)
```

### Pattern 1: compute_metric Pipeline Structure

```json
{
  "pipeline": "cyclomatic_complexity",
  "category": "code-quality",
  "description": "Detects functions with high cyclomatic complexity (CC > 10)",
  "graph": [
    {
      "select": "symbol",
      "exclude": {"is_test_file": true}
    },
    {"compute_metric": "cyclomatic_complexity"},
    {
      "flag": {
        "pattern": "high_cyclomatic_complexity",
        "message": "Function `{{name}}` has cyclomatic complexity of {{cyclomatic_complexity}} (threshold: 10)",
        "severity_map": [
          {"when": {"cyclomatic_complexity": {"gte": 20}}, "severity": "error"},
          {"when": {"cyclomatic_complexity": {"gt": 10}}, "severity": "warning"},
          {"severity": "info"}
        ]
      }
    }
  ]
}
```

Note: `severity_map.when.cyclomatic_complexity` requires the WhereClause extension described above.

### Pattern 2: comment_to_code_ratio — File-Level Select

[VERIFIED: `src/audit/pipelines/typescript/comment_ratio.rs:58-106`] The Rust implementation calls `compute_comment_ratio(tree.root_node(), source, &config)` — operates on the full file root, not a function body. [VERIFIED: `src/graph/executor.rs:824-839`] `execute_compute_metric` special-cases `comment_to_code_ratio`: it calls `compute_comment_ratio(tree.root_node(), ...)` rather than looking for a function body.

**Critical:** This means `comment_to_code_ratio.json` uses `select: "file"` (NOT `select: "symbol"`), and `compute_metric("comment_to_code_ratio")` is a node-level transform that stores the ratio as a percentage integer (0-100) in `metrics["comment_to_code_ratio"]`.

```json
{
  "pipeline": "comment_to_code_ratio",
  "category": "code-quality",
  "description": "Detects files with too few or too many comments relative to code",
  "graph": [
    {
      "select": "file",
      "exclude": {"is_test_file": true}
    },
    {"compute_metric": "comment_to_code_ratio"},
    {
      "flag": {
        "pattern": "under_documented",
        "message": "File has comment ratio of {{comment_to_code_ratio}}% (threshold: 5%-60%)",
        "severity": "warning"
      }
    }
  ]
}
```

Note: The executor stores ratio as integer percentage (ratio * 100). Thresholds must be adjusted: `< 5` (under-documented) = `{"lt": 5}`, `> 60` (over-documented) = `{"gt": 60}`. This requires the WhereClause extension.

For the under/over documented distinction, two separate pipelines (or a single pipeline with both patterns) may be needed since `severity_map.when` can't currently express ranges on custom metrics.

### Pattern 3: n_plus_one_queries — match_pattern Cross-Language

[VERIFIED: `src/graph/executor.rs:714-783`] `execute_match_pattern` iterates workspace files, compiles the tree-sitter query per language (silently skips languages whose grammar rejects the query), emits one `PipelineNode` per AST capture.

The TypeScript Rust pipeline uses two separate queries (method calls and direct calls) plus parent-chain traversal to check loop containment. The JSON `match_pattern` stage runs a SINGLE S-expression query per file. The structural pattern "call inside loop" can be expressed as a nested query in tree-sitter S-expressions.

For TypeScript/JavaScript (tree-sitter-typescript grammar), the broadest containment query:
```
(for_statement
  body: (_
    (call_expression) @call))
```
However, tree-sitter S-expressions do NOT support deeply nested containment without specific intermediate nodes. The better approach is a flat query that captures call expressions and relies on the executor emitting all calls (then a separate where clause filters by loop containment) — but `WhereClause` doesn't support "inside loop" predicates.

**Accepted limitation (D-05/D-06):** The JSON version will capture call_expressions within for/while/forEach loop bodies where structurally visible, with higher false positive rate than the Rust version. The Rust version's DB-name filtering (findOne, axios.get, etc.) is dropped.

### Pattern 4: sync_blocking_in_async — Per-Language

[VERIFIED: `src/audit/pipelines/typescript/sync_blocking_in_async.rs`] TypeScript: looks for calls with Sync-suffix methods inside `async function`/`async arrow_function`. The `is_inside_async_function` check walks the parent chain and text-matches `async` prefix.

[VERIFIED: `src/audit/pipelines/rust/sync_blocking_in_async.rs`] Rust: finds `function_item` nodes with `async` keyword in source text, collects body byte ranges, then checks if `std::fs::*`/`std::thread::sleep` scoped calls fall within those ranges.

[VERIFIED: `src/audit/pipelines/python/sync_blocking_in_async.rs`] Python: uses `GraphPipeline` (not `Pipeline`) — calls `check_with_context()` with `GraphPipelineContext`. Checks for `function_definition` parent with `async` prefix.

[VERIFIED: `src/audit/pipelines/go/sync_blocking_in_async.rs`] Go: The concept is different — Go has goroutines, not async/await. The pipeline detects blocking calls (time.Sleep, http.Get, os.ReadFile) inside goroutine bodies (`go_statement`). This is a `func_literal` inside `go_statement`.

**Key insight for match_pattern queries:** match_pattern emits ALL captures from matching subtrees. To detect "blocking call inside async function", the ideal S-expression queries an async function and captures calls inside its body.

TypeScript/JavaScript async call pattern (simplified):
```
(function_declaration) @fn (call_expression) @call
```
But this doesn't enforce containment. The approach is to write queries that specifically match the outer async construct containing the inner call in a single tree-sitter query.

### Deletion Pattern — mod.rs Updates Required

[VERIFIED: `src/audit/pipelines/typescript/mod.rs`] Each language's `mod.rs` declares `pub mod <pipeline_name>` for each pipeline and includes the pipeline in the `complexity_pipelines()` / `scalability_pipelines()` factory functions.

**For each of the 4 complexity pipelines across 9 language directories:**
1. Delete `<pipeline>.rs` file
2. Remove `pub mod <pipeline>;` from `mod.rs`
3. Remove `Box::new(<Pipeline>::new(language)?)` from the factory function
4. Verify `cargo test` passes after each batch

**n_plus_one_queries and sync_blocking_in_async** exist in all 9 language directories too. Same deletion pattern applies.

**Note:** `src/audit/pipelines/helpers.rs` re-exports from `graph::metrics` and from itself. After all pipeline files that import from `helpers` are deleted, check if `helpers.rs` becomes dead code (deferred to Phase 5 per CONTEXT.md, but note it here).

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Threshold filtering in severity_map | Custom JSON predicate system | Extend WhereClause with named fields | WhereClause already has `count`, `depth` etc. — follow the existing pattern |
| Function body location for compute_metric | Custom line-range logic | `find_function_body_at_line` in executor.rs | Already implemented for all 10 languages via `function_node_kinds_for_language` |
| Per-language ControlFlowConfig | Per-language JSON config | `control_flow_config_for_language(lang)` in metrics.rs | All 10 language configs already implemented and tested |
| Tree-sitter query compilation | Custom parser | `tree_sitter::Query::new(&ts_lang, query_str)` | Already used in `execute_match_pattern` — per-language compilation with silent skip on grammar mismatch |

**Key insight:** The executor already handles all language-dispatch for compute_metric. The JSON author never writes language-specific metric logic — only stage DSL.

---

## Runtime State Inventory

> Not applicable — this is a pure code/JSON migration phase. No rename, rebrand, or migration of stored data. Skipped.

---

## Common Pitfalls

### Pitfall 1: WhereClause Metric Predicate Gap
**What goes wrong:** Planner writes `severity_map.when.cyclomatic_complexity` — this field does not exist in `WhereClause`, causing a serde deserialization error at runtime.
**Why it happens:** `WhereClause` only has fixed named fields. `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio` are NOT fields.
**How to avoid:** Add these fields to `WhereClause` in `src/graph/pipeline.rs` before writing the JSON pipelines. Also add corresponding match arms to `eval_metrics()`.
**Warning signs:** JSON pipeline deserialization error mentioning unknown field in `when` clause.

### Pitfall 2: comment_to_code_ratio Uses File Nodes, Not Symbol Nodes
**What goes wrong:** Using `select: "symbol"` for `comment_to_code_ratio` will cause `execute_compute_metric` to look for a function body at each symbol's line — which is wrong for a file-level ratio.
**Why it happens:** All 4 complexity pipelines look similar, but `comment_to_code_ratio` is fundamentally different: it operates on the whole file.
**How to avoid:** Use `select: "file"` as the first stage in `comment_to_code_ratio.json`.
**Warning signs:** Zero findings for files that should be under-documented.

### Pitfall 3: select Kind Filter Not in WhereClause
**What goes wrong:** Writing `{"select": "symbol", "kind": ["function", "method"]}` — `kind` is not a field in `GraphStage::Select` or `WhereClause`.
**Why it happens:** D-03 specifies kind filtering in the select stage, but the DSL doesn't currently support it.
**How to avoid:** Either add `kind: Option<Vec<String>>` to `WhereClause`, or omit the kind filter and accept that non-function symbols pass through `compute_metric` (which gracefully skips them with eprintln warnings when no function body is found at that line).
**Warning signs:** `cargo test` passes but compute_metric emits many "no function body at line N" warnings for class/variable nodes.

### Pitfall 4: match_pattern Emits All Captures, Not Just Named Ones
**What goes wrong:** A tree-sitter query with multiple captures emits one PipelineNode per capture — including captures on intermediate nodes (not just the finding location).
**Why it happens:** [VERIFIED: `src/graph/executor.rs:764-779`] `execute_match_pattern` loops over `m.captures` and emits a PipelineNode for EVERY capture in the match.
**How to avoid:** Design queries with exactly one meaningful capture (the finding site). If multiple captures are needed for tree navigation, use the tree-sitter `#capture-name` convention and filter in the query itself. Or accept that multiple PipelineNodes per match will be emitted.
**Warning signs:** Duplicate findings at different lines for the same code pattern.

### Pitfall 5: Python sync_blocking Uses GraphPipeline, Not Pipeline
**What goes wrong:** The Rust `sync_blocking_in_async.py` implements `GraphPipeline` (uses `GraphPipelineContext`) rather than `Pipeline`. Deleting it and verifying `cargo test` requires confirming the factory function actually references it.
**Why it happens:** Python pipeline directory uses a mix of `Pipeline` and `GraphPipeline` implementations.
**How to avoid:** Check `src/audit/pipelines/python/mod.rs` scalability_pipelines() factory before assuming standard deletion pattern.
**Warning signs:** `cargo build` error after deletion: factory function references deleted type.

### Pitfall 6: mod.rs Declarations Must Be Removed
**What goes wrong:** Deleting `cyclomatic.rs` but leaving `pub mod cyclomatic;` in `mod.rs` causes a compile error.
**Why it happens:** Rust module system requires mod.rs to declare all submodules.
**How to avoid:** Remove the `pub mod <name>;` line from each language's `mod.rs` in the same commit as deleting the file.
**Warning signs:** `cargo build` error: "file not found for module".

### Pitfall 7: Go sync_blocking_in_async Is Not Async/Await
**What goes wrong:** Using a TypeScript/Python-style "async function wrapper" query for Go. Go has goroutines, not async/await.
**Why it happens:** The Rust Go pipeline detects blocking calls inside goroutines (`go func_literal`), not inside async function declarations.
**How to avoid:** The Go JSON pipeline's match_pattern should query for blocking calls inside `func_literal` bodies when that literal appears as the argument to a `go_statement` — or simply match the blocking calls anywhere in `.go` files (accepting higher false positives).
**Warning signs:** Zero findings on Go code with obvious blocking goroutine calls.

---

## Code Examples

### Existing ControlFlowConfig Thresholds (from metrics.rs)

[VERIFIED: `src/graph/metrics.rs:219-487`] TypeScript/JavaScript decision_point_kinds:
```
"if_statement", "for_statement", "for_in_statement", "while_statement",
"do_statement", "switch_case", "catch_clause"
```

Python (uses `and`/`or` as logical operators, `boolean_operator` as binary kind):
```
decision_point_kinds: if_statement, elif_clause, for_statement, while_statement, except_clause, with_statement
logical_operators: "and", "or"
binary_expression_kind: "boolean_operator"
```

Rust (no ternary, uses match arms):
```
decision_point_kinds: if_expression, for_expression, while_expression, loop_expression, match_arm
nesting_increments adds: closure_expression
```

### Rust n_plus_one_queries S-expressions (source templates)

[VERIFIED: `src/audit/pipelines/rust/n_plus_one_queries.rs:44-49`]
```
(for_expression body: (block) @loop_body) @loop_expr
(while_expression body: (block) @loop_body) @loop_expr
(loop_expression body: (block) @loop_body) @loop_expr
```

[VERIFIED: `src/audit/pipelines/rust/n_plus_one_queries.rs:53-59`]
```
(call_expression
  function: (field_expression
    field: (field_identifier) @method_name)) @call
```

### TypeScript match_pattern for async function detection

[VERIFIED: `src/audit/pipelines/typescript/sync_blocking_in_async.rs:111-131`]
The Rust version walks parent chain checking for `function_declaration`/`arrow_function`/`function_expression`/`method_definition` and then checks if the node text starts with "async". This logic is NOT expressible in a single S-expression without text predicates (`#match?`).

A simplified match_pattern for TypeScript that captures readFileSync/writeFileSync inside any function:
```
(call_expression
  function: (member_expression
    property: (property_identifier) @method)
  (#match? @method "Sync$")) @call
```
Note: `#match?` is a tree-sitter predicate. Whether `execute_match_pattern` supports predicates depends on whether `tree_sitter::Query::new` handles them — it does at the Query compilation level, but the `matches` iteration may or may not filter them. [ASSUMED: predicates are processed by tree-sitter's query engine during match iteration — needs verification by running a test]

### Integration Test Pattern (existing)

[VERIFIED: `tests/audit_json_integration.rs:24-47`]
```rust
#[test]
fn cyclomatic_complexity_finds_complex_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = r#"function complex(x) {
        if (x > 1) {} if (x > 2) {} if (x > 3) {} // ... 11 ifs
    }"#;
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(findings.iter().any(|f| f.pipeline == "cyclomatic_complexity"));
}
```

The `PipelineSelector::Complexity` variant calls `pipeline::complexity_pipelines_for_language(*lang)?` in the engine. After Rust complexity pipelines are deleted and replaced by JSON, the JSON pipelines run through the `json_audits` path instead, with the Rust pipeline suppressed by name-match.

---

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | Rust built-in (`cargo test`) |
| Config file | `Cargo.toml` |
| Quick run command | `cargo test --test audit_json_integration 2>&1 | tail -20` |
| Full suite command | `cargo test 2>&1 | tail -30` |

### Phase Requirements → Test Map
| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| TECH-01 | cyclomatic_complexity JSON finds complex functions | integration | `cargo test cyclomatic_complexity --test audit_json_integration` | ❌ Wave 0 |
| TECH-01 | function_length JSON finds long functions | integration | `cargo test function_length --test audit_json_integration` | ❌ Wave 0 |
| TECH-01 | cognitive_complexity JSON finds cognitively complex functions | integration | `cargo test cognitive_complexity --test audit_json_integration` | ❌ Wave 0 |
| TECH-01 | comment_to_code_ratio JSON finds under-documented files | integration | `cargo test comment_to_code_ratio --test audit_json_integration` | ❌ Wave 0 |
| SCAL-01 | n_plus_one_queries JSON finds call-in-loop | integration | `cargo test n_plus_one --test audit_json_integration` | ❌ Wave 0 |
| SCAL-01 | sync_blocking_in_async JSON finds blocking calls in async | integration | `cargo test sync_blocking --test audit_json_integration` | ❌ Wave 0 |
| TEST-02 | Full test suite passes after all Rust deletions | regression | `cargo test` | ✅ Exists |

### Sampling Rate
- **Per task commit:** `cargo test --test audit_json_integration 2>&1 | tail -20`
- **Per wave merge:** `cargo test 2>&1 | tail -30`
- **Phase gate:** Full suite green before `/gsd-verify-work`

### Wave 0 Gaps
- [ ] New test functions in `tests/audit_json_integration.rs` — covers TECH-01, SCAL-01
- [ ] No new test files needed — extend existing `tests/audit_json_integration.rs`

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Per-language Rust cyclomatic.rs (9 files) | Single cross-language `cyclomatic_complexity.json` | Phase 3 | Reduces 9 files to 1; runtime language dispatch via `control_flow_config_for_language` |
| Per-language Rust n_plus_one_queries.rs (9 files) | Single cross-language `n_plus_one_queries.json` | Phase 3 | Reduces 9 files to 1; loses DB-name specificity |
| Per-language Rust sync_blocking_in_async.rs | ~5 per-language JSON files | Phase 3 | Files exist for languages with async constructs only |
| Pipeline name collision allowed | ENG-01: Rust suppressed by JSON name-match | Phase 1 | Engine deduplicates automatically at line 111 of engine.rs |

---

## Open Questions (RESOLVED)

1. **WhereClause Metric Predicate Extension (BLOCKING)**
   - What we know: `WhereClause` has `count`, `depth`, `cycle_size`, `edge_count`, `ratio` — not `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`
   - What's unclear: Should the planner include a Rust code change to `pipeline.rs` to add these fields as a prerequisite task in Wave 0?
   - Recommendation: Include a small Rust task to add 4 named fields to `WhereClause`: `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio` — each `Option<NumericPredicate>`. Add matching arms to `eval_metrics()`. This is ~20 lines of Rust code and unblocks the JSON design.
   - **RESOLVED:** Plan 03-01 Task 1 adds `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio` as `Option<NumericPredicate>` fields to `WhereClause` in `src/graph/pipeline.rs` with matching `eval_metrics()` arms.

2. **select Stage Kind Filter**
   - What we know: D-03 specifies `{"select": "symbol", "kind": ["function", "method", "arrow_function"]}` but `kind` is not a `WhereClause` field
   - What's unclear: Should `kind` be added to `WhereClause`, or should we rely on `execute_compute_metric`'s graceful skip for non-function symbols?
   - Recommendation: Add `kind: Option<Vec<String>>` to `WhereClause` in the same Wave 0 Rust task as the metric predicates.
   - **RESOLVED:** Plan 03-01 Task 1 also adds `kind: Option<Vec<String>>` to `WhereClause` with a matching arm in `eval()`. Plans 03-02/03-03 use `"where": {"kind": ["function", "method", "arrow_function"]}` in select stages.

3. **match_pattern Predicate Support (#match?, #eq?)**
   - What we know: Tree-sitter queries support predicates like `#match?` and `#eq?` at the `Query` compilation level. Test `test_match_pattern_finds_panic_in_rust` uses `(#eq? @name "panic")` — if that test passes, predicates ARE being evaluated.
   - What's unclear: Does the current `execute_match_pattern` implementation filter by predicates?
   - Recommendation: Check whether `test_match_pattern_finds_panic_in_rust` passes.
   - **RESOLVED:** `#eq?` predicates are confirmed to work (Phase 2 tests green). `#match?` (regex) predicates are attempted in Plan 03-03 sync_blocking_in_async queries with a documented conditional fallback to simpler queries if `#match?` does not compile/filter correctly.

4. **comment_to_code_ratio — Threshold Encoding in executor**
   - What we know: `execute_compute_metric` for `comment_to_code_ratio` stores value as integer percentage: `(comment_lines / (comment_lines + code_lines) * 100) as i64`
   - What's unclear: Thresholds from the Rust implementation are `ratio < 0.05` (under) and `ratio > 0.60` (over). In the JSON, these become `{"lt": 5}` (under) and `{"gt": 60}` (over) once the `comment_to_code_ratio` WhereClause field is added.
   - Recommendation: Document this encoding in the JSON pipeline's description.
   - **RESOLVED:** Plan 03-02 `comment_to_code_ratio.json` uses `{"lt": 5}` and `{"gt": 60}` thresholds (integer percentage scale). The JSON `"description"` field documents the 0-100 encoding explicitly.

5. **Go sync_blocking — Goroutine vs. Async**
   - What we know: Go has no `async`/`await`. Go's `sync_blocking_in_async` Rust pipeline detects blocking calls inside goroutine bodies (`go_statement > func_literal`).
   - What's unclear: Is the match_pattern DSL expressive enough to capture "call inside goroutine body"?
   - Recommendation: Attempt `(go_statement (func_literal body: (_) (call_expression) @call))` — if this works in the Go grammar, include `sync_blocking_in_async_go.json`. If not, omit Go.
   - **RESOLVED:** Plan 03-03 includes `sync_blocking_in_async_go.json` using the goroutine containment query. If the query fails at runtime, the plan includes a documented fallback (simpler call_expression capture with higher false positive rate, or omit Go entirely per D-07 planner discretion).

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `#eq?` predicates in match_pattern are evaluated during query execution (based on test existence) | Code Examples | If wrong, `n_plus_one_queries.json` cannot use `#eq?` for method name matching — must accept broader patterns |
| A2 | Phase 2 is fully complete and `compute_metric`/`match_pattern` stages pass `cargo test` | Standard Stack | If Phase 2 tests are failing, Phase 3 is blocked until fixed |

---

## Environment Availability

Step 2.6: SKIPPED — this is a pure code/JSON migration phase. No external services, CLI tools, or databases required beyond Rust toolchain.

---

## Sources

### Primary (HIGH confidence — verified by direct code inspection)
- `src/graph/executor.rs` — `execute_compute_metric` (line 789), `execute_match_pattern` (line 714), `run_pipeline` signature with `workspace: Option<&Workspace>` parameter
- `src/graph/metrics.rs` — all 4 compute functions + per-language configs + `function_node_kinds_for_language` + `body_field_for_language`
- `src/graph/pipeline.rs` — `WhereClause` struct (lines 107-143), `GraphStage` enum (lines 453-487), `interpolate_message` (line 496), `FlagConfig.resolve_severity`
- `src/audit/engine.rs` — ENG-01 suppression logic (line 111), `PipelineSelector::Complexity` dispatch (line 98)
- `src/audit/pipelines/typescript/cyclomatic.rs` — CC threshold = 10, FUNCTION_QUERY S-expression
- `src/audit/pipelines/typescript/function_length.rs` — line threshold = 50, statement threshold = 20
- `src/audit/pipelines/typescript/cognitive.rs` — cognitive threshold = 15
- `src/audit/pipelines/typescript/comment_ratio.rs` — under = 0.05, over = 0.60; file-level (not per-function)
- `src/audit/pipelines/typescript/n_plus_one_queries.rs` — DB_METHOD_NAMES, DB_OBJ_METHOD_PAIRS, ARRAY_LOOP_METHODS, BARE_CALL_PATTERNS
- `src/audit/pipelines/typescript/sync_blocking_in_async.rs` — SYNC_SUFFIX_METHODS, BLOCKING_OBJ_METHOD_PAIRS
- `src/audit/pipelines/rust/sync_blocking_in_async.rs` — BLOCKING_SCOPED_PREFIXES, scoped_call_query pattern
- `src/audit/pipelines/python/sync_blocking_in_async.rs` — BLOCKING_ATTR_CALLS, BLOCKING_BARE_CALLS; uses GraphPipeline
- `src/audit/pipelines/go/sync_blocking_in_async.rs` — BLOCKING_CALLS, goroutine/channel pattern
- `src/audit/pipelines/typescript/mod.rs` — `complexity_pipelines()` and `scalability_pipelines()` factory functions
- `src/audit/builtin/module_size_distribution_rust.json` — reference DSL for flag + severity_map
- `tests/audit_json_integration.rs` — test pattern and imports

### Secondary (MEDIUM confidence)
- Phase 2 CONTEXT.md decisions D-01 through D-09 — implementation decisions carried into Phase 3 as established facts
- Phase 3 CONTEXT.md decisions D-01 through D-10 — locked user decisions

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — verified directly in source
- Architecture: HIGH — executor.rs implementation inspected line by line
- Pitfalls: HIGH — derived from actual code inspection, not speculation
- WhereClause gap: HIGH — verified absence of cyclomatic_complexity field in pipeline.rs

**Research date:** 2026-04-16
**Valid until:** 2026-05-16 (stable — no external dependencies)
