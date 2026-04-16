---
phase: 03-tech-debt-scalability-json-migration
reviewed: 2026-04-16T00:00:00Z
depth: standard
files_reviewed: 27
files_reviewed_list:
  - src/audit/builtin/cognitive_complexity.json
  - src/audit/builtin/comment_to_code_ratio.json
  - src/audit/builtin/cyclomatic_complexity.json
  - src/audit/builtin/function_length.json
  - src/audit/builtin/n_plus_one_queries.json
  - src/audit/builtin/sync_blocking_in_async_c.json
  - src/audit/builtin/sync_blocking_in_async_csharp.json
  - src/audit/builtin/sync_blocking_in_async_go.json
  - src/audit/builtin/sync_blocking_in_async_java.json
  - src/audit/builtin/sync_blocking_in_async_php.json
  - src/audit/builtin/sync_blocking_in_async_python.json
  - src/audit/builtin/sync_blocking_in_async_rust.json
  - src/audit/builtin/sync_blocking_in_async_typescript.json
  - src/audit/json_audit.rs
  - src/audit/pipelines/c/mod.rs
  - src/audit/pipelines/cpp/mod.rs
  - src/audit/pipelines/csharp/mod.rs
  - src/audit/pipelines/go/mod.rs
  - src/audit/pipelines/java/mod.rs
  - src/audit/pipelines/javascript/mod.rs
  - src/audit/pipelines/php/mod.rs
  - src/audit/pipelines/python/mod.rs
  - src/audit/pipelines/rust/mod.rs
  - src/audit/pipelines/typescript/mod.rs
  - src/graph/executor.rs
  - src/graph/pipeline.rs
  - tests/audit_json_integration.rs
findings:
  critical: 0
  warning: 5
  info: 6
  total: 11
status: issues_found
---

# Phase 03: Code Review Report

**Reviewed:** 2026-04-16T00:00:00Z
**Depth:** standard
**Files Reviewed:** 27
**Status:** issues_found

## Summary

This phase migrated complexity pipelines (`cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`) and scalability pipelines (`n_plus_one_queries`, `sync_blocking_in_async` in 8 language variants) from Rust to JSON. The core graph pipeline executor (`executor.rs`) and the pipeline DSL types (`pipeline.rs`) received the new `MatchPattern` and `ComputeMetric` stage implementations needed to support the JSON definitions.

Overall the implementation is sound. The JSON definitions are internally consistent, the executor handles errors gracefully, and the integration tests are well structured. The most significant issues are:

1. A logic error in `comment_to_code_ratio.json` — it selects `"file"` nodes but passes them to `execute_compute_metric`, which will re-parse the file once per symbol node if the prior `Select: symbol` pipeline pattern were used. As written, the `Select: file` does work with the special `comment_to_code_ratio` branch in the executor (lines 824-839), but the executor silently ignores the `line` field on file nodes (always `1`) and recomputes the full-file ratio on every node. This means any file node will have the metric set — this is correct — but the interaction is fragile and undocumented in the JSON definition.

2. The `sync_blocking_in_async_typescript.json` pipeline flags every `member_expression call_expression` with `severity: "warning"` (not `"info"` like all other sync-blocking variants). Combined with its extremely broad match, this will generate orders-of-magnitude more noise than any other variant. The test for the "clean" case (`fetch(...)`) will actually trigger this pipeline because `fetch` is a member expression call.

3. The `is_nolint` field in `WhereClause` is deserialized, stored, and included in `is_empty()` checks, but the `eval()` and `eval_metrics()` implementations explicitly document it as a no-op. This creates a silent correctness gap where JSON authors who write `{"is_nolint": true}` in an `exclude` clause will see the clause have no effect.

4. The `compute_metric` stage in `execute_compute_metric` propagates nodes through even when `file_language` or `read_file` returns `None` — meaning nodes will reach the `Flag` stage without the metric set and will be scored as if the metric is `0`. For `cyclomatic_complexity` this means functions in files that fail language detection will report CC=0, suppressing real findings or generating false-negatives silently.

5. Minor: `find_function_body_at_line` uses a simple DFS stack but the `line` values in the graph are 1-indexed while tree-sitter rows are 0-indexed. The conversion `node.line.saturating_sub(1)` is correct, but if a symbol's `start_line` in the graph was recorded as 0 (malformed data), `saturating_sub(1)` returns 0, which could accidentally match the very first line of the file — this is a latent edge case.

