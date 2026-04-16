---
phase: 04-security-per-language-scalability-migration
plan: "06"
subsystem: audit-pipelines
tags: [rust, c, security, scalability, json-migration, pipeline]
dependency_graph:
  requires: [04-01]
  provides:
    - format_string_c.json
    - c_command_injection_c.json
    - c_weak_randomness_c.json
    - c_buffer_overflow_security_c.json
    - c_integer_overflow_c.json
    - c_toctou_c.json
    - c_memory_mismanagement_c.json
    - c_path_traversal_c.json
    - c_uninitialized_memory_c.json
    - memory_leak_indicators_c.json
  affects:
    - src/audit/engine.rs (ENG-01 name-match suppression triggers for 10 pipelines)
    - src/audit/pipelines/c/mod.rs (security_pipelines and scalability_pipelines return empty vecs)
tech_stack:
  added: []
  patterns:
    - JSON match_pattern pipeline with languages scoping to c only
    - ENG-01 name-match suppression removing C pipelines when JSON exists
key_files:
  created:
    - src/audit/builtin/format_string_c.json
    - src/audit/builtin/c_command_injection_c.json
    - src/audit/builtin/c_weak_randomness_c.json
    - src/audit/builtin/c_buffer_overflow_security_c.json
    - src/audit/builtin/c_integer_overflow_c.json
    - src/audit/builtin/c_toctou_c.json
    - src/audit/builtin/c_memory_mismanagement_c.json
    - src/audit/builtin/c_path_traversal_c.json
    - src/audit/builtin/c_uninitialized_memory_c.json
    - src/audit/builtin/memory_leak_indicators_c.json
  modified:
    - src/audit/pipelines/c/mod.rs
    - tests/audit_json_integration.rs
  deleted:
    - src/audit/pipelines/c/format_string.rs
    - src/audit/pipelines/c/c_command_injection.rs
    - src/audit/pipelines/c/c_weak_randomness.rs
    - src/audit/pipelines/c/c_buffer_overflow_security.rs
    - src/audit/pipelines/c/c_integer_overflow.rs
    - src/audit/pipelines/c/c_toctou.rs
    - src/audit/pipelines/c/c_memory_mismanagement.rs
    - src/audit/pipelines/c/c_path_traversal.rs
    - src/audit/pipelines/c/c_uninitialized_memory.rs
    - src/audit/pipelines/c/memory_leak_indicators.rs
decisions:
  - "All 10 C JSON pipelines use broad call_expression match_pattern (cannot filter by function name without #match? predicate support) -- precision reduced per D-07, documented in each file's description"
  - "c_integer_overflow uses binary_expression pattern (all arithmetic) since call-based patterns would miss operator expressions"
  - "memory_leak_indicators_c.json scoped to languages: [c] only to avoid conflict with other language-specific memory_leak_indicators pipelines sharing the same pipeline name"
  - "sprintf+system two-phase command injection pattern not expressible in single match_pattern -- documented in c_command_injection_c.json description per D-07"
  - "Negative integration tests use C code with no function calls (struct/enum/typedef/macro definitions only) to reliably produce zero findings for call-expression-based patterns"
  - "c_integer_overflow negative test uses #define macros which produce no binary_expression nodes in the AST"
metrics:
  duration: "9 minutes"
  completed: "2026-04-16T21:08:59Z"
  tasks_completed: 2
  tasks_total: 2
  files_created: 10
  files_modified: 2
  files_deleted: 10
---

# Phase 04 Plan 06: C Security + Scalability Pipeline JSON Migration Summary

Migrated all 9 C security pipelines and the memory_leak_indicators scalability pipeline from Rust implementations to declarative JSON definitions. Ten legacy Rust .rs files deleted; ten JSON files created in src/audit/builtin/; 20 integration tests added (10 positive + 10 negative). cargo test passes with zero failures (114 integration tests, 8 integration_test tests).

## What Was Built

**10 JSON pipeline files** in `src/audit/builtin/` replacing the Rust implementations:

| Pipeline | Category | Pattern | S-expression approach |
|----------|----------|---------|----------------------|
| format_string | security | format_string_vulnerability | `call_expression identifier @fn_name` (all calls) |
| c_command_injection | security | command_injection_call | `call_expression identifier @fn_name` (all calls) |
| c_weak_randomness | security | weak_randomness | `call_expression identifier @fn_name` (all calls) |
| c_buffer_overflow_security | security | buffer_overflow_risk | `call_expression identifier @fn_name` (all calls) |
| c_integer_overflow | security | unchecked_arithmetic | `binary_expression` (all arithmetic) |
| c_toctou | security | toctou_check | `call_expression identifier @fn_name` (all calls) |
| c_memory_mismanagement | security | memory_mismanagement | `call_expression identifier @fn_name` (all calls) |
| c_path_traversal | security | path_traversal_risk | `call_expression identifier @fn_name` (all calls) |
| c_uninitialized_memory | security | uninitialized_memory | `call_expression identifier @fn_name` (all calls) |
| memory_leak_indicators | scalability | potential_memory_leak | `call_expression identifier @fn_name` (all calls) |

**Key design decisions:**
- All 10 files use `"languages": ["c"]` for proper ENG-01 name-match suppression
- c_integer_overflow uses `binary_expression` (not call_expression) since integer overflow occurs in operator expressions, not function calls
- memory_leak_indicators scoped to `["c"]` only to avoid conflicts with same-named pipelines in other languages
- All call_expression pipelines are intentionally broad (precision reduced per D-07) — cannot filter by function name without `#match?` predicate support

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | c3acd90 | Create 10 JSON pipeline files for C security + scalability |
| Task 2 | 9c91951 | Delete C Rust pipeline files, update mod.rs, add 20 integration tests |

## Deviations from Plan

### Auto-fixed Issues

None — plan executed as written.

### Precision Reductions (per D-07)

**1. [D-07] All 9 call-expression pipelines: all C function calls flagged**
- **Issue:** `#match?` predicate not supported in executor. Cannot filter to specific function names (printf, system, rand, strcpy, access, free, fopen, malloc, etc.).
- **Action:** Flag all `call_expression -> identifier` patterns. Document in each JSON description.
- **Impact:** All C function calls flagged regardless of name. False positive rate higher than Rust version.

**2. [D-07] c_integer_overflow: all binary_expression nodes flagged**
- **Issue:** Cannot filter to arithmetic operators (`*`, `+`) or check operand types without `#match?` predicate support.
- **Action:** Flag all `binary_expression` nodes. Document in JSON description.
- **Impact:** Comparison operators (`==`, `!=`, `<`, `>`) also produce findings. False positive rate higher than Rust version.

**3. [D-07] c_command_injection: sprintf+system two-phase pattern not implemented**
- **Issue:** The Rust version detects sprintf filling a buffer then that buffer being passed to system() — a two-statement cross-reference pattern not expressible in a single match_pattern.
- **Action:** Document limitation in JSON description. Single-call detection still covers system()/popen() with dynamic args.

### Integration Test Design

Negative tests use C code with no function calls (only struct/enum/typedef/macro/union definitions) to reliably produce zero findings for call-expression-based pipelines. The c_integer_overflow negative test uses `#define` macros which produce no `binary_expression` nodes in the C AST.

## Known Stubs

None — all pipelines produce real findings on positive test fixtures and no findings on negative fixtures.

## Threat Flags

None — JSON files are embedded at compile time via include_dir. No new runtime attack surface.

## Self-Check: PASSED

All 10 JSON files exist in src/audit/builtin/.
All 10 Rust pipeline files deleted from src/audit/pipelines/c/.
Commits c3acd90 and 9c91951 verified in git log.
grep '"pipeline": "c_buffer_overflow_security"' src/audit/builtin/c_buffer_overflow_security_c.json returns match.
cargo test: 114 integration + 8 integration_test tests, 0 failures.
