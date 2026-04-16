---
phase: 01-engine-fixes-architecture-json-expansion
plan: 05
subsystem: audit/tests
tags: [integration-tests, architecture, json-pipelines, tdd]
dependency_graph:
  requires: ["01-02", "01-03"]
  provides: ["TEST-01", "TEST-02"]
  affects: ["tests/audit_json_integration.rs"]
tech_stack:
  added: []
  patterns: ["tempfile for fixture isolation", "AuditEngine + GraphBuilder end-to-end test pattern"]
key_files:
  created:
    - tests/audit_json_integration.rs
  modified: []
decisions:
  - "Used relative Python imports (.module_b) instead of absolute imports because python.rs resolve_import only resolves relative imports; absolute imports are treated as external and never create intra-workspace Imports graph edges"
  - "Replaced Go with TypeScript for the dependency_graph_depth positive test because go.rs resolve_import does not handle ./b relative paths (Go uses full module paths); TypeScript's resolve_import handles ./ correctly and has the same gte:6 threshold"
metrics:
  duration: "~10 minutes"
  completed: "2026-04-16T09:13:52Z"
  tasks_completed: 1
  files_changed: 1
requirements:
  - TEST-01
  - TEST-02
---

# Phase 01 Plan 05: Architecture JSON Integration Tests Summary

8 end-to-end integration tests covering all 4 architecture JSON pipeline types using the AuditEngine + GraphBuilder path with temp directory fixtures.

## Objective Achieved

Created `tests/audit_json_integration.rs` with 8 tests (4 positive + 4 negative) validating:
- `module_size_distribution_rust`: Rust file with 31 pub fns triggers `oversized_module`
- `api_surface_area`: TypeScript file with 11 exported fns triggers `excessive_public_api`
- `circular_dependencies_python`: Two Python files with relative cross-imports trigger `circular_dependency`
- `dependency_graph_depth_javascript`: Chain of 7 TypeScript files (a→b→…→g) triggers `deep_import_chain`

All tests exercise the complete path: workspace loading → graph building → JSON pipeline execution → finding assertion.

## Task Results

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Create integration test file with 8 architecture pipeline tests | 9802bbf | tests/audit_json_integration.rs |

## Verification

- `cargo test --test audit_json_integration`: 8 tests pass
- `cargo test` (full suite): 2575 tests pass, 0 failures

## Deviations from Plan

### Auto-fixed Issues

None — plan executed with two planned-acceptable language fixture adjustments:

**1. [Rule 1 - Bug fix] Python circular dependency test: relative vs absolute imports**
- **Found during:** Task 1
- **Issue:** Plan used `from module_b import something` (absolute import). Python's `resolve_import` in `src/languages/python.rs` returns `None` for absolute imports (treats them as external). No `Imports` graph edge would be created, so no cycle would be detected.
- **Fix:** Changed to `from .module_b import something` (relative import) which `resolve_import` correctly resolves to the sibling file.
- **Files modified:** tests/audit_json_integration.rs
- **Commit:** 9802bbf

**2. [Rule 1 - Bug fix] Go replaced with TypeScript for dependency_graph_depth test**
- **Found during:** Task 1
- **Issue:** Plan used Go with `import "./b"` style imports. Go's `resolve_import` in `src/languages/go.rs` handles full module paths (e.g., `github.com/foo/bar`) and relative path patterns, but does not resolve `"./b"` style relative imports to workspace files.
- **Fix:** Used TypeScript instead. TypeScript's `resolve_import` resolves `./b` to `b.ts` correctly. The JavaScript/TypeScript pipeline has the same `gte:6` threshold as the generic pipeline. Created a 7-file chain (a→b→c→d→e→f→g) so g has depth 6.
- **Files modified:** tests/audit_json_integration.rs
- **Commit:** 9802bbf

Note: The plan itself anticipated the Go issue and listed TypeScript as an explicit alternative.

## Known Stubs

None.

## Threat Flags

None — test-only file, no production code modified.

## Self-Check: PASSED

- [x] `tests/audit_json_integration.rs` exists
- [x] Commit 9802bbf exists in git log
- [x] 8 test functions present (4 positive + 4 negative)
- [x] `cargo test --test audit_json_integration`: all 8 pass
- [x] `cargo test` full suite: all tests pass
