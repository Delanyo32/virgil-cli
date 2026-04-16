---
phase: 01-engine-fixes-architecture-json-expansion
plan: "04"
subsystem: audit-architecture
tags: [architecture, json-migration, dead-code-removal, cleanup]
dependency_graph:
  requires: ["01-02", "01-03"]
  provides: ["ARCH-10", "TEST-02"]
  affects: ["src/audit/engine.rs", "src/audit/pipeline.rs", "src/audit/json_audit.rs"]
tech_stack:
  added: []
  patterns: ["inline-empty-vec for JSON-only category", "Language::all() for architecture language selection"]
key_files:
  created: []
  modified:
    - src/audit/engine.rs
    - src/audit/pipeline.rs
    - src/audit/json_audit.rs
    - src/audit/pipelines/rust/mod.rs
    - src/audit/pipelines/go/mod.rs
    - src/audit/pipelines/python/mod.rs
    - src/audit/pipelines/php/mod.rs
    - src/audit/pipelines/java/mod.rs
    - src/audit/pipelines/javascript/mod.rs
    - src/audit/pipelines/typescript/mod.rs
    - src/audit/pipelines/c/mod.rs
    - src/audit/pipelines/cpp/mod.rs
    - src/audit/pipelines/csharp/mod.rs
    - src/main.rs
    - src/server.rs
  deleted:
    - src/audit/builtin/api_surface_area.json
    - src/audit/builtin/circular_dependencies.json
    - src/audit/builtin/dependency_depth.json
    - src/audit/builtin/module_size_distribution.json
decisions:
  - "Replace supported_architecture_languages() with Language::all().to_vec() at call sites — arch audit now runs across all languages, JSON pipelines filter by language themselves"
  - "Inline vec![] in engine.rs Architecture arm rather than calling a function — eliminates the dispatch layer entirely"
metrics:
  duration: "~15 minutes"
  completed: "2026-04-16"
  tasks_completed: 1
  tasks_total: 1
  files_modified: 15
  files_deleted: 4
---

# Phase 01 Plan 04: Architecture Migration Cleanup Summary

Complete the architecture migration by removing all legacy Rust architecture dispatch code. All 4 architecture pipeline categories are now 100% JSON-driven with 36 per-language built-in JSON files.

## What Was Done

Deleted the 4 old language-agnostic JSON files, removed 10 empty `architecture_pipelines()` stub functions across all language mod.rs files, deleted `architecture_pipelines_for_language()` and `supported_architecture_languages()` from `pipeline.rs`, updated the `Architecture` match arm in `engine.rs` to return an empty `vec![]` inline, updated callers in `main.rs` and `server.rs` to use `Language::all().to_vec()`, and updated `json_audit.rs` tests to assert 36 built-ins and check per-language pipeline names.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Delete old JSON files and remove Architecture Rust stubs (ARCH-10 + D-02) | 25418b8 | 19 files changed (15 modified, 4 deleted) |

## Verification Results

- `ls src/audit/builtin/*.json | wc -l` = 36
- `cargo build` exits 0 (1 pre-existing warning, out of scope)
- `cargo test --lib` = 2559 passed, 0 failed
- `grep "fn architecture_pipelines" src/audit/pipeline.rs` = empty
- `grep -rn "fn architecture_pipelines" src/audit/pipelines/` = empty
- `grep "architecture_pipelines_for_language" src/audit/engine.rs` = empty

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing functionality] Updated callers of supported_architecture_languages() in main.rs and server.rs**
- **Found during:** Task 1, Step 2
- **Issue:** The plan described deleting `supported_architecture_languages()` from `pipeline.rs`, but did not address the 4 call sites in `main.rs` (lines 630, 915) and `server.rs` (lines 260, 377) that would cause compile errors after deletion
- **Fix:** Replaced all call sites with `Language::all().to_vec()` — architecturally equivalent since JSON pipelines carry their own language filters; all languages are valid candidates
- **Files modified:** `src/main.rs`, `src/server.rs`
- **Commit:** 25418b8

**2. [Rule 1 - Bug] Fixed test_discover_json_audits_project_local_overrides_builtin test assertions**
- **Found during:** Task 1, Step 5
- **Issue:** The test at lines 199-205 checked for old non-language-specific pipeline names (`circular_dependencies`, `dependency_graph_depth`, etc.) that no longer exist as standalone built-ins after deleting the 4 old JSON files
- **Fix:** Updated assertions to check for `circular_dependencies_rust` and `module_size_distribution_go` (per-language built-ins that are still present)
- **Files modified:** `src/audit/json_audit.rs`
- **Commit:** 25418b8

## Known Stubs

None. All architecture pipelines are fully implemented as JSON definitions in `src/audit/builtin/`.

## Threat Flags

None. This plan only removes dead code and files — no new inputs, outputs, or trust boundaries introduced.

## Self-Check: PASSED

- Commit 25418b8 exists: confirmed
- `src/audit/builtin/*.json` count = 36: confirmed
- `src/audit/pipeline.rs` has no `architecture_pipelines_for_language` or `supported_architecture_languages`: confirmed
- All 10 language mod.rs stubs removed: confirmed
- `cargo test --lib`: 2559 passed, 0 failed