---

## Warnings

### WR-01: TypeScript `sync_blocking_in_async` flags at `warning` severity instead of `info` — and the "clean" test will produce false positives

**File:** `src/audit/builtin/sync_blocking_in_async_typescript.json:10`

**Issue:** All seven other language variants of this pipeline use `"severity": "info"`. The TypeScript variant uses `"severity": "warning"`. The description explicitly acknowledges that the JSON version cannot filter by Sync suffix and cannot verify async containment, so every `member_expression call_expression` is flagged. `await fetch(...)` also expands to a `call_expression` containing a `member_expression`, which means `async function load() { const data = await fetch(...) }` — the exact code used in `sync_blocking_in_async_ts_clean_code()` in the integration tests — will produce a finding from this pipeline, making the "clean" negative test unreliable.

**Fix:**

Lower severity to `"info"` to match all other language variants and be consistent with the acknowledged high false-positive rate:

```json
{
  "pipeline": "sync_blocking_in_async",
  "category": "scalability",
  "description": "...",
  "languages": ["typescript", "javascript"],
  "graph": [
    {
      "match_pattern": "(call_expression (member_expression (property_identifier) @method))"
    },
    {
      "flag": {
        "pattern": "sync_call_in_async",
        "message": "Member expression call `{{name}}` detected -- verify this is not a blocking call in an async context",
        "severity": "info"
      }
    }
  ]
}
```

If the intent is to keep `warning` for TypeScript specifically, the `sync_blocking_in_async_ts_clean_code` integration test should be revised to assert `!findings.iter().any(...)` only for patterns matching a more restrictive criterion, not a blanket absence.

---

### WR-02: `comment_to_code_ratio.json` uses `select: "file"` but `execute_compute_metric` is designed for symbol nodes — metric is computed redundantly and the pipeline description/pipeline interaction is misleading

**File:** `src/audit/builtin/comment_to_code_ratio.json:7-10`

**Issue:** The pipeline selects `file` nodes, then runs `compute_metric: "comment_to_code_ratio"`. The executor's `comment_to_code_ratio` branch (executor.rs:824-839) handles this correctly via a special-case: it computes the ratio on the whole file from the node's `file_path` rather than looking up a function body by line. However:

- The special-case branch silently continues without checking whether the `select` type is `file` or `symbol`. If `comment_to_code_ratio` were ever used after `select: "symbol"`, the metric would be applied per-symbol node but still compute the whole-file ratio — producing one finding per function in the file, all with the same ratio.
- The executor logs a warning when it cannot find a function body at the target line but only for non-`comment_to_code_ratio` metrics. If the file fails to parse, the node is pushed through without the metric set (lines 810-816), and the downstream `flag` stage will then evaluate `comment_to_code_ratio` against `0` — which is less than `5`, triggering a spurious `"warning"` finding for an unparseable file.

**Fix:** In the executor, when `workspace.file_language` or parse fails in the `comment_to_code_ratio` branch, skip the node rather than pushing it through without the metric:

```rust
// In execute_compute_metric, after the parse failure block:
None => {
    eprintln!("Warning: compute_metric: failed to parse {}", node.file_path);
    // Do NOT push the node — metric is unset; downstream Flag would misfire
    continue;
}
```

---

### WR-03: `execute_compute_metric` silently propagates nodes with no metric set when `file_language` or `read_file` returns `None`

**File:** `src/graph/executor.rs:797-800`

**Issue:** When `workspace.file_language(&node.file_path)` or `workspace.read_file(&node.file_path)` returns `None`, the node is pushed into `result` without any metric inserted. The downstream `Flag` stage will call `node.metric_f64(metric_name)` which returns `0.0` for missing keys. For `cyclomatic_complexity` with thresholds `gt: 10` and `gte: 20`, CC=0 will not match and the finding is suppressed — this is a false negative, not a false positive. For `comment_to_code_ratio` with threshold `lt: 5`, ratio=0 (< 5) will produce a spurious warning finding for any file the workspace cannot locate.

