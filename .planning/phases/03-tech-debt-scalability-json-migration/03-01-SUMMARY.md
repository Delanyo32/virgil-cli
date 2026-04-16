---
phase: 03-tech-debt-scalability-json-migration
plan: "01"
subsystem: graph/pipeline
tags: [rust, audit-engine, where-clause, severity-suppression, json-pipelines]
dependency_graph:
  requires: []
  provides:
    - WhereClause with kind + 4 metric predicate fields
    - FlagConfig::resolve_severity returning Option<String> with suppression
    - executor filter_map for finding generation
  affects:
    - src/graph/pipeline.rs
    - src/graph/executor.rs
tech_stack:
  added: []
  patterns:
    - "Option<String> return type for severity resolution enabling suppression"
    - "filter_map in executor to skip suppressed findings"
key_files:
  created: []
  modified:
    - src/graph/pipeline.rs
    - src/graph/executor.rs
decisions:
  - "kind field added as Option<Vec<String>> with case-insensitive match via eq_ignore_ascii_case"
  - "resolve_severity returns None (suppresses) only when severity_map exists, no entry matches, and no bare severity field — backward compat preserved for no-config flags"
  - "executor test test_flag_stage_produces_findings updated to use filter_map pattern to match production code"
metrics:
  duration: "~15 minutes"
  completed: "2026-04-16"
  tasks_completed: 2
  files_modified: 2
---

# Phase 03 Plan 01: WhereClause Extension + Severity Suppression Summary

Extended `WhereClause` with 5 new fields (kind + 4 compute-metric predicates) and changed `FlagConfig::resolve_severity` to return `Option<String>` with suppression support; executor now uses `filter_map` to skip below-threshold nodes.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Add kind + 4 metric predicate fields to WhereClause | f4c326c | src/graph/pipeline.rs |
| 2 | Add severity suppression to resolve_severity and update executor | bf3048d | src/graph/pipeline.rs, src/graph/executor.rs |

## What Was Built

### Task 1 — WhereClause Extension (f4c326c)

Added 5 new fields to `WhereClause` struct in `src/graph/pipeline.rs`:

- `kind: Option<Vec<String>>` — symbol kind filter evaluated in `eval()` via case-insensitive `eq_ignore_ascii_case` match
- `cyclomatic_complexity: Option<NumericPredicate>` — evaluated in both `eval()` and `eval_metrics()`
- `function_length: Option<NumericPredicate>` — evaluated in both `eval()` and `eval_metrics()`
- `cognitive_complexity: Option<NumericPredicate>` — evaluated in both `eval()` and `eval_metrics()`
- `comment_to_code_ratio: Option<NumericPredicate>` — evaluated in both `eval()` and `eval_metrics()`

Updated three methods:
- `is_empty()` — 5 additional `is_none()` checks
- `eval_metrics()` — 4 new metric predicate blocks after existing `ratio` check
- `eval()` — `kind` node property check after `exported` check, plus same 4 metric predicate blocks

Added 2 new unit tests: `where_clause_metric_predicates_cyclomatic` and `where_clause_kind_filter`.

### Task 2 — Severity Suppression (bf3048d)

Changed `FlagConfig::resolve_severity` return type from `String` to `Option<String>`:

- `Some(severity)` when severity_map entry matches or bare severity field serves as fallback
- `None` (suppress finding) when severity_map exists but no entry matches AND no bare `severity` field
- `Some("warning")` when no severity_map and no severity field (backward compat)

Updated `src/graph/executor.rs` flag finding generation from `.map()` to `.filter_map()` — nodes where `resolve_severity` returns `None` are silently skipped (no finding emitted).

Updated 6 existing `resolve_severity` tests to expect `Option<String>`. Added `test_resolve_severity_map_no_match_no_severity_suppresses` test for new suppression case.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] executor test used map() pattern inconsistent with updated resolve_severity**
- **Found during:** Task 2
- **Issue:** `test_flag_stage_produces_findings` in executor.rs manually called `resolve_severity` and assigned result to `severity: String` field — would not compile after `Option<String>` change
- **Fix:** Updated test to use `filter_map` pattern (same as production code)
- **Files modified:** src/graph/executor.rs
- **Commit:** bf3048d

## Verification Results

```
cargo test --lib graph::pipeline: 43 passed, 0 failed
cargo test (full suite): all passed, 0 failed
grep -c "cyclomatic_complexity" src/graph/pipeline.rs: 15 (>= 4 required)
grep "Option<String>" ... resolve_severity: confirmed
grep "filter_map" src/graph/executor.rs: confirmed in flag finding generation
```

## Self-Check: PASSED

- [x] src/graph/pipeline.rs modified — confirmed (f4c326c, bf3048d)
- [x] src/graph/executor.rs modified — confirmed (bf3048d)
- [x] Task 1 commit f4c326c exists
- [x] Task 2 commit bf3048d exists
- [x] cargo test passes with zero failures

## Known Stubs

None — all changes are functional implementations, not placeholders.

## Threat Flags

None — changes are internal to the audit engine pipeline evaluation logic. No new network endpoints, auth paths, file access patterns, or schema changes at trust boundaries.
