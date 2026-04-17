# Design: Pipeline Module Extraction + DSL Composability

**Date:** 2026-04-16
**Status:** Approved

## Goal

Extract the JSON pipeline layer into its own top-level module, delete all dead Rust pipeline infrastructure, and make the JSON DSL composable — generic metric predicates and decomposed taint stages.

## Background

virgil-cli was originally built with per-language Rust pipeline implementations. All analysis has since been migrated to JSON-driven pipelines. The codebase still carries the full legacy Rust pipeline layer (`audit/pipelines/`, `audit/pipeline.rs`, `audit/project_analyzer.rs`, `audit/analyzers/`, `audit/primitives.rs`) which is entirely unreachable — `engine.rs` explicitly states "No Rust pipelines remain." The JSON DSL structs live inside `graph/` despite having nothing to do with graph data structures. `WhereClause` has 11 hardcoded metric fields that require Rust schema changes to extend.

## Module Restructure

### New layout

```
src/
├── pipeline/               NEW — owns the entire JSON pipeline layer
│   ├── mod.rs
│   ├── dsl.rs              moved from graph/pipeline.rs
│   ├── executor.rs         moved from graph/executor.rs
│   └── loader.rs           moved from audit/json_audit.rs
│
├── audit/                  SLIMMED — orchestration + output only
│   ├── engine.rs           updated imports
│   ├── format.rs           unchanged
│   └── models.rs           unchanged
│
├── graph/                  SLIMMED — graph data structures + builder only
│   ├── mod.rs              unchanged (CodeGraph definition)
│   ├── builder.rs          unchanged
│   ├── taint.rs            unchanged (internal taint engine)
│   ├── metrics.rs          unchanged
│   ├── cfg.rs              unchanged
│   └── resource.rs         unchanged
│
└── (cli, main, language, workspace, s3, server — unchanged)
```

### Deleted files

| Path | Reason |
|------|--------|
| `src/audit/pipelines/` | All per-language Rust pipeline implementations |
| `src/audit/pipeline.rs` | Legacy trait hierarchy (`Pipeline`, `NodePipeline`, `GraphPipeline`, `AnyPipeline`) + category dispatch functions |
| `src/audit/analyzers/mod.rs` | Two functions returning `vec![]` |
| `src/audit/project_analyzer.rs` | `ProjectAnalyzer` trait with no implementations |
| `src/audit/primitives.rs` | Shared helpers for deleted Rust pipelines |
| `src/graph/pipeline.rs` | Content moved to `src/pipeline/dsl.rs` |
| `src/graph/executor.rs` | Content moved to `src/pipeline/executor.rs` |
| `src/audit/json_audit.rs` | Content moved to `src/pipeline/loader.rs` |

### Public API

`main.rs` and `server.rs` call `AuditEngine` and `run_pipeline` exactly as before. Only import paths change. No behavior change.

`lib.rs` exports update:
- Remove: `pub use audit::json_audit`, `pub use graph::executor`, `pub use graph::pipeline`
- Add: `pub mod pipeline` with re-exports of `pipeline::dsl`, `pipeline::executor`, `pipeline::loader`

## DSL Composability

### Generic metric predicates

**Problem:** `WhereClause` has 11 hardcoded named fields for computed metrics. Adding a new metric requires a Rust schema change.

**Fix:** Replace all 11 named metric `Option<NumericPredicate>` fields with a single `HashMap<String, NumericPredicate>` under a `metrics` key.

Affected fields moved under `metrics`:
`cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`, `efferent_coupling`, `afferent_coupling`, `count`, `cycle_size`, `depth`, `edge_count`, `ratio`

Non-metric fields stay as named fields: `is_test_file`, `is_generated`, `is_barrel_file`, `is_nolint`, `exported`, `kind`, `unreferenced`, `is_entry_point`, `and`, `or`, `not`. `unreferenced` and `is_entry_point` are boolean flags (not numeric thresholds) so they remain as `Option<bool>`, not in the `metrics` map.

**Rust before:**
```rust
pub cyclomatic_complexity: Option<NumericPredicate>,
pub function_length: Option<NumericPredicate>,
pub efferent_coupling: Option<NumericPredicate>,
// ...8 more
```

**Rust after:**
```rust
#[serde(default)]
pub metrics: HashMap<String, NumericPredicate>,
```

**JSON before:**
```json
{"when": {"cyclomatic_complexity": {"gte": 30}}}
```

**JSON after:**
```json
{"when": {"metrics": {"cyclomatic_complexity": {"gte": 30}}}}
```

All builtin JSON files that use `severity_map` `when` clauses are updated to use the `metrics` key. `eval_metrics` and `eval` on `WhereClause` iterate the `metrics` map rather than checking named fields.

`is_empty()` checks `metrics.is_empty()` instead of 11 separate `is_none()` calls.

### Taint stage decomposition

**Problem:** `taint` bundles source tracking, sanitizer definitions, and sink detection in one monolithic key. Every security pipeline re-declares identical source lists.

**Fix:** Split into three composable stages that accumulate into a shared taint context per pipeline run.

```
GraphStage::TaintSources  — declares taint sources (adds to context)
GraphStage::TaintSanitizers — declares sanitizers (adds to context)
GraphStage::TaintSinks    — runs taint analysis against declared sources/sanitizers
```

**JSON after:**
```json
[
  {"taint_sources": [{"pattern": "request.form", "kind": "user_input"}, ...]},
  {"taint_sanitizers": [{"pattern": "escape"}, ...]},
  {"taint_sinks": [{"pattern": "cursor.execute", "vulnerability": "sql_injection"}]},
  {"flag": {"pattern": "sql_injection", "message": "...", "severity": "error"}}
]
```

The existing `taint` combined stage is retained as a working alias — it is desugared by the executor into `TaintSources` + `TaintSanitizers` + `TaintSinks` before execution. This means no external pipeline files break.

All ~20 builtin taint JSON files (`sql_injection_*.json`, `ssrf_*.json`, `xss_*.json`, `xxe_*.json`) are migrated to the decomposed form.

**Executor change:** Introduce a `TaintContext` that accumulates sources and sanitizers as stages execute. `TaintSinks` reads from this context when it runs.

## Dead Code Removal

The Rust pipeline infrastructure in `audit/pipelines/` contains real analysis logic (tech debt, complexity, code style, security, scalability) for Go, Java, PHP, JavaScript, CSharp, and Python. It is entirely unreachable from `engine.rs` and is deleted in full — the JSON builtins cover the same ground.

## Documentation Updates

- **`CLAUDE.md`** — Update module layout section; add `pipeline/` as the authoritative DSL + executor module; remove references to legacy trait hierarchy and `audit/pipelines/`
- **`src/pipeline/dsl.rs`** — Module-level doc comment explaining the composable stage model
- **`src/pipeline/loader.rs`** — Short doc comment on discovery order (project-local → user-global → built-ins)

## Invariants Preserved

- All existing builtin audit JSON files produce identical findings after migration (same behavior, different JSON key names)
- `taint` combined stage continues to work (desugared by executor)
- `AuditEngine` public API unchanged
- `run_pipeline` public API unchanged
- All existing tests pass