```rust
// Current (lines 797-800):
let Some(lang) = workspace.file_language(&node.file_path) else {
    result.push(node);   // <-- node has no metric; will misfire on lt: thresholds
    continue;
};
```

**Fix:** Skip nodes that cannot be resolved rather than forwarding them without metric data:

```rust
let Some(lang) = workspace.file_language(&node.file_path) else {
    eprintln!("Warning: compute_metric: unknown language for {}", node.file_path);
    continue; // skip; no metric to attach
};
let Some(source) = workspace.read_file(&node.file_path) else {
    eprintln!("Warning: compute_metric: cannot read {}", node.file_path);
    continue;
};
```

---

### WR-04: `is_nolint` in `WhereClause` is always a no-op — silently produces incorrect results for any JSON author using it

**File:** `src/graph/pipeline.rs:124-127` and `src/graph/pipeline.rs:306-309`

**Issue:** `is_nolint: Option<bool>` is deserialized and included in `is_empty()`, but both `eval()` and `eval_metrics()` explicitly skip evaluating it. The comment says "Implemented in executor (Task 2) via a separate nolint check," but no code in the executor currently calls any nolint scanning path. A JSON audit author who writes:

```json
"exclude": {"is_nolint": true}
```

will see the clause fully parsed, the `is_empty()` check return `false`, but the actual exclude predicate have no effect — no suppression happens. This is a silent correctness gap.

**Fix:** Either implement the nolint scanning in `eval()`, or remove the field from `WhereClause` until it is implemented. Keeping a silently no-op public field is a maintenance hazard. At minimum, add a runtime warning when `is_nolint` is set:

```rust
if self.is_nolint.is_some() {
    eprintln!("Warning: 'is_nolint' predicate is not yet implemented and has no effect");
}
```

---

### WR-05: `test_builtin_audits_returns_four` test name is stale — the assertion checks `>= 36` but the function is named `returns_four`

**File:** `src/audit/json_audit.rs:151`

**Issue:** The test function is named `test_builtin_audits_returns_four` but the assertion checks `audits.len() >= 36`. The name was not updated when the phase expanded the builtin count from 4 to 36+. This causes confusion about the invariant being enforced and misleads future contributors about the expected count.

**Fix:** Rename the test to reflect its actual assertion:

```rust
#[test]
fn test_builtin_audits_returns_at_least_36() {
    let audits = builtin_audits();
    assert!(audits.len() >= 36, "Expected at least 36 built-in audits, got {}", audits.len());
    ...
}
```

---

## Info

### IN-01: `n_plus_one_queries.json` tree-sitter pattern omits `foreach_statement` and `for_each_statement` loop variants present in many languages

**File:** `src/audit/builtin/n_plus_one_queries.json:7`

**Issue:** The `match_pattern` only covers `for_statement`, `for_in_statement`, `while_statement`, and `do_statement`. TypeScript/JavaScript `for...of` loops use a different node kind (`for_of_statement`) and Python `for` loops use `for_statement` with a different grammar shape. The description acknowledges high false positive rate but does not acknowledge the false negative issue for `for...of`.

**Fix:** Extend the pattern to include `for_of_statement`:

```
[(for_statement body: ...) (for_in_statement body: ...) (for_of_statement body: (_ (expression_statement (call_expression) @call))) (while_statement body: ...) (do_statement body: ...)]
```

---

### IN-02: `sync_blocking_in_async_c.json` and `sync_blocking_in_async_cpp.json` share the same `pipeline` name with no language disambiguation

**File:** `src/audit/builtin/sync_blocking_in_async_c.json:2`, `src/audit/builtin/sync_blocking_in_async_cpp.json` (inferred)

**Issue:** Both C and C++ variants share `"pipeline": "sync_blocking_in_async"` (as do all other variants). This is correct for deduplication logic — the `dedup_key` function in `json_audit.rs` uses both `pipeline` name and language filter to produce a unique key, so `sync_blocking_in_async:c,cpp` is distinct from `sync_blocking_in_async:typescript,javascript`. However, in output, the `finding.pipeline` field for all these variants will be the same string `"sync_blocking_in_async"`, making it impossible for callers to distinguish which language variant triggered a finding from the pipeline name alone.

