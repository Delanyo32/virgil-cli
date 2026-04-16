---
phase: 04-security-per-language-scalability-migration
plan: "03"
subsystem: audit-pipelines
tags: [rust, go, security, scalability, json-migration, pipeline]
dependency_graph:
  requires: [04-01]
  provides:
    - command_injection_go.json
    - go_path_traversal_go.json
    - race_conditions_go.json
    - resource_exhaustion_go.json
    - go_integer_overflow_go.json
    - go_type_confusion_go.json
    - memory_leak_indicators_go.json
  affects:
    - src/audit/engine.rs (ENG-01 name-match suppression triggers for 7 pipelines)
    - src/audit/pipelines/go/mod.rs (security_pipelines returns only sql_injection + ssrf)
tech_stack:
  added: []
  patterns:
    - JSON match_pattern pipeline with languages scoping to go only
    - ENG-01 name-match suppression removing Go Rust pipelines when JSON exists
    - Go grammar statement_list wrapping: for_statement body uses (block (statement_list ...))
key_files:
  created:
    - src/audit/builtin/command_injection_go.json
    - src/audit/builtin/go_path_traversal_go.json
    - src/audit/builtin/race_conditions_go.json
    - src/audit/builtin/resource_exhaustion_go.json
    - src/audit/builtin/go_integer_overflow_go.json
    - src/audit/builtin/go_type_confusion_go.json
    - src/audit/builtin/memory_leak_indicators_go.json
  modified:
    - src/audit/pipelines/go/mod.rs
    - tests/audit_json_integration.rs
  deleted:
    - src/audit/pipelines/go/command_injection.rs
    - src/audit/pipelines/go/go_path_traversal.rs
    - src/audit/pipelines/go/go_race_conditions.rs
    - src/audit/pipelines/go/go_resource_exhaustion.rs
    - src/audit/pipelines/go/go_integer_overflow.rs
    - src/audit/pipelines/go/go_type_confusion.rs
    - src/audit/pipelines/go/memory_leak_indicators.rs
decisions:
  - "race_conditions_go.json and resource_exhaustion_go.json scoped to languages: [go] to avoid conflict with identically-named Rust JSON pipelines"
  - "memory_leak_indicators_go.json scoped to languages: [go] to avoid conflict with Rust memory_leak_indicators pipeline"
  - "Go grammar wraps block statements in statement_list node -- patterns must use (block (statement_list (go_statement))) not (block (go_statement))"
  - "command_injection_go.json and go_path_traversal_go.json use broad selector_expression match (precision reduced per D-07 -- #match? not supported)"
  - "go_integer_overflow_go.json matches all call_expression+identifier (type conversion syntax) -- also matches non-conversion calls like append()/len()"
  - "go_type_confusion_go.json matches all type_assertion_expression (cannot distinguish guarded from unguarded without #match?)"
  - "sql_injection.rs and ssrf_open_redirect.rs preserved as permanent Rust exceptions (require FlowsTo/SanitizedBy graph predicates)"
metrics:
  duration: "18 minutes"
  completed: "2026-04-16T20:46:33Z"
  tasks_completed: 2
  tasks_total: 2
  files_created: 7
  files_modified: 2
  files_deleted: 7
---

# Phase 04 Plan 03: Go Security + Scalability Pipeline JSON Migration Summary

Migrated 6 Go security pipelines and the memory_leak_indicators scalability pipeline from Rust to declarative JSON definitions. Seven legacy Rust .rs files deleted; seven JSON files created in src/audit/builtin/; 14 integration tests added (7 positive + 7 negative). sql_injection.rs and ssrf_open_redirect.rs preserved as permanent Rust exceptions. cargo test passes with zero failures.

## What Was Built

**7 JSON pipeline files** in `src/audit/builtin/` replacing the Rust implementations:

