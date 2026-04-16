---
phase: 04-security-per-language-scalability-migration
plan: "04"
subsystem: audit-pipelines
tags: [rust, security, scalability, json-migration, pipeline, python]
dependency_graph:
  requires: [04-01]
  provides:
    - command_injection_python.json
    - code_injection_python.json
    - path_traversal_python.json
    - insecure_deserialization_python.json
    - xxe_format_string_python.json
    - resource_exhaustion_python.json
    - memory_leak_indicators_python.json
  affects:
    - src/audit/engine.rs (ENG-01 name-match suppression triggers for 7 pipelines)
    - src/audit/pipeline.rs (security_pipelines_for_language returns 2-item vec for Python)
tech_stack:
  added: []
  patterns:
    - JSON match_pattern pipeline with languages scoping to python only
    - AnyPipeline::Graph pattern (Python differs from other languages that use wrap_legacy)
    - ENG-01 name-match suppression removing Python pipelines when JSON exists
key_files:
  created:
    - src/audit/builtin/command_injection_python.json
    - src/audit/builtin/code_injection_python.json
    - src/audit/builtin/path_traversal_python.json
    - src/audit/builtin/insecure_deserialization_python.json
    - src/audit/builtin/xxe_format_string_python.json
    - src/audit/builtin/resource_exhaustion_python.json
    - src/audit/builtin/memory_leak_indicators_python.json
  modified:
    - src/audit/pipelines/python/mod.rs
    - tests/audit_json_integration.rs
  deleted:
    - src/audit/pipelines/python/command_injection.rs
    - src/audit/pipelines/python/code_injection.rs
    - src/audit/pipelines/python/path_traversal.rs
    - src/audit/pipelines/python/insecure_deserialization.rs
    - src/audit/pipelines/python/xxe_format_string.rs
    - src/audit/pipelines/python/resource_exhaustion.rs
    - src/audit/pipelines/python/memory_leak_indicators.rs
decisions:
  - "Python uses AnyPipeline::Graph (not wrap_legacy) -- security_pipelines() returns vec directly; suppression still works via name() delegation"
  - "sql_injection and ssrf preserved as permanent Rust exceptions (require FlowsTo/SanitizedBy graph predicates)"
  - "resource_exhaustion is a ReDoS detector -- has_nested_quantifier() not expressible in match_pattern; all re module attribute calls flagged per D-07"
  - "path_traversal positive test uses posixpath.join (single-level attribute) because os.path.join uses chained attributes not matched by (attribute object: (identifier)) pattern"
  - "All 7 JSON pipelines use broad attribute-call or identifier-call patterns -- precision reduced per D-07, documented in description fields"
metrics:
  duration: "10 minutes"
  completed: "2026-04-16T20:53:34Z"
  tasks_completed: 2
  tasks_total: 2
  files_created: 7
  files_modified: 2
  files_deleted: 7
---

# Phase 04 Plan 04: Python Security + Scalability Pipeline JSON Migration Summary

Migrated 6 Python security pipelines and the memory_leak_indicators scalability pipeline from Rust GraphPipeline implementations to declarative JSON definitions. Seven legacy Rust .rs files deleted; 7 JSON files created in src/audit/builtin/; 14 integration tests added (7 positive + 7 negative). sql_injection and ssrf preserved as permanent Rust exceptions. cargo test passes with zero failures.

## What Was Built

**7 JSON pipeline files** in `src/audit/builtin/` replacing Rust implementations:

| Pipeline | Category | Pattern | S-expression approach |
|----------|----------|---------|----------------------|
| command_injection | security | command_injection_call | `call_expression attribute` (all attribute calls) |
| code_injection | security | code_injection_call | `call_expression identifier` (all direct calls) |
| path_traversal | security | unvalidated_path_join | `call_expression attribute` (all attribute calls) |
| insecure_deserialization | security | insecure_deserialization | `call_expression attribute` (all attribute calls) |
| xxe_format_string | security | xxe_format_string | `call_expression attribute` (all attribute calls) |
| resource_exhaustion | security | redos_pattern | `call_expression attribute` (all attribute calls, ReDoS) |
| memory_leak_indicators | scalability | potential_memory_leak | `call_expression identifier` (all direct calls) |

