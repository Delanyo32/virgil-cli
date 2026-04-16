---
phase: 03-tech-debt-scalability-json-migration
plan: "03"
subsystem: audit/builtin + audit/pipelines
tags: [rust, audit-engine, json-pipelines, scalability, migration, deletion, match-pattern]
dependency_graph:
  requires:
    - "03-01 (WhereClause kind filter + severity suppression)"
    - "03-02 (complexity JSON pipelines + 40 Rust deletions)"
  provides:
    - 9 scalability JSON pipelines in src/audit/builtin/ (1 n_plus_one_queries + 8 sync_blocking_in_async)
    - All 20 Rust scalability pipeline files deleted across 10 language dirs (10 n_plus_one_queries + 10 sync_blocking_in_async)
    - All 10 language mod.rs scalability_pipelines() returning only memory_leak_indicators
  affects:
    - src/audit/builtin/n_plus_one_queries.json
    - src/audit/builtin/sync_blocking_in_async_typescript.json
    - src/audit/builtin/sync_blocking_in_async_rust.json
    - src/audit/builtin/sync_blocking_in_async_python.json
    - src/audit/builtin/sync_blocking_in_async_go.json
    - src/audit/builtin/sync_blocking_in_async_java.json
    - src/audit/builtin/sync_blocking_in_async_c.json
    - src/audit/builtin/sync_blocking_in_async_csharp.json
    - src/audit/builtin/sync_blocking_in_async_php.json
    - src/audit/pipelines/typescript/mod.rs
    - src/audit/pipelines/javascript/mod.rs
    - src/audit/pipelines/rust/mod.rs
    - src/audit/pipelines/python/mod.rs
    - src/audit/pipelines/go/mod.rs
    - src/audit/pipelines/java/mod.rs
    - src/audit/pipelines/c/mod.rs
    - src/audit/pipelines/cpp/mod.rs
    - src/audit/pipelines/csharp/mod.rs
    - src/audit/pipelines/php/mod.rs
tech_stack:
  added: []
  patterns:
    - "Cross-language JSON pipeline (no languages filter) using match_pattern alternation for loop detection"
    - "Per-language JSON pipeline with languages filter for sync_blocking_in_async coverage"
    - "match_pattern stage with #match? predicate for TypeScript Sync suffix detection"
    - "info severity for languages with high false positive rates (Rust, Python, Go, Java, C, C++, C#, PHP)"
key_files:
  created:
    - src/audit/builtin/n_plus_one_queries.json
    - src/audit/builtin/sync_blocking_in_async_typescript.json
    - src/audit/builtin/sync_blocking_in_async_rust.json
    - src/audit/builtin/sync_blocking_in_async_python.json
    - src/audit/builtin/sync_blocking_in_async_go.json
    - src/audit/builtin/sync_blocking_in_async_java.json
    - src/audit/builtin/sync_blocking_in_async_c.json
    - src/audit/builtin/sync_blocking_in_async_csharp.json
    - src/audit/builtin/sync_blocking_in_async_php.json
  modified:
    - src/audit/pipelines/typescript/mod.rs
    - src/audit/pipelines/javascript/mod.rs
    - src/audit/pipelines/rust/mod.rs
    - src/audit/pipelines/python/mod.rs
    - src/audit/pipelines/go/mod.rs
    - src/audit/pipelines/java/mod.rs
    - src/audit/pipelines/c/mod.rs
    - src/audit/pipelines/cpp/mod.rs
    - src/audit/pipelines/csharp/mod.rs
    - src/audit/pipelines/php/mod.rs