| Pipeline | Category | Pattern | S-expression approach |
|----------|----------|---------|----------------------|
| command_injection | security | exec_command_injection | `call_expression selector_expression` (all selector calls) |
| go_path_traversal | security | unvalidated_path_join | `call_expression selector_expression` (all selector calls) |
| race_conditions | security | loop_var_capture | `for_statement (block (statement_list (go_statement)))` (precise) |
| resource_exhaustion | security | unbounded_goroutine_spawn | `for_statement (block (statement_list (go_statement)))` (precise) |
| go_integer_overflow | security | narrowing_conversion | `call_expression (identifier)` (all type conversion-style calls) |
| go_type_confusion | security | unsafe_pointer_cast | `type_assertion_expression` (all type assertions) |
| memory_leak_indicators | scalability | potential_memory_leak | `for_statement (block (statement_list (go_statement)))` (precise) |

**Key design decisions:**
- All 7 files use `"languages": ["go"]` for proper ENG-01 name-match suppression and language isolation
- `race_conditions`, `resource_exhaustion`, and `memory_leak_indicators` also need language scoping to avoid name collision with identically-named Rust JSON pipelines
- `race_conditions` and `resource_exhaustion` achieve semi-precise detection (for loop + goroutine pattern)
- `command_injection` and `go_path_traversal` use broad selector call patterns (precision reduced per D-07)
- `go_integer_overflow` and `go_type_confusion` use broader patterns (precision reduced per D-07)

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | 12d6c85 | Create 7 JSON pipeline files for Go security + scalability |
| Task 2 | 80e187f | Delete Go Rust pipeline files, update mod.rs, add 14 integration tests |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Go grammar uses statement_list wrapper inside for_statement body**
- **Found during:** Task 2 integration test failures
- **Issue:** Initial JSON patterns used `(for_statement body: (block (go_statement)))` but Go's tree-sitter grammar wraps block contents in a `statement_list` node: `(block (statement_list (go_statement ...)))`. Pattern 1 as written compiled but matched zero nodes.
- **Fix:** Updated race_conditions_go.json, resource_exhaustion_go.json, and memory_leak_indicators_go.json to use `(for_statement body: (block (statement_list (go_statement) @go_stmt))) @for_stmt`.
- **Files modified:** race_conditions_go.json, resource_exhaustion_go.json, memory_leak_indicators_go.json
- **Commit:** 80e187f (included in Task 2 commit)

### Precision Reductions (per D-07)

**1. [D-07] command_injection, go_path_traversal: all selector calls flagged**
- **Issue:** Cannot filter by package/method name (`exec.Command`, `filepath.Join`) without `#match?` predicate support.
- **Action:** Flag all `call_expression -> selector_expression` patterns. Document in JSON description.
- **Impact:** All package.Method() calls flagged regardless of package or method name.

**2. [D-07] go_integer_overflow: all call_expression+identifier flagged**
- **Issue:** Cannot filter to narrowing type names (int8/int16/int32/uint8/uint16/uint32) without `#match?` predicate support. Type conversion syntax `int32(x)` is identical to function call syntax `append(x)` in Go's grammar.
- **Action:** Flag all `call_expression -> identifier` patterns. Document in JSON description.
- **Impact:** All function/type calls with simple identifiers flagged.

**3. [D-07] go_type_confusion: all type assertions flagged**
- **Issue:** Cannot distinguish guarded (`val, ok := x.(Type)`) from unguarded (`val := x.(Type)`) type assertions without `#match?` predicate support.
- **Action:** Flag all `type_assertion_expression` nodes. Document in JSON description.
- **Impact:** Both guarded and unguarded type assertions flagged.

### Integration Test Design

- Positive tests trigger findings using the broad patterns documented above
- Negative tests for command_injection and go_path_traversal use struct/const-only Go files (no function calls)
- Negative tests for go_integer_overflow use type and variable declarations with no call expressions
- Negative tests for race_conditions, resource_exhaustion, memory_leak_indicators use goroutine-outside-loop patterns

## Known Stubs

None — all pipelines produce real findings on positive test fixtures and no findings on negative fixtures.

## Threat Flags

None — JSON files are embedded at compile time via include_dir. No new runtime attack surface.

## Self-Check: PASSED

All 7 JSON files exist in src/audit/builtin/.
All 7 Rust pipeline files deleted from src/audit/pipelines/go/.
sql_injection.rs and ssrf_open_redirect.rs still exist as permanent exceptions.
Commits 12d6c85 and 80e187f verified in git log.
cargo test: 2214 lib + 66 integration + 8 integration_test = 2288 tests, 0 failures.
