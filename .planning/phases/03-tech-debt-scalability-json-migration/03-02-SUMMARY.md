---
phase: 03-tech-debt-scalability-json-migration
plan: "02"
subsystem: audit/builtin + audit/pipelines
tags: [rust, audit-engine, json-pipelines, complexity, migration, deletion]
dependency_graph:
  requires:
    - "03-01 (WhereClause kind filter + severity suppression)"
  provides:
    - 4 cross-language complexity JSON pipelines in src/audit/builtin/
    - All 40 Rust complexity pipeline files deleted across 10 language dirs
    - All 10 language mod.rs complexity_pipelines() returning empty vec
  affects:
    - src/audit/builtin/cyclomatic_complexity.json
    - src/audit/builtin/function_length.json
    - src/audit/builtin/cognitive_complexity.json
    - src/audit/builtin/comment_to_code_ratio.json
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
    - "Cross-language JSON pipeline (no languages filter) replacing 10 language-specific Rust files"
    - "severity_map-only flag config (no bare severity) enabling threshold suppression via 03-01 resolve_severity"
    - "select: file for file-level metrics vs select: symbol for function-level metrics"
key_files:
  created:
    - src/audit/builtin/cyclomatic_complexity.json
    - src/audit/builtin/function_length.json
    - src/audit/builtin/cognitive_complexity.json
    - src/audit/builtin/comment_to_code_ratio.json
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
  - "complexity_pipelines() uses _language prefix to silence unused parameter warning (TypeScript keeps Language import for other factory functions)"
  - "comment_to_code_ratio uses select: file (not symbol) matching executor's special-casing for whole-file metric"
  - "No bare severity field in any of the 4 JSON pipelines — suppression entirely through severity_map when clauses"
  - "cyclomatic_complexity thresholds upgraded: warning > 10, error >= 20 (vs Rust single-threshold warning only)"
  - "cognitive_complexity thresholds: warning > 15, error >= 30 (upgrade from Rust's warning-only)"
  - "function_length statement-count check (> 20 stmts) dropped — not expressible in JSON DSL, documented in description"
metrics:
  duration: "~5 minutes"
  completed: "2026-04-16"
  tasks_completed: 4
  files_modified: 14
  files_created: 4
  files_deleted: 40
---

# Phase 03 Plan 02: 4 Cross-Language Complexity JSON Pipelines + 40 Rust Deletions Summary

Created 4 cross-language JSON complexity pipelines using severity_map-only suppression and deleted all 40 Rust complexity pipeline files across 10 language directories, with all language mod.rs factories returning empty vecs.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Create 4 cross-language complexity JSON pipelines | 1687293 | src/audit/builtin/cyclomatic_complexity.json, function_length.json, cognitive_complexity.json, comment_to_code_ratio.json |
| 2 | Delete TS/JS/Rust complexity Rust files, update 3 mod.rs | 2573ead | typescript/mod.rs, javascript/mod.rs, rust/mod.rs (12 files deleted) |
| 3 | Delete Python/Go/Java/C complexity Rust files, update 4 mod.rs | 08660d7 | python/mod.rs, go/mod.rs, java/mod.rs, c/mod.rs (16 files deleted) |
| 4 | Delete C++/C#/PHP complexity Rust files, update 3 mod.rs | d99a512 | cpp/mod.rs, csharp/mod.rs, php/mod.rs (12 files deleted) |

## What Was Built

### Task 1 — 4 Cross-Language JSON Pipelines (1687293)

Created 4 JSON pipeline files in `src/audit/builtin/` — no `languages` filter so they apply to all supported languages:

**`cyclomatic_complexity.json`**: select symbol where kind in [function, method, arrow_function], compute_metric cyclomatic_complexity, flag with severity_map: error >= 20, warning > 10. Pattern: `high_cyclomatic_complexity`.

**`function_length.json`**: select symbol where kind in [function, method, arrow_function], compute_metric function_length, flag with severity_map: error >= 100 lines, warning > 50 lines. Pattern: `function_too_long`. Statement count check dropped (not expressible in DSL).

**`cognitive_complexity.json`**: select symbol where kind in [function, method, arrow_function], compute_metric cognitive_complexity, flag with severity_map: error >= 30, warning > 15. Pattern: `high_cognitive_complexity`.

**`comment_to_code_ratio.json`**: select file (whole-file metric), compute_metric comment_to_code_ratio, flag with severity_map: warning < 5% (under-documented) or > 60% (over-documented). Pattern: `comment_ratio_violation`.

All 4 use `exclude: {is_test_file: true}` and severity_map-only (no bare `severity` field) — below-threshold nodes suppressed via 03-01's `resolve_severity` returning None.

### Tasks 2-4 — 40 Rust Files Deleted (2573ead, 08660d7, d99a512)

Deleted all Rust complexity pipeline files across 10 language directories in 3 batches:
- Batch 1 (2573ead): typescript, javascript, rust — 12 files
- Batch 2 (08660d7): python, go, java, c — 16 files
- Batch 3 (d99a512): cpp, csharp, php — 12 files

Each language `mod.rs` was updated:
- Removed 4 `pub mod` declarations (`cognitive`, `comment_ratio`, `cyclomatic`, `function_length`)
- `complexity_pipelines()` body replaced with `Ok(vec![])`
- TypeScript uses `_language: Language` prefix to suppress unused-variable warning (Language import retained for other factory functions)
- JavaScript, Rust, Python, Go, Java, C, C++, C#, PHP all use no-arg signature or existing pattern unchanged

The engine's `json_pipeline_names` HashSet suppression mechanism ensures no doubled findings — JSON pipeline names exactly match the deleted Rust pipelines' `fn name()` return values.

## Deviations from Plan

None — plan executed exactly as written.

## Verification Results

```
find src/audit/pipelines -name "cyclomatic.rs" -o -name "cognitive.rs" -o -name "function_length.rs" -o -name "comment_ratio.rs"
# Returns empty — all 40 deleted

ls src/audit/builtin/cyclomatic_complexity.json src/audit/builtin/function_length.json \
   src/audit/builtin/cognitive_complexity.json src/audit/builtin/comment_to_code_ratio.json
# All 4 exist

cargo test: 8 passed, 0 failed (all 4 tasks)
```

## Self-Check: PASSED

- [x] src/audit/builtin/cyclomatic_complexity.json created — confirmed (1687293)
- [x] src/audit/builtin/function_length.json created — confirmed (1687293)
- [x] src/audit/builtin/cognitive_complexity.json created — confirmed (1687293)
- [x] src/audit/builtin/comment_to_code_ratio.json created — confirmed (1687293)
- [x] 40 Rust complexity files deleted across 10 language dirs — confirmed (2573ead, 08660d7, d99a512)
- [x] All 10 mod.rs complexity_pipelines() return Ok(vec![]) — confirmed
- [x] Task 1 commit 1687293 exists
- [x] Task 2 commit 2573ead exists
- [x] Task 3 commit 08660d7 exists
- [x] Task 4 commit d99a512 exists
- [x] cargo test passes with zero failures

## Known Stubs

None — all JSON pipelines are fully wired to executor stages (compute_metric, flag with severity_map). No placeholder values.

## Threat Flags

None — changes are internal to the audit engine pipeline evaluation logic. New JSON files extend existing pipeline dispatch; no new network endpoints, auth paths, file access patterns, or schema changes at trust boundaries.