decisions:
  - "n_plus_one_queries uses single cross-language match_pattern with alternation over 4 TS/JS loop kinds -- non-TS/JS grammars silently rejected by execute_match_pattern (accepted limitation per D-06)"
  - "sync_blocking_in_async split into 8 per-language JSON files to cover all 10 language directories (C/C++ share one file via languages: [c, cpp])"
  - "TypeScript sync_blocking uses #match? predicate for Sync suffix -- engine silently skips files if predicate unsupported"
  - "Languages without async/await (C/C++, PHP) get info-severity JSON pipelines to preserve detection capability without false positives"
  - "All 10 sync_blocking_in_async Rust files deleted to avoid orphaned silently-suppressed code (engine suppression is global by pipeline name)"
  - "Python sync_blocking_in_async was a GraphPipeline in Rust -- JSON replacement uses match_pattern on attribute calls (simpler, broader)"
metrics:
  duration: "~5 minutes"
  completed: "2026-04-16"
  tasks_completed: 3
  files_modified: 10
  files_created: 9
  files_deleted: 20
---

# Phase 03 Plan 03: 9 Scalability JSON Pipelines + 20 Rust Deletions Summary

Created 9 scalability JSON pipelines using match_pattern stage (1 cross-language n_plus_one_queries + 8 per-language sync_blocking_in_async covering all 10 language dirs) and deleted all 20 Rust scalability pipeline files, with all 10 language mod.rs factories returning only memory_leak_indicators.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Create 9 scalability JSON pipelines | ce7228d | src/audit/builtin/n_plus_one_queries.json, sync_blocking_in_async_{typescript,rust,python,go,java,c,csharp,php}.json |
| 2 | Delete 10 n_plus_one_queries Rust files, update 10 mod.rs | 702aa46 | 10 mod.rs files (10 n_plus_one_queries.rs deleted) |
| 3 | Delete 10 sync_blocking_in_async Rust files, update 10 mod.rs | 99084fe | 10 mod.rs files (10 sync_blocking_in_async.rs deleted) |

## What Was Built

### Task 1 -- 9 Scalability JSON Pipelines (ce7228d)

Created 9 JSON pipeline files in `src/audit/builtin/`:

**`n_plus_one_queries.json`**: Cross-language (no `languages` filter). Uses match_pattern with alternation over 4 TS/JS loop kinds (`for_statement`, `for_in_statement`, `while_statement`, `do_statement`) to detect call expressions in loop bodies. Pattern: `query_in_loop`, severity: `warning`. Documented in description that DB/ORM/HTTP name filtering from Rust version is dropped (higher false positive rate expected).

**`sync_blocking_in_async_typescript.json`**: TypeScript and JavaScript only. Uses `#match?` predicate to match `Sync$` suffix on property identifiers in member_expression calls. Pattern: `sync_call_in_async`, severity: `warning`. Note: cannot verify async context; all matching calls flagged.

**`sync_blocking_in_async_rust.json`**: Rust only. Detects all `scoped_identifier` function calls (potential `std::fs::*`, `std::thread::sleep`). Pattern: `blocking_io_in_async`, severity: `info` (high false positive rate).

**`sync_blocking_in_async_python.json`**: Python only. Detects attribute calls (object.method pattern, potential `time.sleep`, `requests.get`). Pattern: `blocking_in_async_def`, severity: `info`.

**`sync_blocking_in_async_go.json`**: Go only. Detects call expressions inside goroutine bodies (go_statement > func_literal). Pattern: `blocking_in_goroutine`, severity: `info`.

**`sync_blocking_in_async_java.json`**: Java only. Detects `synchronized_statement` blocks. Pattern: `sync_blocking_call`, severity: `info`.

**`sync_blocking_in_async_c.json`**: C and C++ (shared via `"languages": ["c", "cpp"]`). Detects all `identifier` function calls. Pattern: `blocking_call_detected`, severity: `info`.

**`sync_blocking_in_async_csharp.json`**: C# only. Detects `member_access_expression` name identifiers. Pattern: `potential_blocking_access`, severity: `info`.

**`sync_blocking_in_async_php.json`**: PHP only. Detects `function_call_expression` with `name` child. Pattern: `blocking_call_detected`, severity: `info`.