**Fix:** Consider adding a `pipeline_name` override in the `flag` config to include the language, e.g. `"pipeline_name": "sync_blocking_in_async_c"`. This is low-urgency but affects API consumers filtering by `pipeline` name in structured output.

---

### IN-03: `comment_to_code_ratio.json` description says "value stored as integer percentage (0-100)" but the denominator used is `comment_lines + code_lines`, not `code_lines`

**File:** `src/audit/builtin/comment_to_code_ratio.json:4`, `src/graph/executor.rs:830-834`

**Issue:** The description says "Under-documented = ratio < 5%; over-documented = ratio > 60%." The executor computes:

```rust
let ratio = (comment_lines as f64 / (comment_lines + code_lines) as f64 * 100.0) as i64;
```

This is `comment / (comment + code)` — a share of total lines, not `comment / code`. The description is ambiguous about which denominator is intended. If `code_lines` alone is the denominator (the more common industry definition), a file with 1 comment line and 1 code line would report 100%, not 50%. The current formula always produces values 0-100 regardless, but the thresholds (5% and 60%) were presumably chosen for one formula or the other. If the intent is `comment / (comment + code)`, the description should say "share of total non-blank lines." If the intent is `comment / code_lines`, the formula in executor.rs should be corrected.

This is an informational inconsistency, not a logic error, since both formulas produce results in [0, 100].

---

### IN-04: `n_plus_one_queries` integration test `n_plus_one_queries_ts_clean_code` may be fragile — `db.findOne(1)` is itself a member expression call that will match `sync_blocking_in_async_typescript`

**File:** `tests/audit_json_integration.rs:520-536`

**Issue:** The clean test for `n_plus_one_queries` uses `db.findOne(1)` outside a loop and asserts no `query_in_loop` finding. This is correctly testing `n_plus_one_queries`. However, if this test were ever combined with a scalability pipeline selector that includes both `n_plus_one_queries` and `sync_blocking_in_async`, the `db.findOne(1)` call would produce a finding from the TypeScript `sync_blocking_in_async` pipeline. This is not currently a bug since each test uses `PipelineSelector::Scalability` independently, but is worth documenting as a test isolation note.

No code change needed. Informational.

---

### IN-05: `all language mod.rs` files have `complexity_pipelines()` returning empty `Vec` — missing link between Rust pipeline removal and JSON replacement

**File:** `src/audit/pipelines/rust/mod.rs:47-49`, `src/audit/pipelines/typescript/mod.rs:51-53`, and all other language `mod.rs` files.

**Issue:** Every language's `complexity_pipelines()` returns an empty `Vec`. The complexity pipelines (`cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`) now live in JSON. This is correct per the migration plan. However, the Rust function signatures are still present and called by the engine — the empty return is by design. The issue is that any future contributor adding a Rust complexity pipeline to these functions would not know the JSON path exists, and vice versa. There is no comment explaining why the function returns empty.

**Fix:** Add a comment to each empty `complexity_pipelines()` to explain the intentional gap:

```rust
pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    // Complexity pipelines (cyclomatic_complexity, function_length, cognitive_complexity,
    // comment_to_code_ratio) are implemented as JSON built-in audit definitions in
    // src/audit/builtin/ and run via the JSON pipeline executor. This Rust function
    // is intentionally empty.
    Ok(vec![])
}
```

---

### IN-06: `WhereClause::is_empty()` does not check `is_nolint` consistently with the other boolean fields — it does check it, but `is_empty` returns `false` for `is_nolint: Some(false)` which is logically equivalent to "always true"

**File:** `src/graph/pipeline.rs:162-181`

**Issue:** `is_empty()` returns `false` whenever `is_nolint` is `Some(_)`, even if the value is `Some(false)` (meaning "only include nolint-suppressed nodes — which is always false since nolint is a no-op). Since `is_nolint` is a no-op in `eval()`, an `exclude` clause of `{"is_nolint": false}` would have `is_empty()` return `false` (indicating the clause is active) but `eval()` would not evaluate the predicate — so the exclude would neither exclude everything nor exclude nothing; it would just skip evaluation of that one predicate and check the rest. This is consistent with the no-op status but adds to the confusion around why the field is surfaced at all.

No code change required; this is purely a consequence of WR-04 and is resolved by fixing WR-04.

---

_Reviewed: 2026-04-16T00:00:00Z_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
