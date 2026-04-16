---
phase: 02-executor-stage-implementation
plan: "01"
subsystem: graph
tags: [metrics, pipeline-dsl, graph-executor, refactor]
dependency_graph:
  requires: []
  provides: [graph::metrics module, ControlFlowConfig re-exports, GraphStage::MatchPattern, GraphStage::ComputeMetric]
  affects: [src/graph/executor.rs, src/audit/pipelines/helpers.rs, src/graph/pipeline.rs]
tech_stack:
  added: []
  patterns: [re-export for backward compatibility, per-language config dispatch]
key_files:
  created:
    - src/graph/metrics.rs
  modified:
    - src/graph/mod.rs
    - src/graph/pipeline.rs
    - src/graph/executor.rs
    - src/audit/pipelines/helpers.rs
decisions:
  - "Moved ControlFlowConfig and compute functions from audit::pipelines::helpers to graph::metrics for executor access; re-exported from helpers for backward compatibility"
  - "body_field_for_language returns 'body' for all languages (all supported languages use the same field name)"
  - "executor.rs updated alongside pipeline.rs to maintain compile correctness — stale match arms replaced with MatchPattern/ComputeMetric stubs"
metrics:
  duration: "~10 minutes"
  completed: "2026-04-16"
  tasks_completed: 2
  tasks_total: 2
  files_modified: 5
---

# Phase 2 Plan 01: Metrics Module and Pipeline DSL Update Summary

Established the `graph::metrics` module containing metric computation functions and per-language control flow configs, and updated the `GraphStage` enum with two new stage variants while removing five dead stub variants.

## Tasks Completed

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Create src/graph/metrics.rs and declare module | 966aee5 | src/graph/metrics.rs (new), src/graph/mod.rs, src/audit/pipelines/helpers.rs |
| 2 | Update GraphStage enum — add MatchPattern and ComputeMetric, delete 5 stub variants | 7dc2cca | src/graph/pipeline.rs, src/graph/executor.rs |

## What Was Built

**Task 1: graph::metrics module**

Created `src/graph/metrics.rs` as the canonical home for metric computation functions used by the graph executor's `compute_metric` stage. The module contains:

- `ControlFlowConfig` struct (moved from `audit::pipelines::helpers`)
- `compute_cyclomatic()`, `compute_cognitive()`, `count_function_lines()`, `compute_comment_ratio()` (moved verbatim)
- Private helpers `count_statements()`, `walk_all()` (moved, kept private)
- `control_flow_config_for_language(lang: Language)` dispatcher matching all 10 language variants
- 10 private per-language config functions (`ts_config`, `js_config`, `rust_config`, `python_config`, `go_config`, `java_config`, `c_config`, `cpp_config`, `csharp_config`, `php_config`)
- `function_node_kinds_for_language()` — per-language function AST node kinds
- `body_field_for_language()` — returns `"body"` for all languages (universal field name)

`src/audit/pipelines/helpers.rs` was updated to replace the moved definitions with `pub use crate::graph::metrics::{ControlFlowConfig, compute_cyclomatic, compute_cognitive, count_function_lines, compute_comment_ratio}` re-exports preserving backward compatibility. All 2559 existing tests passed after this change.

**Task 2: GraphStage enum update**

Updated `src/graph/pipeline.rs`:

- Removed 5 dead config structs: `TraverseConfig`, `CountEdgesConfig`, `FilterConfig`, `MatchNameConfig`, `PairConfig`
- Removed 5 dead variants from `GraphStage`: `Traverse`, `CountEdges`, `Filter`, `MatchName`, `Pair`
- Added 2 new variants before `Flag` (serde untagged — order matters for deserialization):
  - `MatchPattern { match_pattern: String }` — tree-sitter S-expression query string
  - `ComputeMetric { compute_metric: String }` — named metric computation (e.g., "cyclomatic_complexity")
- Removed 3 deleted deserialization tests, added 2 new tests for the new variants
- `Flag` remains the last variant as required for serde untagged deserialization correctness

Updated `src/graph/executor.rs` to remove stale match arms for the deleted variants and add placeholder stubs for `MatchPattern` and `ComputeMetric` (marked TODO for Phase 2 Plan 02).

## Deviations from Plan

**1. [Rule 3 - Blocking] Updated executor.rs alongside pipeline.rs**

- **Found during:** Task 2
- **Issue:** Deleting 5 GraphStage variants from pipeline.rs would cause compile errors in executor.rs (which had match arms for all deleted variants). The plan noted this was expected "until Plan 02", but the build would be broken between plans in the same wave.
- **Fix:** Replaced the 5 stale match arms (`Traverse`, `Filter`, `MatchName`, `CountEdges`, `Pair`) with 2 new stubs for `MatchPattern` and `ComputeMetric`. Both marked `// TODO: implement ... (Phase 2)`.
- **Files modified:** src/graph/executor.rs
- **Commit:** 7dc2cca

## Verification Results

```
cargo build: OK (1 pre-existing unused-variable warning, unrelated)
cargo test --lib graph::pipeline::tests: 41 passed, 0 failed
cargo test --lib: 2558 passed, 0 failed (net -1 from removed tests: -3 old + 2 new)
grep -c "pub use crate::graph::metrics" helpers.rs: 1
grep -c "pub mod metrics" graph/mod.rs: 1
```

## Known Stubs

- `executor.rs: GraphStage::MatchPattern { .. } => Ok(nodes)` — tree-sitter pattern matching not yet implemented; placeholder for Plan 02
- `executor.rs: GraphStage::ComputeMetric { .. } => Ok(nodes)` — metric computation not yet implemented; placeholder for Plan 02

These stubs are intentional. Plan 02 (`02-02-PLAN.md`) implements `execute_match_pattern` and `execute_compute_metric` in executor.rs.

## Self-Check: PASSED

- [x] src/graph/metrics.rs exists
- [x] src/graph/mod.rs contains `pub mod metrics`
- [x] src/audit/pipelines/helpers.rs contains `pub use crate::graph::metrics`
- [x] src/graph/pipeline.rs contains `MatchPattern { match_pattern: String`
- [x] src/graph/pipeline.rs contains `ComputeMetric { compute_metric: String`
- [x] Commits 966aee5 and 7dc2cca exist in git log
- [x] cargo test --lib: 2558 passed, 0 failed
