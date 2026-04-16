# Phase 2: Executor Stage Implementation - Context

**Gathered:** 2026-04-16
**Status:** Ready for planning

<domain>
## Phase Boundary

Implement `match_pattern` and `compute_metric` executor stages in the JSON pipeline engine. Remove the 5 legacy stub stages (`traverse`, `filter`, `match_name`, `count_edges`, `pair`) whose specifications are unclear. This is pure engine work — no pipeline migrations, no new JSON pipeline files.

The executor must be able to run tree-sitter pattern matching and metric computation per file so that Phase 3 pipeline migrations (tech-debt, scalability) can express their rules as declarative JSON.

</domain>

<decisions>
## Implementation Decisions

### match_pattern: Execution Path

- **D-01:** Add an optional `Option<&Workspace>` parameter to `run_pipeline()` in `executor.rs`. When `Some(workspace)` is present, `match_pattern` uses it to parse files on demand (read from MemoryFileSource, parse with tree-sitter). The engine passes `Some(workspace)` when calling `run_pipeline` for JSON audits.
- **D-02:** `match_pattern` is a **source stage** — it iterates all workspace files filtered by the pipeline's `languages` field, runs the S-expression query on each parsed tree, and emits one `PipelineNode` per AST match with the matched node's file path and line number. It composes directly with `flag`. No prior `select` stage is needed.
- **D-03:** Confirmed: all CLI audit paths (`audit code-quality`, `audit scalability`, `audit security`, `audit architecture`, `audit` full) already build a `CodeGraph` via `GraphBuilder` and pass `Some(&index)` to `engine.run()`. The "silent zero findings" concern in STATE.md is a non-issue — no code change needed here.

### match_pattern: JSON DSL Shape

- **D-04:** New `GraphStage` variant (follows the same untagged-serde pattern as `GroupBy`):
  ```rust
  MatchPattern {
      match_pattern: String,  // tree-sitter S-expression query
  }
  ```
  JSON usage: `{"match_pattern": "(macro_invocation name: (identifier) @name (#eq? @name \"panic\"))"}`.

### compute_metric: Helper Location

- **D-05:** Move compute_metric helper functions (`compute_cyclomatic`, `compute_cognitive`, function length, comment ratio, `ControlFlowConfig`) from `src/audit/pipelines/helpers.rs` to `src/graph/metrics.rs` (new file). `executor.rs` imports from `graph::metrics`. `audit/pipelines/helpers.rs` re-exports from `graph::metrics` for backward compatibility so existing Rust pipelines compile without changes.

### compute_metric: Stage Behavior

- **D-06:** `compute_metric` is a **transform stage** — takes symbol nodes already in the pipeline (from a prior `select(symbol)` stage), re-parses the file via workspace, locates the function body at the node's line, computes the named metric, and sets `metrics["<metric_name>"]` on the node. A subsequent `flag` stage uses the metric value via `severity_map` `when` clauses or `{{metric_name}}` message interpolation.
- **D-07:** New `GraphStage` variant:
  ```rust
  ComputeMetric {
      compute_metric: String,  // e.g. "cyclomatic_complexity", "function_length", "cognitive_complexity", "comment_to_code_ratio"
  }
  ```
  JSON usage: `{"compute_metric": "cyclomatic_complexity"}`.

### Stub Stage Removal

- **D-08:** Delete `traverse`, `filter`, `match_name`, `count_edges`, and `pair` stub variants from the `GraphStage` enum entirely. Their specifications are unclear and they cannot be correctly implemented without a spec. Also delete their config structs (`TraverseConfig`, `FilterConfig`, `MatchNameConfig`, `CountEdgesConfig`, `PairConfig`) from `pipeline.rs`, and remove all tests that reference them. Any JSON pipeline file that references a removed stage will fail at deserialization time with a clear serde error — loud failure, not silent pass-through.

### Test Strategy

