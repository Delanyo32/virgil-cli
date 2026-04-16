---
phase: 02-executor-stage-implementation
reviewed: 2026-04-16T00:00:00Z
depth: standard
files_reviewed: 8
files_reviewed_list:
  - src/audit/engine.rs
  - src/audit/pipelines/helpers.rs
  - src/graph/executor.rs
  - src/graph/metrics.rs
  - src/graph/mod.rs
  - src/graph/pipeline.rs
  - src/main.rs
  - src/query_engine.rs
findings:
  critical: 0
  warning: 4
  info: 5
  total: 9
status: issues_found
---

# Phase 02: Code Review Report

**Reviewed:** 2026-04-16
**Depth:** standard
**Files Reviewed:** 8
**Status:** issues_found

## Summary

This phase introduces the graph executor pipeline (`executor.rs`), a metrics module (`metrics.rs`), and plumbs both into `engine.rs` and `query_engine.rs`. The design is well-structured — stages compose cleanly, the `run_pipeline` / `execute_graph_pipeline` split is sensible, and the parallel rayon architecture in `engine.rs` is intact. No security or crash-level bugs were found. The warnings below are logic correctness issues that could produce incorrect output or silent data loss under specific inputs. The info items flag maintainability concerns.

---

## Warnings

### WR-01: `execute_count` uses the representative node's `file_path`, not the group key, causing wrong `file` in findings

**File:** `src/graph/executor.rs:389`
**Issue:** `execute_count` picks `members[0]` as the representative after grouping by a metric key (e.g. `"file_path"`). When the group key is `"file_path"` this is fine — the first member will belong to the right file. However, when `group_by` uses `"language"` or `"kind"` (both documented group-by fields in `execute_group_by`), the representative's `file_path` will be an arbitrary file belonging to that language/kind group, not the group key itself. If a `flag` stage subsequently uses `{{file}}` in its message, the finding will report the wrong file. The issue is most visible when grouping by `"language"` and flagging the group — the finding's `file_path` will be whichever file happened to be `members[0]`.

**Fix:** After selecting the representative, set `rep.file_path` to the group key when the group key is not a file path (or at least preserve `_group` so the flag message can use `{{_group}}`). The simplest safe fix:

```rust
let group_key_str = key.clone();
let mut rep = members[0].clone();
rep.metrics
    .insert("count".to_string(), MetricValue::Int(members.len() as i64));
// Preserve group key so messages can reference it
rep.metrics
    .insert("_group".to_string(), MetricValue::Text(group_key_str));
result.push(rep);
```

### WR-02: `execute_compute_metric` silently pushes nodes without a metric when `find_function_body_at_line` fails, allowing unflagged nodes through threshold filters

**File:** `src/graph/executor.rs:847-854`
**Issue:** When `find_function_body_at_line` returns `None` (line not found — e.g., symbol line is off by one, or it is a method inside a class), the code emits a warning and pushes the node with no metric set. If a downstream `Flag` stage is used **without** a threshold `count`/`depth` `where` clause (i.e. unconditional flag), that node will be flagged despite having no valid metric. Conversely, if the flag has a `when` clause like `count >= 10`, the node with a missing metric evaluates to `metric_f64("cyclomatic_complexity") == 0.0`, which may silently pass or fail the threshold incorrectly. The missing warning message also only goes to `stderr` — callers in the server path will not surface it.

**Fix:** Either skip the node entirely when the body is not found (remove it from the result set), or set a sentinel metric value and document it:

```rust
let Some(body) = body_node else {
    // Skip nodes where we cannot locate the function body — avoids
    // emitting findings with metric=0 which would be misleading.
    // (Node omitted from result; calling stage will simply not flag it.)
    continue; // or: result.push(node); with an explicit 0 sentinel + doc
};
```

### WR-03: `ordered_cycle_path_for_edge` infinite loop risk on large SCCs with no outgoing edges of the requested type

**File:** `src/graph/executor.rs:1041-1068`
**Issue:** The outer `'outer: loop` in `ordered_cycle_path_for_edge` will never terminate if the SCC contains a node that has no outgoing edges of `edge_type` to any other SCC member **and** no other SCC member has been visited. In that situation the "Couldn't complete" fallback is reached and `return` is executed — the loop does break via that path. However, in a degenerate case where `path.last()` gets stuck cycling between two visited nodes (both already in `visited`) without the `start` node being reachable, the loop can run indefinitely because neither `next == start` nor `!visited.contains(&next)` will ever be true for any unvisited node.

Specifically: if `current` has outgoing edges only to `start` (but `path.len() == 1` so `next == start` fails the `path.len() > 1` guard) AND all other outgoing edges are to already-visited nodes, the loop iterates through all edges, falls through without matching either inner `if`, and repeats the outer loop from the same `current` node forever.

**Fix:** Add a visited-edge guard or break the loop after a full traversal without progress:

```rust
'outer: loop {
    let current = *path.last().unwrap();
    let mut made_progress = false;
    for edge in graph.graph.edges_directed(current, Direction::Outgoing) {
        // ... existing logic ...
        if !visited.contains(&next) {
            visited.insert(next);
            path.push(next);
            made_progress = true;
            continue 'outer;
        }
    }
    // No progress made — fall back
    let mut sorted: Vec<String> = members
        .iter()
        .map(|&idx| node_path(&graph.graph[idx]))
        .filter(|p| !p.is_empty())
        .collect();
    sorted.sort();
    return sorted.join(" -> ");
}
```

### WR-04: `run_json_audit_file_ws` in `main.rs` calls `run_pipeline` without passing the workspace, so `match_pattern` and `compute_metric` stages will hard-error

**File:** `src/main.rs:766`
**Issue:** The call to `run_pipeline` in `run_json_audit_file_ws` passes `None` for the `workspace` parameter:

```rust
let output = virgil_cli::graph::executor::run_pipeline(
    &json_audit.graph, &graph, None, None, None, &json_audit.pipeline
)?;
```

If the JSON audit file's `graph` field contains a `match_pattern` or `compute_metric` stage, `execute_stage` will return a hard `bail!` error:

```
match_pattern stage requires workspace -- call run_pipeline with Some(workspace)
```

This means `virgil audit --file <audit.json>` will fail for any JSON audit using those stages, even though the workspace has already been built. The workspace is available in scope as `workspace: &Workspace`.

**Fix:**

```rust
let output = virgil_cli::graph::executor::run_pipeline(
    &json_audit.graph,
    &graph,
    Some(workspace),  // pass the workspace
    json_audit.languages.as_deref(),
    None,
    &json_audit.pipeline,
)?;
```

---

## Info

### IN-01: `walk_all` in `metrics.rs` accepts an unused `cursor` parameter

**File:** `src/graph/metrics.rs:186-197`
**Issue:** The `cursor` parameter in `walk_all` is accepted but immediately dropped (`let _ = cursor;`). The comment says "Keep cursor alive for borrow checker", but the stack-based traversal creates its own cursors internally and does not use the passed-in one. This is dead code and misleads readers about the function's contract.

**Fix:** Remove the parameter and update all three call sites (`compute_cyclomatic`, `compute_cognitive` indirectly via `walk_all`, `compute_comment_ratio`). If the borrow-checker concern is real, document it with a concrete explanation. If not, drop the parameter.

```rust
fn walk_all<F: FnMut(Node)>(node: Node, f: &mut F) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        f(current);
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
}
```

### IN-02: `AuditEngine::new()` hardcodes `Language::Rust` as the default language

**File:** `src/audit/engine.rs:43-49`
**Issue:** The `new()` constructor initialises `languages` to `vec![Language::Rust]`. Every caller in `main.rs` immediately overrides this via `.languages(...)`, so it is not a bug today, but it is a silent footgun: any future code that calls `AuditEngine::new().run(...)` without setting languages will silently audit only Rust files. The `Default` impl delegates to `new()` and thus has the same issue.

**Fix:** Default to an empty Vec and document that callers are expected to call `.languages(...)`:

```rust
pub fn new() -> Self {
    Self {
        languages: Vec::new(), // caller must set via .languages(...)
        pipeline_filter: Vec::new(),
        pipeline_selector: PipelineSelector::TechDebt,
        progress: None,
        project_dir: None,
    }
}
```

### IN-03: `execute_graph_pipeline` backward-compat wrapper drops `workspace` and `pipeline_languages`

**File:** `src/graph/executor.rs:41-48`
**Issue:** The `execute_graph_pipeline` compatibility wrapper hardcodes `None` for `workspace` and `pipeline_languages`. Any caller using this wrapper with a pipeline that contains `match_pattern` or `compute_metric` stages will receive a hard error (see WR-04). Since this is a public API (`pub fn`), external callers are silently broken. The rustdoc comment says "Prefer calling run_pipeline directly" but does not explain why.

**Fix:** Update the doc comment to explicitly note the limitation, or deprecate the function:

```rust
/// Alias for [`run_pipeline`] kept for backward compatibility.
///
/// **Limitation:** passes `workspace = None` — pipelines using `match_pattern`
/// or `compute_metric` stages will return an error. Use [`run_pipeline`] directly
/// for full functionality.
#[deprecated(note = "Use run_pipeline directly to pass a workspace")]
pub fn execute_graph_pipeline(...) { ... }
```

### IN-04: `is_safe_expression` recursive call risks stack overflow on pathological AST input

**File:** `src/audit/pipelines/helpers.rs:1143-1170`
**Issue:** `is_safe_expression` recurses through `parenthesized_expression` and `binary_expression` nodes. While most real-world code is shallow, a deeply nested parenthesized expression (e.g. from generated code or obfuscated input) could overflow the stack. The project's CLAUDE.md explicitly notes stack concerns and uses `4MB` rayon stacks. For binary expressions the recursion goes through all named children, each of which may themselves be binary expressions.

**Fix:** Replace the recursion with an explicit stack:

```rust
pub fn is_safe_expression(
    root: tree_sitter::Node,
    is_literal: impl Fn(tree_sitter::Node) -> bool + Copy,
) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if is_literal(node) { continue; }
        if node.kind() == "parenthesized_expression" {
            if let Some(inner) = node.named_child(0) {
                stack.push(inner);
                continue;
            }
            return false;
        }
        if node.kind() == "binary_expression" {
            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            if children.is_empty() { return false; }
            for child in children { stack.push(child); }
            continue;
        }
        return false;
    }
    true
}
```

### IN-05: `find_enclosing_function_callers` iterates all graph nodes on every call — O(n) per finding

**File:** `src/audit/pipelines/helpers.rs:1381-1411`
**Issue:** This helper iterates `graph.graph.node_indices()` (all nodes in the graph) to find the narrowest enclosing symbol for a given `(file_path, line)`. It is called from pipeline checks, potentially once per finding per file. For large codebases this is O(findings * graph_nodes) per audit run. Calling this inside a rayon-parallel per-file closure compounds the cost without benefit since the graph is immutable.

This is flagged as info (not warning) because it is a performance issue and out of v1 scope, but the access pattern is fragile: if the call ever moves into a hot inner loop, it becomes a correctness-affecting bottleneck (timeouts, dropped findings under server 120s limit).

**Fix:** Pre-build a `HashMap<(String, u32..u32), NodeIndex>` spatial index in `CodeGraph` at build time, or at minimum document the O(n) cost prominently so callers avoid it in hot paths.

---

_Reviewed: 2026-04-16_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
