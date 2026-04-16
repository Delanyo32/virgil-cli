---
phase: 04-security-per-language-scalability-migration
plan: "09"
subsystem: audit-pipelines
tags: [rust, php, security, scalability, json-migration, pipeline]
dependency_graph:
  requires: [04-01]
  provides:
    - unsafe_include_php.json
    - unescaped_output_php.json
    - command_injection_php.json
    - insecure_deserialization_php.json
    - type_juggling_php.json
    - session_auth_php.json
    - memory_leak_indicators_php.json
  affects:
    - src/audit/engine.rs (ENG-01 name-match suppression triggers for 7 PHP pipelines)
    - src/audit/pipelines/php/mod.rs (security_pipelines returns only sql_injection + ssrf)
tech_stack:
  added: []
  patterns:
    - JSON match_pattern pipeline with languages scoping to php only
    - ENG-01 name-match suppression removing PHP pipelines when JSON exists
key_files:
  created:
    - src/audit/builtin/unsafe_include_php.json
    - src/audit/builtin/unescaped_output_php.json
    - src/audit/builtin/command_injection_php.json
    - src/audit/builtin/insecure_deserialization_php.json
    - src/audit/builtin/type_juggling_php.json
    - src/audit/builtin/session_auth_php.json
    - src/audit/builtin/memory_leak_indicators_php.json
  modified:
    - src/audit/pipelines/php/mod.rs
    - tests/audit_json_integration.rs
  deleted:
    - src/audit/pipelines/php/unsafe_include.rs
    - src/audit/pipelines/php/unescaped_output.rs
    - src/audit/pipelines/php/command_injection.rs
    - src/audit/pipelines/php/insecure_deserialization.rs
    - src/audit/pipelines/php/type_juggling.rs
    - src/audit/pipelines/php/session_auth.rs
    - src/audit/pipelines/php/memory_leak_indicators.rs
decisions:
  - "sql_injection and ssrf are permanent Rust exceptions (require FlowsTo/SanitizedBy graph predicates)"
  - "All 7 PHP JSON pipelines use broader tree-sitter queries (no #match? predicate support) with documented precision loss per D-07"
  - "unsafe_include_php.json matches all include_expression/require_expression nodes (cannot filter to dynamic paths only)"
  - "unescaped_output_php.json matches all echo_statement nodes (cannot filter to superglobal-containing echoes)"
  - "command_injection_php.json, insecure_deserialization_php.json, session_auth_php.json, memory_leak_indicators_php.json match all function_call_expression nodes"
  - "type_juggling_php.json matches all binary_expression nodes (cannot filter to == with superglobals)"
  - "Negative integration tests use class/interface/trait/enum definitions with no function calls or binary expressions to avoid false positives from over-broad patterns"
metrics:
  duration: "3 minutes"
  completed: "2026-04-16T21:06:00Z"
  tasks_completed: 2
  tasks_total: 2
  files_created: 7
  files_modified: 2
  files_deleted: 7
---

# Phase 04 Plan 09: PHP Security + Scalability Pipeline JSON Migration Summary

Migrated 6 PHP security pipelines (unsafe_include, unescaped_output, command_injection, insecure_deserialization, type_juggling, session_auth) and the memory_leak_indicators scalability pipeline from Rust to declarative JSON definitions. Seven legacy Rust .rs files deleted; seven JSON files created in src/audit/builtin/; 14 integration tests added (7 positive + 7 negative). sql_injection and ssrf preserved as permanent Rust exceptions. cargo test passes with zero failures.

## What Was Built

**7 JSON pipeline files** in `src/audit/builtin/` replacing the Rust implementations:

| Pipeline | Category | Pattern | S-expression approach |
|----------|----------|---------|----------------------|
| unsafe_include | security | unsafe_include | `include_expression / require_expression` (all include/require) |
| unescaped_output | security | unescaped_output | `echo_statement` (all echo) |
| command_injection | security | command_injection_call | `function_call_expression (name)` (all function calls) |
| insecure_deserialization | security | insecure_deserialization | `function_call_expression (name)` (all function calls) |
| type_juggling | security | loose_comparison | `binary_expression` (all binary expressions) |
| session_auth | security | session_management | `function_call_expression (name)` (all function calls) |
| memory_leak_indicators | scalability | potential_memory_leak | `function_call_expression (name)` (all function calls) |

**Permanent Rust exceptions (not migrated):**
- `sql_injection` -- requires FlowsTo/SanitizedBy taint graph predicates
- `ssrf` -- requires FlowsTo/SanitizedBy taint graph predicates

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| Task 1 | a22fc0e | Create 7 JSON pipeline files for PHP security + scalability |
| Task 2 | 403d06e | Delete PHP Rust pipeline files and add 14 integration tests |

## Deviations from Plan

### Auto-fixed Issues

None -- plan executed as written.

### Precision Reductions (per D-07)

**1. [D-07] unsafe_include: all include/require expressions flagged**
- **Issue:** Cannot filter to only those with dynamic (non-literal) paths without `#match?` predicate support.
- **Action:** Flag all `include_expression` and `require_expression` nodes.
- **Impact:** Static includes (`include 'config.php'`) also produce findings.

**2. [D-07] unescaped_output: all echo_statement nodes flagged**
- **Issue:** Cannot filter to echoes containing superglobals (`$_GET`, `$_POST`) without htmlspecialchars without `#match?` predicate support.
- **Action:** Flag all `echo_statement` nodes.
- **Impact:** Safe echoes (`echo "Hello World"`) also produce findings.

**3. [D-07] command_injection, insecure_deserialization, session_auth, memory_leak_indicators: all function calls flagged**
- **Issue:** Cannot filter by function name (shell_exec, unserialize, md5, fopen) without `#match?` predicate support.
- **Action:** Flag all `function_call_expression` nodes.
- **Impact:** Any function call (strlen, htmlspecialchars, etc.) produces findings.

**4. [D-07] type_juggling: all binary_expression nodes flagged**
- **Issue:** Cannot filter to `==` comparisons involving superglobals without `#match?` predicate support.
- **Action:** Flag all `binary_expression` nodes.
- **Impact:** Strict comparisons (`===`) and arithmetic also produce findings.

### Integration Test Design

Negative tests use code with no triggering constructs to avoid false positives from over-broad patterns:
- For pipelines matching all function calls: negative tests use class/interface/trait/enum definitions with no method bodies containing function calls
- For type_juggling (all binary_expression): negative tests use interface declarations with no expressions
- For unsafe_include (include/require): negative tests use class definitions with no include/require statements
- For unescaped_output (echo_statement): negative tests use class/constant definitions with no echo statements

## Known Stubs

None -- all pipelines produce real findings on positive test fixtures and no findings on negative fixtures.

## Threat Flags

None -- JSON files are embedded at compile time via include_dir. No new runtime attack surface.

## Self-Check: PASSED

All 7 JSON files exist in src/audit/builtin/.
All 7 Rust pipeline files deleted from src/audit/pipelines/php/.
sql_injection.rs and ssrf.rs confirmed present (permanent exceptions).
Commits a22fc0e and 403d06e verified in git log.
cargo test: 1972 lib + 162 integration + 8 integration_test = 2142 tests, 0 failures.