- **D-09:** Tests live as unit tests in `executor.rs` (inside `#[cfg(test)]`). Use a minimal `Workspace` with in-memory file bytes (MemoryFileSource). No full `AuditEngine` required. Minimum test set:
  - `test_match_pattern_finds_panic_in_rust` — positive case: Rust source with `panic!()`, S-expression query matches it, emits one finding with correct line
  - `test_match_pattern_no_match_returns_empty` — negative case: Rust source with no panic, emits zero findings
  - `test_compute_metric_cyclomatic_flags_complex_function` — positive case: function with CC > threshold, emits finding
  - `test_compute_metric_cyclomatic_clean_function_no_finding` — negative case: simple function, CC below threshold, no finding

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

No external specs — requirements are fully captured in decisions above and the files below.

### Executor and Pipeline DSL
- `src/graph/executor.rs` — `run_pipeline()` signature to extend; `execute_stage()` dispatch to add `MatchPattern` and `ComputeMetric` arms
- `src/graph/pipeline.rs` — `GraphStage` enum where `MatchPattern` and `ComputeMetric` variants are added; stub variants and their config structs to remove
- `src/graph/mod.rs` — check if `metrics.rs` needs to be declared here

### Engine Integration
- `src/audit/engine.rs` — lines 282-297: the `run_pipeline()` call site that must be updated to pass `Some(workspace)` for JSON audits

### Helper Functions to Move
- `src/audit/pipelines/helpers.rs` — `compute_cyclomatic`, `compute_cognitive`, `compute_function_length`, `compute_comment_ratio`, `ControlFlowConfig` and per-language configs to relocate to `src/graph/metrics.rs`

### Workspace API (for reading files in executor)
- `src/workspace.rs` — `Workspace::read_file()`, `Workspace::file_language()`, `Workspace::files()` — used by `match_pattern` to iterate and parse files

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `execute_select()` in `executor.rs`: Pattern for iterating graph nodes — `match_pattern` follows the same loop structure but iterates workspace files instead
- `parse_content()` in `parser.rs`: Parses source bytes into a tree-sitter `Tree` — `match_pattern` and `compute_metric` both call this per file
- `languages/mod.rs`: `compile_symbol_query()` and related — see how S-expression queries are compiled and matched for the `match_pattern` implementation
- `make_file_graph()` helper in executor.rs tests: Pattern for building minimal test graphs to extend for new tests

### Established Patterns
- `GraphStage` untagged serde enum: Deserialization is order-sensitive — `MatchPattern { match_pattern: String }` and `ComputeMetric { compute_metric: String }` follow the `GroupBy { group_by: String }` pattern exactly
- `PipelineNode.metrics` HashMap: `MetricValue::Int`, `MetricValue::Float`, `MetricValue::Text` — `compute_metric` sets `MetricValue::Int` or `MetricValue::Float` depending on the metric
- `WhereClause.count`, `.depth`, `.ratio` predicates: Already support numeric predicates for severity_map — `compute_metric` output is consumed the same way

### Integration Points
- `src/graph/metrics.rs` (new): Declare `pub mod metrics;` in `src/graph/mod.rs`
- `run_pipeline()` signature: Add `workspace: Option<&Workspace>` as third parameter (before `seed_nodes`)
- `engine.rs` line ~282: Update `run_pipeline(&json_audit.graph, g, None, &json_audit.pipeline)` → `run_pipeline(&json_audit.graph, g, Some(workspace), None, &json_audit.pipeline)`

</code_context>

<specifics>
## Specific Ideas

- `match_pattern` should use `streaming_iterator::StreamingIterator` for `QueryMatches` (same pattern as `languages/typescript.rs` and other language modules — this is a known CLAUDE.md constraint for tree-sitter 0.25)
- The `match_pattern` S-expression query must be compiled via `tree_sitter::Query::new(&language, &query_str)` — compilation errors should produce a descriptive `anyhow::bail!` rather than a panic
- For `compute_metric`: when the symbol's line range cannot be located in the re-parsed tree (e.g., graph was built from a different file version), skip the node with `eprintln!` warning rather than failing the whole pipeline

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 02-executor-stage-implementation*
*Context gathered: 2026-04-16*
