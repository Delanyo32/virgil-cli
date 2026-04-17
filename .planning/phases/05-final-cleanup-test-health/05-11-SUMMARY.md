---
phase: 05-final-cleanup-test-health
plan: 11
subsystem: audit-pipeline-cleanup
tags: [cleanup, dead-code, helpers, taint-exceptions, rust-exceptions]
dependency_graph:
  requires: [05-01, 05-02, 05-03, 05-04, 05-05, 05-06, 05-07, 05-08, 05-09, 05-10]
  provides: [final-clean-audit-pipeline-layer]
  affects: [src/audit/pipelines/, src/audit/pipeline.rs]
tech_stack:
  added: []
  patterns: [dead-code-removal, permanent-exception-annotation]
key_files:
  created: []
  modified:
    - src/audit/pipelines/helpers.rs
    - src/audit/pipelines/mod.rs
    - src/audit/pipeline.rs
    - src/audit/pipelines/csharp/csharp_ssrf.rs
    - src/audit/pipelines/csharp/sql_injection.rs
    - src/audit/pipelines/csharp/xxe.rs
    - src/audit/pipelines/go/sql_injection.rs
    - src/audit/pipelines/go/ssrf_open_redirect.rs
    - src/audit/pipelines/java/java_ssrf.rs
    - src/audit/pipelines/java/sql_injection.rs
    - src/audit/pipelines/java/xxe.rs
    - src/audit/pipelines/javascript/xss_dom_injection.rs
    - src/audit/pipelines/javascript/ssrf.rs
    - src/audit/pipelines/php/sql_injection.rs
    - src/audit/pipelines/php/ssrf.rs
    - src/audit/pipelines/python/sql_injection.rs
    - src/audit/pipelines/python/ssrf.rs
  deleted:
    - src/audit/pipelines/rust/ (entire directory — mod.rs, primitives.rs)
    - src/audit/pipelines/c/mod.rs
    - src/audit/pipelines/cpp/mod.rs
decisions:
  - helpers.rs pruned from 1743 lines to ~250 lines — 8 live functions kept, ~49 dead pub fn deleted
  - pub use crate::graph::metrics::* re-export removed (no non-pipeline callers confirmed by grep)
  - pipeline.rs dispatch arms for rust/c/cpp removed; fall through to _ => Ok(vec![])
  - PERMANENT RUST EXCEPTION comment added to all 14 taint pipeline files (D-12 complete)
  - find_enclosing_function_callers retained — used by taint pipelines indirectly via graph
metrics:
  duration_minutes: 8
  completed_date: "2026-04-17"
  tasks_completed: 2
  files_modified: 19
  files_deleted: 4
---

# Phase 05 Plan 11: Final Cleanup Summary

Final cleanup after all 10 language migration plans: deleted dead language directories, pruned helpers.rs to only live functions, annotated 14 taint exception files, updated mod.rs and pipeline.rs. CLEAN-01, CLEAN-02, CLEAN-03 satisfied.

## What Was Built

**Dead language directories deleted (3):** `src/audit/pipelines/rust/`, `c/`, `cpp/` — all pipelines from these languages were migrated to JSON in Plans 01–10; only empty `mod.rs` stubs (returning `Ok(vec![])`) remained.

**helpers.rs pruned:** Reduced from 1743 lines (~50 public functions) to ~250 lines (8 live public functions). Functions confirmed dead via `grep -r "helpers::{fn_name}" src/ | grep -v "src/audit/pipelines/"` — zero non-pipeline results for all deleted functions.

**Surviving functions in helpers.rs:**
- `is_test_file` — called by `src/graph/executor.rs`
- `is_excluded_for_arch_analysis` — called by `src/graph/executor.rs`, `src/audit/analyzers/coupling.rs`
- `is_barrel_file` — called by `src/graph/executor.rs`, `src/audit/analyzers/coupling.rs`
- `count_all_identifier_occurrences` — called by `src/audit/engine.rs`
- `is_literal_node_go`, `is_literal_node_java`, `is_literal_node_csharp` — called by taint SQL injection pipelines
- `is_safe_expression`, `all_args_are_literals` — called by taint SQL injection pipelines
- `find_enclosing_function_callers` — retained for taint pipeline graph access