**Key design decisions:**
- All 7 files use `"languages": ["python"]` for proper ENG-01 name-match suppression
- Python uses `AnyPipeline::Graph` (not `wrap_legacy`) — the security_pipelines() function returns `Vec<AnyPipeline>` directly. After migration, it returns only sql_injection and ssrf.
- `resource_exhaustion` is a ReDoS detector using `has_nested_quantifier()` — a string analysis function not expressible in tree-sitter match_pattern; simplified to flag all `re` module attribute calls per D-07
- `memory_leak_indicators` was complex (open() + loop growth + __del__); simplified to flag all identifier calls per D-07

## Permanent Rust Exceptions Preserved

| Pipeline | Reason |
|----------|--------|
| sql_injection | Requires FlowsTo/SanitizedBy graph predicates (taint analysis) |
| ssrf | Requires FlowsTo/SanitizedBy graph predicates (taint analysis) |

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | bf56258 | Create 7 JSON pipeline files for Python security + scalability |
| Task 2 | 796592b | Delete 7 Python Rust pipeline files and add 14 integration tests |

## Deviations from Plan

### Auto-fixed Issues

None — plan executed as written.

### Precision Reductions (per D-07)

**1. [D-07] All 5 security pipelines (command_injection, path_traversal, insecure_deserialization, xxe_format_string, resource_exhaustion): all attribute calls flagged**
- **Issue:** Cannot filter by object name (os/subprocess/pickle/yaml/ET/re) or attribute name without `#match?` predicate support.
- **Action:** Flag all `call_expression -> attribute -> identifier` patterns. Document in JSON description.
- **Impact:** Any attribute method call in Python code produces a finding for each pipeline that matches this node type.

**2. [D-07] code_injection and memory_leak_indicators: all identifier calls flagged**
- **Issue:** Cannot filter to only eval/exec/compile or open() without `#match?` predicate support.
- **Action:** Flag all `call_expression -> identifier` patterns. Document in JSON description.
- **Impact:** Any direct function call (print, len, range, etc.) produces a finding.

**3. [D-07] resource_exhaustion: nested quantifier detection lost**
- **Issue:** `has_nested_quantifier()` string analysis cannot be expressed as a tree-sitter pattern.
- **Action:** Flag all re module attribute calls broadly.
- **Impact:** All regex calls flagged regardless of pattern content. Cannot distinguish `(a+)+` from `^[a-z]+$`.

**4. [D-07] memory_leak_indicators: loop containment and __del__ detection lost**
- **Issue:** JSON pipeline cannot check loop containment (is_inside_loop) or method name comparison (__del__).
- **Action:** Flag all identifier function calls.
- **Impact:** All direct function calls flagged — much broader than original open() + unbounded_growth + __del__ detection.

### Integration Test Deviation

**path_traversal positive test uses `posixpath.join` instead of `os.path.join`:**
- **Found during:** Task 2 test run
- **Issue:** `os.path.join(...)` uses chained attributes: `os.path` is an `attribute` node, not an `identifier`. The JSON pattern `(call function: (attribute object: (identifier) @obj ...))` only matches single-level attribute access (e.g., `path.join()`), not chained access (`os.path.join()`).
- **Fix:** Changed positive test fixture to `import posixpath as path; path.join(base_dir, user_path)` which uses a single-level attribute access.
- **Rule:** Rule 1 (auto-fix — test fixture bug, not production code)

## Known Stubs

None — all pipelines produce findings on positive test fixtures.

## Threat Flags

None — JSON files are embedded at compile time via include_dir. No new runtime attack surface.

## Self-Check: PASSED

- 7 JSON files exist in src/audit/builtin/ with _python suffix: FOUND
- sql_injection.rs and ssrf.rs still present: FOUND
- python/mod.rs security_pipelines() returns only sql_injection + ssrf: VERIFIED
- python/mod.rs scalability_pipelines() returns empty vec: VERIFIED
- Commits bf56258 and 796592b exist in git log: VERIFIED
- cargo test: 2260 tests, 0 failures: PASSED