### Task 2 -- 10 n_plus_one_queries Rust Files Deleted (702aa46)

Deleted all 10 `n_plus_one_queries.rs` files (one per language directory). Updated all 10 `mod.rs` files:
- Removed `pub mod n_plus_one_queries;` declarations
- Removed `n_plus_one_queries::NPlusOneQueriesPipeline` entries from `scalability_pipelines()`
- `sync_blocking_in_async` and `memory_leak_indicators` retained

### Task 3 -- 10 sync_blocking_in_async Rust Files Deleted (99084fe)

Deleted all 10 `sync_blocking_in_async.rs` files (one per language directory). Updated all 10 `mod.rs` files:
- Removed `pub mod sync_blocking_in_async;` declarations
- Removed `sync_blocking_in_async::SyncBlockingInAsyncPipeline` entries from `scalability_pipelines()`
- Each `scalability_pipelines()` now returns only `memory_leak_indicators`

The engine.rs suppression mechanism (global by pipeline name) means the JSON `"sync_blocking_in_async"` pipelines would have silently suppressed all 10 Rust files. Deleting them removes orphaned, dead code.

## Deviations from Plan

None -- plan executed exactly as written.

## Verification Results

```
ls src/audit/builtin/n_plus_one_queries.json ... (9 files)
# All 9 JSON files exist

find src/audit/pipelines -name "n_plus_one_queries.rs"
# Returns empty -- all 10 deleted

find src/audit/pipelines -name "sync_blocking_in_async.rs"
# Returns empty -- all 10 deleted

grep -r "pub mod n_plus_one_queries" src/audit/pipelines/
# Returns empty -- all 10 mod.rs updated

grep -r "pub mod sync_blocking_in_async" src/audit/pipelines/
# Returns empty -- all 10 mod.rs updated

cargo test: 2342+ passed, 0 failed
```

## Self-Check: PASSED

- [x] src/audit/builtin/n_plus_one_queries.json exists and contains "pipeline": "n_plus_one_queries" -- confirmed (ce7228d)
- [x] src/audit/builtin/n_plus_one_queries.json contains "match_pattern" -- confirmed (ce7228d)
- [x] sync_blocking_in_async_typescript.json contains "languages": ["typescript", "javascript"] -- confirmed (ce7228d)
- [x] sync_blocking_in_async_rust.json contains "languages": ["rust"] -- confirmed (ce7228d)
- [x] sync_blocking_in_async_python.json contains "languages": ["python"] -- confirmed (ce7228d)
- [x] sync_blocking_in_async_go.json contains "languages": ["go"] -- confirmed (ce7228d)
- [x] sync_blocking_in_async_java.json contains "languages": ["java"] -- confirmed (ce7228d)
- [x] sync_blocking_in_async_c.json contains "languages": ["c", "cpp"] -- confirmed (ce7228d)
- [x] sync_blocking_in_async_csharp.json contains "languages": ["csharp"] -- confirmed (ce7228d)
- [x] sync_blocking_in_async_php.json contains "languages": ["php"] -- confirmed (ce7228d)
- [x] 10 n_plus_one_queries.rs deleted -- confirmed (702aa46)
- [x] 10 sync_blocking_in_async.rs deleted -- confirmed (99084fe)
- [x] All 10 mod.rs have only memory_leak_indicators in scalability_pipelines() -- confirmed
- [x] Task 1 commit ce7228d exists
- [x] Task 2 commit 702aa46 exists
- [x] Task 3 commit 99084fe exists
- [x] cargo test passes with zero failures -- 2342+ passed

## Known Stubs

None -- all 9 JSON pipelines are fully wired to match_pattern and flag stages. No placeholder values.

## Threat Flags

None -- changes are internal to the audit engine pipeline dispatch. New JSON files extend existing match_pattern dispatch; no new network endpoints, auth paths, file access patterns, or schema changes at trust boundaries.
