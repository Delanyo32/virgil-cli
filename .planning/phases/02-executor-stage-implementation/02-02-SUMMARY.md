---
phase: 02-executor-stage-implementation
plan: "02"
subsystem: graph/executor
tags: [executor, tree-sitter, metrics, match_pattern, compute_metric, audit-engine]
dependency_graph:
  requires: [02-01]
  provides: [run_pipeline-workspace, execute_match_pattern, execute_compute_metric]
  affects: [src/audit/engine.rs, src/query_engine.rs, src/main.rs]
tech_stack:
  added: []
  patterns: [StreamingIterator for tree-sitter 0.25 QueryMatches, workspace-threaded executor, per-language query compilation with skip-on-error]
key_files:
  created: []
  modified:
    - src/graph/executor.rs
    - src/audit/engine.rs
    - src/query_engine.rs
    - src/main.rs
decisions:
  - run_pipeline gains workspace+pipeline_languages params before seed_nodes for natural API ordering
  - execute_graph_pipeline backward-compat wrapper passes None/None for workspace/pipeline_languages
  - match_pattern skips files where query compilation fails (different grammars)
  - Tree-sitter query syntax uses (macro_invocation (identifier) @name) not name: field syntax
  - comment_to_code_ratio operates on whole file root, not per-function body
metrics:
  duration: ~25 minutes
  completed: "2026-04-16"
  tasks_completed: 2
  files_modified: 4
---

# Phase 02 Plan 02: Executor Stage Implementation Summary

Implemented the `match_pattern` and `compute_metric` executor stages in `src/graph/executor.rs`, wired workspace and pipeline_languages into the `run_pipeline` call site in `src/audit/engine.rs`, and updated all other call sites across the codebase.

## What Was Built

**`execute_match_pattern`** — source stage that iterates all workspace files, applies an optional `pipeline_languages` filter before parsing (per D-02), compiles the tree-sitter S-expression query per language (skipping files where the query is grammatically invalid for that language), and emits `PipelineNode` entries for every capture. Uses `while let Some(m) = matches.next()` per the CLAUDE.md tree-sitter 0.25 hard constraint.

**`execute_compute_metric`** — transform stage that takes an existing node list and computes one of four metrics (`cyclomatic_complexity`, `cognitive_complexity`, `function_length`, `comment_to_code_ratio`) by re-parsing the source file and locating the function body at the node's start line. Metrics are stored in `node.metrics` as `MetricValue::Int`.

**`find_function_body_at_line`** — stack-based tree walker that locates a function node at a given line and returns its `body` child, using per-language node kind tables from `src/graph/metrics.rs`.

**Updated call sites:** `run_pipeline` in `src/query_engine.rs` (main + 2 test calls), `src/main.rs` (1 call), and `src/audit/engine.rs` (1 call) all updated to pass the new workspace and pipeline_languages arguments.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed invalid tree-sitter query syntax in unit tests**
- **Found during:** Task 1 test execution
- **Issue:** Plan's test query used `name: (identifier) @name` (named field syntax) which is invalid for the tree-sitter-rust 0.23/0.24 grammar. `Query::new` silently failed (returned Err), causing `execute_match_pattern` to skip the file and produce zero matches.
- **Fix:** Changed to `(macro_invocation (identifier) @name (#eq? @name "panic")) @call` — direct child capture syntax that works with the installed grammar version. Confirmed working via isolated binary test.
- **Files modified:** `src/graph/executor.rs` (both `test_match_pattern_finds_panic_in_rust` and `test_match_pattern_no_match_returns_empty`)
- **Commit:** 98d634a

## Test Results

- `cargo test --lib graph::executor`: 15/15 passed (11 pre-existing + 4 new)
- `cargo test` (full suite): 2562 lib + 8 integration + 8 doc = zero failures

## Known Stubs

None — `match_pattern` and `compute_metric` are fully implemented with real behavior.

## Threat Flags

None — all threat mitigations from the plan's threat model are implemented:
- T-02-04: `Query::new()` validates S-expression at compile time; `Err` → `continue` (no unwrap)
- T-02-06: Unknown metric names → `anyhow::bail!` with supported list

## Self-Check

### Created files exist
- No new files created (modifications only)

### Commits exist
- 98d634a: `feat(02-02): implement match_pattern and compute_metric executor stages`
- a3c15b5: `feat(02-02): update engine.rs to pass workspace and pipeline_languages to run_pipeline`

## Self-Check: PASSED