**pipeline.rs updated:** Removed `pipelines::rust::*`, `pipelines::c::*`, `pipelines::cpp::*` dispatch arms from all 5 `*_for_language()` match blocks. These now fall through to `_ => Ok(vec![])`.

**14 taint files annotated (D-12):** Added 3-line PERMANENT RUST EXCEPTION comment block at the top of each file documenting why they stay in Rust (FlowsTo/SanitizedBy graph predicates not expressible in match_pattern JSON DSL).

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | cadd983 | Delete dead lang dirs, prune helpers.rs, update mod.rs + pipeline.rs |
| Task 2 | f5b11eb | Add PERMANENT RUST EXCEPTION comments to 14 taint files |

## Verification Results

- `cargo test`: 1996 tests pass (518 unit + 1470 audit integration + 8 integration), zero failures
- `cargo build`: zero warnings anywhere in codebase
- Dead directories confirmed absent: `rust/`, `c/`, `cpp/`
- `pipelines/mod.rs` contains only 8 module declarations (no rust/c/cpp)
- `helpers.rs` does not contain `count_nodes_of_kind`, `hash_block_normalized`, or any of the ~49 deleted functions
- All 14 taint files have PERMANENT RUST EXCEPTION as first 3 lines

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] pipeline.rs still referenced deleted modules**
- **Found during:** Task 1 — before deleting directories, grep revealed `src/audit/pipeline.rs` references `pipelines::rust::*`, `pipelines::c::*`, `pipelines::cpp::*` in 5 dispatch functions
- **Issue:** Deleting directories without updating pipeline.rs would break compilation
- **Fix:** Removed all Rust/C/C++ dispatch arms from `pipelines_for_language()`, `complexity_pipelines_for_language()`, `code_style_pipelines_for_language()`, `security_pipelines_for_language()`, `scalability_pipelines_for_language()` — each now falls to `_ => Ok(vec![])`
- **Files modified:** `src/audit/pipeline.rs`
- **Commit:** cadd983

**2. [Rule 2 - Missing] helpers.rs had additional taint-pipeline callers beyond plan's 4**
- **Found during:** Task 1 — grep revealed `csharp/sql_injection.rs`, `go/sql_injection.rs`, `java/sql_injection.rs` import `all_args_are_literals`, `is_literal_node_{csharp,go,java}`, `is_safe_expression`
- **Issue:** Plan's RESEARCH.md listed only 4 survivor functions; 4 additional functions also have live callers in taint exception files
- **Fix:** Retained `is_literal_node_go`, `is_literal_node_java`, `is_literal_node_csharp`, `is_safe_expression`, `all_args_are_literals`, `find_enclosing_function_callers` in pruned helpers.rs
- **Files modified:** `src/audit/pipelines/helpers.rs`
- **Commit:** cadd983

**3. [Rule 1 - Cleanup] Dead pub use crate::graph::metrics::* re-export removed**
- **Found during:** Task 1 — grep confirmed zero non-pipeline callers of re-exported metrics functions
- **Fix:** Removed the `pub use crate::graph::metrics::*` block (lines 7-14 of original) and associated tests that tested those re-exported functions
- **Files modified:** `src/audit/pipelines/helpers.rs`
- **Commit:** cadd983

## Known Stubs

None — this plan only removes dead code and adds comments; no new functionality introduced.

## Threat Flags

None — no new network endpoints, auth paths, file access patterns, or schema changes introduced.

## Self-Check: PASSED

- `src/audit/pipelines/rust/` absent: confirmed
- `src/audit/pipelines/c/` absent: confirmed
- `src/audit/pipelines/cpp/` absent: confirmed
- `src/audit/pipelines/helpers.rs` present with live functions: confirmed
- `src/audit/pipelines/mod.rs` has 8 module declarations: confirmed
- Commits cadd983, f5b11eb exist in git log: confirmed
- All 14 taint files have PERMANENT RUST EXCEPTION comment: confirmed (16 total including primitives.rs from prior plans)
- 1996 tests pass, zero warnings: confirmed
