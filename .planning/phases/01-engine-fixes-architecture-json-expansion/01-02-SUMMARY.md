---
phase: 01-engine-fixes-architecture-json-expansion
plan: 02
subsystem: audit-json-pipelines
tags: [json-audit, architecture, pipeline, typescript, javascript, python, rust, go, java]
dependency_graph:
  requires: [ENG-02]
  provides: [ARCH-01, ARCH-02, ARCH-03, ARCH-04, ARCH-05]
  affects:
    - src/audit/builtin/module_size_distribution_javascript.json
    - src/audit/builtin/module_size_distribution_python.json
    - src/audit/builtin/module_size_distribution_rust.json
    - src/audit/builtin/module_size_distribution_go.json
    - src/audit/builtin/module_size_distribution_java.json
    - src/audit/builtin/circular_dependencies_javascript.json
    - src/audit/builtin/circular_dependencies_python.json
    - src/audit/builtin/circular_dependencies_rust.json
    - src/audit/builtin/circular_dependencies_go.json
    - src/audit/builtin/circular_dependencies_java.json
    - src/audit/builtin/dependency_graph_depth_javascript.json
    - src/audit/builtin/dependency_graph_depth_python.json
    - src/audit/builtin/dependency_graph_depth_rust.json
    - src/audit/builtin/dependency_graph_depth_go.json
    - src/audit/builtin/dependency_graph_depth_java.json
    - src/audit/builtin/api_surface_area_javascript.json
    - src/audit/builtin/api_surface_area_python.json
    - src/audit/builtin/api_surface_area_rust.json
    - src/audit/builtin/api_surface_area_go.json
    - src/audit/builtin/api_surface_area_java.json
tech_stack:
  added: []
  patterns: [per-language JSON pipeline files with languages filter field, calibrated depth thresholds per language group]
key_files:
  created:
    - src/audit/builtin/module_size_distribution_javascript.json
    - src/audit/builtin/module_size_distribution_python.json
    - src/audit/builtin/module_size_distribution_rust.json
    - src/audit/builtin/module_size_distribution_go.json
    - src/audit/builtin/module_size_distribution_java.json
    - src/audit/builtin/circular_dependencies_javascript.json
    - src/audit/builtin/circular_dependencies_python.json
    - src/audit/builtin/circular_dependencies_rust.json
    - src/audit/builtin/circular_dependencies_go.json
    - src/audit/builtin/circular_dependencies_java.json
    - src/audit/builtin/dependency_graph_depth_javascript.json
    - src/audit/builtin/dependency_graph_depth_python.json
    - src/audit/builtin/dependency_graph_depth_rust.json
    - src/audit/builtin/dependency_graph_depth_go.json
    - src/audit/builtin/dependency_graph_depth_java.json
    - src/audit/builtin/api_surface_area_javascript.json
    - src/audit/builtin/api_surface_area_python.json
    - src/audit/builtin/api_surface_area_rust.json
    - src/audit/builtin/api_surface_area_go.json
    - src/audit/builtin/api_surface_area_java.json
  modified: []
decisions:
  - "JavaScript language group uses languages: [typescript, javascript, tsx, jsx] to cover all 4 TS/JS dialects"
  - "dependency_graph_depth_rust.json uses threshold gte 4 (stricter — Rust crate structure is flatter)"
  - "dependency_graph_depth_go.json uses threshold gte 5 (stricter — Go packages tend shallower)"
  - "All other depth thresholds remain gte 6 (matching template baseline)"
  - "module_size_distribution and circular_dependencies thresholds identical across all 5 language groups"
  - "api_surface_area thresholds identical across all 5 language groups (count gte 10 and ratio gte 0.8)"
metrics:
  duration: "3 minutes"
  completed_date: "2026-04-16"
  tasks_completed: 1
  files_modified: 21
---

# Phase 1 Plan 02: Per-Language Architecture JSON Pipelines (Wave 2) Summary

**One-liner:** 20 per-language JSON architecture pipeline files for TS/JS, Python, Rust, Go, and Java — 4 pipeline types each with language-calibrated thresholds embedded at compile time via `include_dir!`.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Create 20 per-language JSON architecture pipeline files for TS/JS, Python, Rust, Go, Java | 1e47dfd | 20 new JSON files + Cargo.lock |

## What Was Built

### 20 Per-Language JSON Architecture Pipeline Files

Each of the 4 architecture pipeline types (module_size_distribution, circular_dependencies, dependency_graph_depth, api_surface_area) was instantiated for each of 5 language groups, producing 20 new JSON files in `src/audit/builtin/`. Each file:

1. Has a unique `"pipeline"` field value in the form `{pipeline}_{lang}` (e.g., `"module_size_distribution_rust"`)
2. Has a `"languages"` field filtering to the correct language group
3. Has `"category": "architecture"`
4. Uses language-calibrated thresholds where appropriate

**Language group to languages field mapping:**
- `_javascript` files: `["typescript", "javascript", "tsx", "jsx"]`
- `_python` files: `["python"]`
- `_rust` files: `["rust"]`
- `_go` files: `["go"]`
- `_java` files: `["java"]`

**Language-calibrated thresholds (dependency_graph_depth only):**
- Rust: `{"gte": 4}` — stricter, crate module structure is naturally flat
- Go: `{"gte": 5}` — stricter, Go package hierarchies tend shallower
- TypeScript/JS, Python, Java: `{"gte": 6}` — same as baseline template

All other pipelines use identical thresholds across all language groups.

**Discovery mechanism:** The `include_dir!` macro (added in Plan 01) automatically embeds all `.json` files in `src/audit/builtin/` at compile time. No source code changes required to register these new files — they are discovered automatically by `builtin_audits()`.

## Verification Results

- `ls src/audit/builtin/*_{javascript,python,rust,go,java}.json | wc -l` returns 20
- `cargo test --lib -- json_audit`: 9/9 tests pass
- `cargo test --lib`: 2559/2559 tests pass (zero regressions)

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None. All 20 JSON files are complete pipeline definitions with proper language filters and thresholds.

## Threat Flags

None. JSON files are embedded at compile time via `include_dir!`. No new trust boundaries, network endpoints, or runtime loading introduced.

## Self-Check: PASSED

- [x] Exactly 20 new JSON files exist in src/audit/builtin/ (4 pipelines x 5 language groups): confirmed (count = 20)
- [x] Each file has unique "pipeline" field ending in _javascript, _python, _rust, _go, or _java: confirmed
- [x] Each file has "languages" field with correct language strings: confirmed
- [x] Each file has "category": "architecture": confirmed
- [x] dependency_graph_depth_rust.json has threshold {"gte": 4}: confirmed
- [x] dependency_graph_depth_go.json has threshold {"gte": 5}: confirmed
- [x] dependency_graph_depth_javascript.json has threshold {"gte": 6}: confirmed
- [x] module_size_distribution_javascript.json has "languages": ["typescript", "javascript", "tsx", "jsx"]: confirmed
- [x] cargo test --lib exits 0: confirmed (2559 passed)
- [x] Commit 1e47dfd exists: confirmed
