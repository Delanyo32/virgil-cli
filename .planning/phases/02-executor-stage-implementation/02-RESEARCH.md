# Phase 2: Executor Stage Implementation - Research

**Researched:** 2026-04-16
**Domain:** Rust audit executor — tree-sitter query dispatch, metric computation, stub stage removal
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

- **D-01:** Add `Option<&Workspace>` as a new parameter to `run_pipeline()` in `executor.rs`. When `Some(workspace)` is present, `match_pattern` uses it to parse files on demand. The engine passes `Some(workspace)` when calling `run_pipeline` for JSON audits.
- **D-02:** `match_pattern` is a **source stage** — it iterates all workspace files filtered by the pipeline's `languages` field, runs the S-expression query on each parsed tree, and emits one `PipelineNode` per AST match with the matched node's file path and line number.
- **D-03:** Confirmed: all CLI audit paths already build a `CodeGraph` via `GraphBuilder` and pass `Some(&index)` to `engine.run()`. No code change needed for the silent-zero-findings concern.
- **D-04:** New `GraphStage` variant (untagged-serde): `MatchPattern { match_pattern: String }`. JSON usage: `{"match_pattern": "(macro_invocation name: (identifier) @name (#eq? @name \"panic\"))"}`.
- **D-05:** Move compute_metric helper functions (`compute_cyclomatic`, `compute_cognitive`, function length, comment ratio, `ControlFlowConfig`) from `src/audit/pipelines/helpers.rs` to `src/graph/metrics.rs` (new file). `executor.rs` imports from `graph::metrics`. `audit/pipelines/helpers.rs` re-exports from `graph::metrics` for backward compatibility.
- **D-06:** `compute_metric` is a **transform stage** — takes symbol nodes already in the pipeline (from a prior `select(symbol)` stage), re-parses the file via workspace, locates the function body at the node's line, computes the named metric, and sets `metrics["<metric_name>"]` on the node.
- **D-07:** New `GraphStage` variant: `ComputeMetric { compute_metric: String }`. JSON usage: `{"compute_metric": "cyclomatic_complexity"}`. Supported metrics: `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`.
- **D-08:** Delete `Traverse`, `Filter`, `MatchName`, `CountEdges`, and `Pair` stub variants from `GraphStage` entirely. Also delete their config structs (`TraverseConfig`, `FilterConfig`, `MatchNameConfig`, `CountEdgesConfig`, `PairConfig`) from `pipeline.rs`, and remove all tests that reference them. JSON pipelines referencing removed stages will fail at deserialization with a clear serde error.
- **D-09:** Tests live as unit tests in `executor.rs` (inside `#[cfg(test)]`). Minimum four tests: `test_match_pattern_finds_panic_in_rust`, `test_match_pattern_no_match_returns_empty`, `test_compute_metric_cyclomatic_flags_complex_function`, `test_compute_metric_cyclomatic_clean_function_no_finding`.

### Claude's Discretion

None — discussion stayed within phase scope; decisions cover all implementation choices.

### Deferred Ideas (OUT OF SCOPE)

None — discussion stayed within phase scope.
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| ENG-03 | `match_pattern` stage implemented in executor — accepts tree-sitter S-expression, runs per-file, emits matching nodes as findings | Full implementation path documented: signature, streaming_iterator pattern, language dispatch, workspace integration |
| ENG-04 | `compute_metric` stage implemented — wires helpers.rs functions into stage dispatch | Per-language ControlFlowConfig dispatcher approach documented; metrics.rs move pattern described |
| ENG-05 | Stub stages removed: traverse, filter, match_name, count_edges, pair — each fails loudly or is deleted | D-08 decision: full deletion confirmed; struct list identified; test cleanup scope defined |
</phase_requirements>

---

## Summary

Phase 2 implements two new `GraphStage` variants (`match_pattern` and `compute_metric`) in `src/graph/executor.rs`, moves the compute_metric helper functions to a new `src/graph/metrics.rs` module, and deletes five stub stage variants from `GraphStage` and their config structs from `pipeline.rs`.

The implementation work has three independent tracks: (1) the `match_pattern` source stage uses the existing workspace + tree-sitter parsing infrastructure already present in the codebase — the main novelty is wiring `Option<&Workspace>` into `run_pipeline()` and writing the streaming_iterator loop over query matches; (2) the `compute_metric` transform stage moves the existing `ControlFlowConfig` + helper functions from `helpers.rs` to a new `graph/metrics.rs` and dispatches by metric name and language; (3) the stub stage deletions are mechanical removes across `pipeline.rs`, `executor.rs`, and their tests.

The key risk is the `run_pipeline` signature change: the new `Option<&Workspace>` parameter changes the function signature, so the call site in `engine.rs` at lines 282-297 and the `execute_graph_pipeline` compatibility wrapper both need updating. There is no circular dependency concern — `graph/executor.rs` already imports from `audit::pipelines::helpers` and `crate::workspace::Workspace` is already imported transitively via the module tree.

**Primary recommendation:** Work in this order — (1) add `metrics.rs`, (2) add `MatchPattern`/`ComputeMetric` to `GraphStage` and implement `execute_match_pattern`/`execute_compute_metric` in executor, (3) update `run_pipeline` signature and `engine.rs` call site, (4) delete stub stages and their tests, (5) add the four new unit tests.

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| match_pattern execution | `graph/executor.rs` | `workspace.rs`, `parser.rs` | Executor orchestrates; workspace provides bytes; parser provides Tree |
| compute_metric execution | `graph/executor.rs` | `graph/metrics.rs` | Executor dispatches; metrics module provides per-language configs and compute functions |
| Helper function location | `graph/metrics.rs` (new) | `audit/pipelines/helpers.rs` (re-exports) | Graph layer is the right owner for metric computation used by executor |
| run_pipeline signature | `graph/executor.rs` | `audit/engine.rs` (call site) | executor.rs owns the signature; engine.rs is the only external caller |
| Stub stage removal | `graph/pipeline.rs` | `graph/executor.rs` | pipeline.rs owns enum/config structs; executor.rs owns dispatch arms |
| Language-to-config dispatch | `graph/metrics.rs` | `language.rs` | New function: `control_flow_config_for_language(Language) -> ControlFlowConfig` |
| Function-body-locating query | `graph/metrics.rs` | `languages/` modules | New per-language S-expression queries to find function body nodes by start line |

---

## Standard Stack

### Core (all pre-existing — no new deps needed)
[VERIFIED: codebase grep]

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tree-sitter | 0.25 | AST parsing + S-expression query matching | Already in use; `match_pattern` reuses the same `Query::new` + `QueryCursor` + streaming_iterator pattern |
| streaming-iterator | 0.1 | `QueryMatches` iteration | CLAUDE.md constraint for tree-sitter 0.25 — `QueryMatches` is not `std::Iterator` |
| rayon | 1.11 | Parallel file parsing | Already used in workspace loading and query engine |
| anyhow | 1.0 | Error handling with context | Already used throughout |
| serde_json | 1 | GraphStage deserialization | Already handles all existing variants via untagged enum |

### No new Cargo.toml entries required
All dependencies already present. `graph/metrics.rs` uses only types from `tree-sitter` and `crate::language::Language` — both already in scope.

---

## Architecture Patterns

### System Architecture Diagram

```
JSON audit file
    │
    ▼
AuditEngine::run()          ← engine.rs
    │  passes Some(workspace)
    ▼
run_pipeline(stages, graph, workspace, pipeline_name)   ← executor.rs
    │
    ├── MatchPattern stage  ─────────────────────────────────────────────────┐
    │   │  for each file in workspace filtered by pipeline.languages:        │
    │   │    1. workspace.read_file(path) → Arc<str>                         │
    │   │    2. parser::create_parser(lang) → Parser                         │
    │   │    3. parser.parse(source) → Tree                                  │
    │   │    4. Query::new(ts_lang, match_pattern) → Query  [compile once]   │
    │   │    5. cursor.matches(query, root, source) → QueryMatches           │
    │   │    6. while let Some(m) = matches.next() → emit PipelineNode       │
    │   └── returns Vec<PipelineNode>                                         │
    │                                                                         │
    ├── ComputeMetric stage ──────────────────────────────────────────────────┤
    │   │  for each PipelineNode (symbol from prior select(symbol) stage):   │
    │   │    1. workspace.read_file(node.file_path) → Arc<str>               │
    │   │    2. create_parser(lang) + parse → Tree                            │
    │   │    3. locate function body by node.line in tree                     │
    │   │    4. dispatch to metrics::compute_* via metric_name               │
    │   │    5. node.metrics.insert(metric_name, MetricValue::Int(value))    │
    │   └── returns Vec<PipelineNode> with metrics populated                  │
    │                                                                         │
    └── Flag stage  ──────────────────────────────────────────────────────────┘
        │  maps PipelineNode → AuditFinding using node.metrics
        ▼
    PipelineOutput::Findings(Vec<AuditFinding>)
```

### Recommended Project Structure Changes

```
src/
├── graph/
│   ├── mod.rs           # add: pub mod metrics;
│   ├── executor.rs      # change: run_pipeline signature + MatchPattern/ComputeMetric arms
│   ├── pipeline.rs      # change: add 2 variants, remove 5 variants + 5 config structs
│   └── metrics.rs       # NEW: ControlFlowConfig + per-language configs + compute_* functions
└── audit/
    └── pipelines/
        └── helpers.rs   # change: re-export from graph::metrics (backward compat)
```

### Pattern 1: Untagged Serde Enum Addition

Both new `GraphStage` variants follow the existing `GroupBy` pattern exactly — a struct variant with a single field whose name is the discriminant key. Ordering in the enum matters for serde's untagged deserialization (first variant whose fields match wins).

[VERIFIED: codebase read of pipeline.rs]

```rust
// In pipeline.rs GraphStage enum:
#[serde(untagged)]
pub enum GraphStage {
    // ... existing variants ...
    MatchPattern {
        match_pattern: String,   // S-expression query string
    },
    ComputeMetric {
        compute_metric: String,  // metric name: "cyclomatic_complexity", etc.
    },
    Flag {
        flag: FlagConfig,
    },
}
```

Placement: Insert `MatchPattern` and `ComputeMetric` before `Flag` (Flag must remain last so it deserializes correctly as the terminal stage). The existing `Traverse`, `Filter`, `MatchName`, `CountEdges`, `Pair` variants are deleted.

### Pattern 2: streaming_iterator for QueryMatches (tree-sitter 0.25)

[VERIFIED: CLAUDE.md constraint + codebase read of rust_lang.rs and cyclomatic.rs]

```rust
// Source: src/languages/rust_lang.rs + CLAUDE.md
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

let mut cursor = QueryCursor::new();
let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
while let Some(m) = matches.next() {
    // m.captures contains the matched nodes
    for cap in m.captures {
        let node = cap.node;
        let line = node.start_position().row as u32 + 1;
        // emit PipelineNode...
    }
}
```

`QueryMatches` uses `streaming_iterator::StreamingIterator`, NOT `std::iter::Iterator`. Using `.for_each()` or `for` loop directly on it will fail to compile.

### Pattern 3: match_pattern as Source Stage

`match_pattern` starts with an empty `nodes` vec and produces new nodes directly, unlike `select` which reads the graph. The iteration is over `workspace.files()` filtered by language.

[VERIFIED: codebase read of executor.rs execute_select + workspace.rs API]

```rust
fn execute_match_pattern(
    query_str: &str,
    workspace: &Workspace,
    languages: Option<&[String]>,  // from JsonAuditFile.languages, passed through
) -> anyhow::Result<Vec<PipelineNode>> {
    use streaming_iterator::StreamingIterator;

    let mut result = Vec::new();

    for rel_path in workspace.files() {
        let Some(lang) = workspace.file_language(rel_path) else { continue };

        // Apply language filter if provided
        if let Some(langs) = languages {
            let lang_str = lang.as_str();
            if !langs.iter().any(|l| l.eq_ignore_ascii_case(lang_str)) {
                continue;
            }
        }

        let Some(source) = workspace.read_file(rel_path) else { continue };
        let ts_lang = lang.tree_sitter_language();

        // Compile query per language (can cache per run_pipeline call if needed)
        let query = match tree_sitter::Query::new(&ts_lang, query_str) {
            Ok(q) => q,
            Err(e) => anyhow::bail!("match_pattern: invalid S-expression query: {e}"),
        };

        let mut parser = crate::parser::create_parser(lang)?;
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => {
                eprintln!("Warning: match_pattern: failed to parse {rel_path}");
                continue;
            }
        };

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let node = cap.node;
                let line = node.start_position().row as u32 + 1;
                result.push(PipelineNode {
                    node_idx: petgraph::graph::NodeIndex::new(0), // synthetic
                    file_path: rel_path.clone(),
                    name: node.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
                    kind: node.kind().to_string(),
                    line,
                    exported: false,
                    language: lang.as_str().to_string(),
                    metrics: std::collections::HashMap::new(),
                });
            }
        }
    }
    Ok(result)
}
```

**Note on `node_idx`:** `match_pattern` nodes are not backed by graph nodes. Using `NodeIndex::new(0)` as a sentinel is safe — `node_idx` is only used in `execute_find_cycles` and `execute_max_depth` which require graph nodes, and `match_pattern` nodes will not be fed into those stages. The planner should document this in the implementation task.

### Pattern 4: compute_metric as Transform Stage

`compute_metric` takes the existing `nodes` vec (symbol nodes from `select(symbol)`) and augments each with a metric value. It re-parses the file from workspace and locates the function body that begins at `node.line`.

[VERIFIED: codebase read of helpers.rs, cyclomatic.rs patterns]

```rust
fn execute_compute_metric(
    metric_name: &str,
    nodes: Vec<PipelineNode>,
    workspace: &Workspace,
) -> anyhow::Result<Vec<PipelineNode>> {
    use streaming_iterator::StreamingIterator;

    let mut result = Vec::new();

    for mut node in nodes {
        let Some(lang) = workspace.file_language(&node.file_path) else {
            result.push(node);
            continue;
        };
        let Some(source) = workspace.read_file(&node.file_path) else {
            result.push(node);
            continue;
        };

        let mut parser = crate::parser::create_parser(lang)?;
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => {
                eprintln!("Warning: compute_metric: failed to parse {}", node.file_path);
                result.push(node);
                continue;
            }
        };

        let config = crate::graph::metrics::control_flow_config_for_language(lang);
        let target_line = node.line.saturating_sub(1) as usize; // 0-indexed

        // Find function body node at target_line using a language-appropriate query
        let body_node = find_function_body_at_line(tree.root_node(), target_line, source.as_bytes(), lang);
        let Some(body) = body_node else {
            eprintln!("Warning: compute_metric: no function body at line {} in {}", node.line, node.file_path);
            result.push(node);
            continue;
        };

        let value = match metric_name {
            "cyclomatic_complexity" => {
                crate::graph::metrics::compute_cyclomatic(body, &config, source.as_bytes()) as i64
            }
            "function_length" => {
                let (lines, _) = crate::graph::metrics::count_function_lines(body);
                lines as i64
            }
            "cognitive_complexity" => {
                crate::graph::metrics::compute_cognitive(body, &config, source.as_bytes()) as i64
            }
            "comment_to_code_ratio" => {
                // comment_ratio operates on the whole file root, not just function body
                let (comment_lines, code_lines) = crate::graph::metrics::compute_comment_ratio(
                    tree.root_node(), source.as_bytes(), &config
                );
                let ratio = if code_lines > 0 {
                    (comment_lines as f64 / (comment_lines + code_lines) as f64 * 100.0) as i64
                } else { 0 };
                ratio
            }
            other => {
                anyhow::bail!("compute_metric: unknown metric '{other}' — supported: cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio");
            }
        };

        node.metrics.insert(metric_name.to_string(), MetricValue::Int(value));
        result.push(node);
    }
    Ok(result)
}
```

### Pattern 5: Function Body Location by Line

`compute_metric` needs to find the function body node whose start line matches `node.line`. The approach is a tree walk that finds the narrowest function-like node containing `target_line`.

[VERIFIED: codebase read of helpers.rs find_enclosing_function_callers]

```rust
fn find_function_body_at_line<'a>(
    root: tree_sitter::Node<'a>,
    target_line: usize,
    source: &[u8],
    lang: Language,
) -> Option<tree_sitter::Node<'a>> {
    // Walk tree, find function node whose start_line == target_line, return its body child
    let func_kinds = function_node_kinds_for_language(lang); // per-language
    let body_field = body_field_for_language(lang);          // e.g., "body" or "block"

    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if func_kinds.contains(&node.kind()) && node.start_position().row == target_line {
            if let Some(body) = node.child_by_field_name(body_field) {
                return Some(body);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    None
}
```

The per-language function kinds and body field names must be added to `metrics.rs`. These are extractable directly from the existing cyclomatic pipeline sources.

### Pattern 6: run_pipeline Signature Update

[VERIFIED: codebase read of executor.rs + engine.rs]

Current signature:
```rust
pub fn run_pipeline(
    stages: &[GraphStage],
    graph: &CodeGraph,
    seed_nodes: Option<Vec<NodeIndex>>,
    pipeline_name: &str,
) -> anyhow::Result<PipelineOutput>
```

New signature (D-01):
```rust
pub fn run_pipeline(
    stages: &[GraphStage],
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    seed_nodes: Option<Vec<NodeIndex>>,
    pipeline_name: &str,
) -> anyhow::Result<PipelineOutput>
```

Call site in `engine.rs` line 282 must change from:
```rust
match crate::graph::executor::run_pipeline(
    &json_audit.graph,
    g,
    None,          // seed_nodes
    &json_audit.pipeline,
)
```
to:
```rust
match crate::graph::executor::run_pipeline(
    &json_audit.graph,
    g,
    Some(workspace),   // workspace — NEW
    None,              // seed_nodes
    &json_audit.pipeline,
)
```

The `execute_graph_pipeline` compatibility wrapper (line 40-47 of executor.rs) also needs updating to pass `None` for workspace to maintain backward compatibility.

### Pattern 7: Stub Stage Deletion

Items to delete from `pipeline.rs`:
- `GraphStage::Traverse { traverse: TraverseConfig }` variant
- `GraphStage::Filter { filter: FilterConfig }` variant
- `GraphStage::MatchName { match_name: MatchNameConfig }` variant
- `GraphStage::CountEdges { count_edges: CountEdgesConfig }` variant
- `GraphStage::Pair { pair: PairConfig }` variant
- Structs: `TraverseConfig`, `FilterConfig`, `MatchNameConfig`, `CountEdgesConfig`, `PairConfig`

Items to delete from `executor.rs`:
- `execute_stage` match arms for `Traverse`, `Filter`, `MatchName`, `CountEdges`, `Pair`

Tests to delete from `pipeline.rs` `#[cfg(test)]`:
- `test_deserialize_traverse_stage` (line ~1030)
- `test_deserialize_find_cycles_stage` — this one stays (find_cycles is kept)
- `test_deserialize_count_edges_stage` (line ~1076)
- `test_deserialize_match_name_stage` (line ~1105)

[VERIFIED: codebase read of pipeline.rs tests section]

### Anti-Patterns to Avoid

- **Using `for` loop on `QueryMatches` directly:** `QueryMatches` implements `StreamingIterator`, not `Iterator`. Using `for m in matches` will fail to compile. Always use `while let Some(m) = matches.next()`.
- **Compiling the S-expression query inside a rayon task:** Query compilation is language-specific and relatively cheap, but if called per-file in a rayon parallel loop, errors are silently discarded. Compile the query once before the parallel loop if `match_pattern` is later parallelized.
- **Returning `anyhow::Error` for unknown metrics mid-pipeline:** An unknown metric name in `compute_metric` should bail early with a descriptive error (as shown in the Pattern 4 example). Do NOT silently pass nodes through — that would produce a silent no-op, violating ENG-05's "fail loudly" requirement.
- **`node_idx` confusion:** `match_pattern` nodes use a synthetic `NodeIndex::new(0)`. Do not feed these into `execute_find_cycles` or `execute_max_depth` — those stages expect real graph nodes. The executor stage dispatch should document this invariant.
- **Re-using a `tree_sitter::Parser` across rayon tasks:** `Parser` is `!Send`. Create one per task/invocation as all other pipeline code does.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Streaming tree-sitter iteration | Custom iterator adapter | `streaming_iterator::StreamingIterator::next()` | API contract for tree-sitter 0.25 — other patterns don't compile |
| Cyclomatic complexity | Custom CFG analysis | `compute_cyclomatic()` from metrics.rs | Already tested and correct; see helpers.rs tests |
| Cognitive complexity | Custom nesting tracker | `compute_cognitive()` from metrics.rs | Stack-based, avoids stack overflow on deep ASTs |
| Function line counting | Line-split + count | `count_function_lines()` from metrics.rs | Handles multi-line bodies, statement counting |
| Per-language tree-sitter language handle | Hardcoded grammar constants | `Language::tree_sitter_language()` | Central dispatch; handles all 12 language variants |

**Key insight:** The entire compute_metric implementation is a wiring exercise — the hard algorithmic work (cyclomatic complexity, cognitive complexity, function length) is already implemented and tested in `helpers.rs`. The task is moving those functions to a better module location and calling them from the executor.

---

## Common Pitfalls

### Pitfall 1: Serde Untagged Enum Ordering
**What goes wrong:** If `MatchPattern { match_pattern: String }` is placed after `GroupBy { group_by: String }` but the JSON is `{"group_by": "file"}`, serde tries each variant in order. If both have a `String` field and the key names differ, it works. But if order is wrong relative to `Flag`, a `{"flag": {...}}` that appears before `Flag` variant in the enum may match an earlier variant's `String` field unexpectedly.
**Why it happens:** `#[serde(untagged)]` tries variants in declaration order, first-match wins.
**How to avoid:** Keep `Flag` as the last variant in the enum. New variants (`MatchPattern`, `ComputeMetric`) go before `Flag`. Verify via the existing `test_deserialize_flag_stage` test.
**Warning signs:** `flag` stages deserializing as `MatchPattern` or other string-keyed variants.

### Pitfall 2: run_pipeline Signature Change Breaks execute_graph_pipeline Wrapper
**What goes wrong:** `execute_graph_pipeline` in executor.rs (line 40-47) is a compatibility wrapper that calls `run_pipeline`. After the signature change, it will fail to compile unless updated.
**Why it happens:** The wrapper is a thin pass-through; it must propagate the new `workspace` parameter.
**How to avoid:** Update both `run_pipeline` and `execute_graph_pipeline` in the same commit. Add `workspace: Option<&Workspace>` to the wrapper too, and pass it through. Alternatively, hard-code `None` in the wrapper since it is described as a backward-compat shim and its callers don't pass workspace.
**Warning signs:** Compile error at `execute_graph_pipeline` call site.

### Pitfall 3: match_pattern Language Filter vs. Pipeline Language Filter
**What goes wrong:** `JsonAuditFile.languages` is `Option<Vec<String>>`. The engine's `engine.rs` already filters by language before calling `run_pipeline`. But inside `run_pipeline`, `match_pattern` iterates `workspace.files()` — which may contain files in many languages. Without an additional language filter inside `execute_match_pattern`, it would try to run a Rust S-expression query against TypeScript files.
**Why it happens:** `workspace` is not pre-filtered by the pipeline's languages.
**How to avoid:** Pass the `json_audit.languages` filter into `run_pipeline` (or derive it from context) and apply it inside `execute_match_pattern`. The simplest approach: add a `pipeline_languages: Option<&[String]>` argument to `execute_match_pattern`, extracted from the pipeline's JSON.
**Warning signs:** Tree-sitter query compilation errors for wrong-language files, or spurious matches in wrong-language files.

### Pitfall 4: compute_metric Fails to Find Body Node
**What goes wrong:** `compute_metric` tries to find the function body node at `node.line`. But `node.line` comes from `NodeWeight::Symbol::start_line` in the graph, which is set during graph construction. If the file changed between graph construction and metric computation (workspace is always the same in-memory version, so this shouldn't happen), or if the symbol is a non-function (method, arrow_function, etc. with different body fields), the body lookup may fail.
**Why it happens:** `node.line` is 1-indexed (from `Symbol.start_line` in the graph); tree-sitter positions are 0-indexed. Off-by-one error in line comparison is common.
**How to avoid:** Convert node.line to 0-indexed (`target_line = node.line.saturating_sub(1)`) before comparing with `node.start_position().row`. Log a warning and continue (not bail) when body not found.
**Warning signs:** All `compute_metric` findings are empty even for known complex functions.

### Pitfall 5: Circular Dependency When Importing Workspace from graph::executor
**What goes wrong:** `graph/executor.rs` importing `crate::workspace::Workspace` might create a circular dependency if `workspace.rs` imports from `graph/`.
**Why it happens:** Module dependency cycles in Rust.
**How to avoid:** Verify `workspace.rs` imports — it only uses `crate::discovery`, `crate::file_source`, `crate::language`, `crate::s3`. It does NOT import from `crate::graph`. So the import direction `graph::executor → workspace` is safe with no cycle. [VERIFIED: codebase read of workspace.rs imports lines 1-11]
**Warning signs:** Compiler error "cycle detected when computing the crate dependency graph".

### Pitfall 6: helpers.rs Re-export Completeness
**What goes wrong:** After moving functions to `metrics.rs`, if `helpers.rs` re-exports are incomplete, existing Rust pipelines that import from `helpers.rs` will fail to compile.
**Why it happens:** Many files import specific functions: `use crate::audit::pipelines::helpers::{ControlFlowConfig, compute_cyclomatic}`. Each moved symbol must have a corresponding `pub use graph::metrics::*` or explicit re-export.
**How to avoid:** Use `pub use crate::graph::metrics::{ControlFlowConfig, compute_cyclomatic, compute_cognitive, count_function_lines, compute_comment_ratio};` in helpers.rs. Run `cargo build` after moving to verify zero new compile errors.
**Warning signs:** `unresolved import` errors in pipeline files.

---

## Code Examples

### Complete match_pattern Stage Integration into execute_stage

[VERIFIED: codebase read of executor.rs execute_stage dispatch]

The dispatch arm in `execute_stage` signature must accommodate `workspace`:

```rust
fn execute_stage(
    stage: &GraphStage,
    nodes: Vec<PipelineNode>,
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    pipeline_languages: Option<&[String]>,
    _pipeline_name: &str,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<PipelineNode>> {
    match stage {
        // ... existing arms ...
        GraphStage::MatchPattern { match_pattern } => {
            match workspace {
                Some(ws) => execute_match_pattern(match_pattern, ws, pipeline_languages),
                None => anyhow::bail!(
                    "match_pattern stage requires workspace — call run_pipeline with Some(workspace)"
                ),
            }
        }
        GraphStage::ComputeMetric { compute_metric } => {
            match workspace {
                Some(ws) => execute_compute_metric(compute_metric, nodes, ws),
                None => anyhow::bail!(
                    "compute_metric stage requires workspace — call run_pipeline with Some(workspace)"
                ),
            }
        }
        // Removed: Traverse, Filter, MatchName, CountEdges, Pair
    }
}
```

### Test Pattern Using MemoryFileSource (D-09)

[VERIFIED: codebase read of executor.rs tests + workspace.rs]

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_source::MemoryFileSource;
    use crate::language::Language;
    use crate::workspace::Workspace;
    use std::sync::Arc;

    fn make_workspace_with_file(rel_path: &str, content: &str, lang: Language) -> Workspace {
        let mut file_map = std::collections::HashMap::new();
        let mut size_map = std::collections::HashMap::new();
        file_map.insert(rel_path.to_string(), Arc::from(content));
        size_map.insert(rel_path.to_string(), content.len() as u64);
        let source = Box::new(MemoryFileSource::new(file_map, size_map));
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert(rel_path.to_string(), lang);
        Workspace::from_parts(std::path::PathBuf::from("."), source, lang_map)
    }

    #[test]
    fn test_match_pattern_finds_panic_in_rust() {
        let source = r#"fn foo() { panic!("oops"); }"#;
        let ws = make_workspace_with_file("src/lib.rs", source, Language::Rust);
        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern: r#"(macro_invocation
                  name: (identifier) @name
                  (#eq? @name "panic")) @call"#.to_string(),
            },
            GraphStage::Flag {
                flag: crate::graph::pipeline::FlagConfig {
                    pattern: "panic_detected".to_string(),
                    message: "panic at {{file}}:{{line}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, "panic_detection").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert_eq!(findings.len(), 1);
                assert_eq!(findings[0].line, 1);
                assert_eq!(findings[0].pattern, "panic_detected");
            }
            _ => panic!("expected Findings"),
        }
    }
}
```

**Note:** `Workspace::from_parts` is a constructor that does NOT exist yet. Either add it to `workspace.rs` or replicate the test setup using `tempfile` to create an actual temp directory (as existing workspace tests do). The simpler path for unit tests is `tempfile::tempdir()` + `write` + `Workspace::load()`.

### metrics.rs Module Structure

[VERIFIED: based on helpers.rs read]

```rust
// src/graph/metrics.rs
use crate::language::Language;
use tree_sitter::Node;

// Re-exported from old location — helpers.rs will re-export these
pub struct ControlFlowConfig { /* existing fields */ }
pub fn compute_cyclomatic(body: Node, config: &ControlFlowConfig, source: &[u8]) -> usize { ... }
pub fn compute_cognitive(body: Node, config: &ControlFlowConfig, source: &[u8]) -> usize { ... }
pub fn count_function_lines(body: Node) -> (usize, usize) { ... }
pub fn compute_comment_ratio(root: Node, source: &[u8], config: &ControlFlowConfig) -> (usize, usize) { ... }

/// Dispatch per-language ControlFlowConfig for the four supported metrics.
pub fn control_flow_config_for_language(lang: Language) -> ControlFlowConfig {
    match lang {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => ts_config(),
        Language::Rust => rust_config(),
        Language::Python => python_config(),
        Language::Go => go_config(),
        Language::Java => java_config(),
        Language::C => c_config(),
        Language::Cpp => cpp_config(),
        Language::CSharp => csharp_config(),
        Language::Php => php_config(),
    }
}

/// Per-language function node kinds and body field name for locate-by-line.
pub fn function_node_kinds_for_language(lang: Language) -> &'static [&'static str] { ... }
pub fn body_field_for_language(lang: Language) -> &'static str { ... }
```

The per-language `ControlFlowConfig` values can be copied directly from the existing per-language `cyclomatic.rs` files. Each language directory (`rust/`, `typescript/`, `go/`, etc.) has its own `config()` function — these become the named functions in `metrics.rs`.

---

## Runtime State Inventory

Not applicable — this is a greenfield engine implementation phase, not a rename/refactor/migration phase.

---

## Environment Availability

Step 2.6: No external dependencies beyond the Rust toolchain. All tools verified present via test run above.

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| Rust stable toolchain | All compilation | Yes | Rust 2024 edition | — |
| cargo test | Test verification | Yes | cargo (from toolchain) | — |
| tree-sitter 0.25 | match_pattern | Yes | 0.25 (Cargo.lock) | — |
| streaming-iterator | match_pattern QueryMatches | Yes | 0.1 (Cargo.lock) | — |

[VERIFIED: `cargo test --lib --quiet` → 2559 passed; 0 failed]

---

## Validation Architecture

### Test Framework

| Property | Value |
|----------|-------|
| Framework | Rust built-in test harness |
| Config file | none (Cargo.toml `[lib]` + `[[test]]`) |
| Quick run command | `cargo test --lib graph::executor` |
| Full suite command | `cargo test` |

### Phase Requirements to Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| ENG-03 | match_pattern finds panic!() in Rust source | unit | `cargo test --lib graph::executor::tests::test_match_pattern` | ❌ Wave 0 |
| ENG-03 | match_pattern returns empty for no-match source | unit | `cargo test --lib graph::executor::tests::test_match_pattern_no_match` | ❌ Wave 0 |
| ENG-04 | compute_metric cyclomatic flags complex function | unit | `cargo test --lib graph::executor::tests::test_compute_metric_cyclomatic` | ❌ Wave 0 |
| ENG-04 | compute_metric cyclomatic: clean function no finding | unit | `cargo test --lib graph::executor::tests::test_compute_metric_cyclomatic_clean` | ❌ Wave 0 |
| ENG-05 | Deleted stub stages fail at deserialization time | unit | `cargo test --lib graph::pipeline::tests` | ❌ Wave 0 (remove traverse/match_name/count_edges deser tests) |
| TEST-02 | No regressions in full test suite | regression | `cargo test` | ✅ existing |

### Sampling Rate

- **Per task commit:** `cargo test --lib graph::executor graph::pipeline`
- **Per wave merge:** `cargo test`
- **Phase gate:** `cargo test` green before `/gsd-verify-work`

### Wave 0 Gaps

- [ ] `src/graph/metrics.rs` — new file, all compute metric functions
- [ ] Four new unit tests in `src/graph/executor.rs` `#[cfg(test)]`
- [ ] Test helper: in-memory workspace construction (either `Workspace::from_parts` constructor or `tempfile`-based setup)

---

## Open Questions

1. **Workspace constructor for tests**
   - What we know: Existing executor tests use `make_file_graph()` with manual node construction. Workspace tests in `workspace.rs` use `tempfile::tempdir()` + disk writes.
   - What's unclear: The cleanest test setup for `execute_match_pattern` tests — `Workspace::from_parts` doesn't exist; `tempfile` works but requires I/O.
   - Recommendation: Add a package-private `Workspace::from_parts(root, source_box, lang_map)` constructor to `workspace.rs` for test use. Three lines of code; avoids disk I/O in unit tests. Alternative: use `tempfile` (already a dev-dependency from workspace.rs tests) for the four new unit tests.

2. **pipeline_languages threading into execute_match_pattern**
   - What we know: `JsonAuditFile.languages` is the language filter. It's available in `engine.rs` but not currently passed to `run_pipeline`.
   - What's unclear: Whether to thread it through `run_pipeline`'s new signature or derive it from the pipeline stages at call time.
   - Recommendation: The simplest approach — add `pipeline_languages: Option<&[String]>` as a parameter to `run_pipeline` alongside `workspace`. The planner should decide this explicitly. Alternatively, apply the language filter only in `execute_match_pattern` based on the node's language after parsing, not before.

3. **`comment_to_code_ratio` metric semantics in compute_metric context**
   - What we know: `compute_comment_ratio` in helpers.rs operates on the entire file root node, not a single function body. When `compute_metric: "comment_to_code_ratio"` is used with a `select(symbol)` pipeline, it would compute the file-level ratio and apply it to each symbol node in that file.
   - What's unclear: Whether this semantic (file-level ratio applied to per-symbol nodes) is correct or whether each symbol should get the ratio of its own body.
   - Recommendation: Implement as file-level ratio per symbol (compute once per file, apply to all symbols in that file) with a comment in the code. Document the semantics in the `ComputeMetric` stage doc comment.

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Hard-coded `include_str!` for JSON pipelines | `include_dir!` macro auto-discovery | Phase 1 | JSON audit files drop-in without code change |
| Architecture pipelines in Rust code | JSON pipeline files | Phase 1 | 36 JSON files, 0 Rust architecture pipeline files |
| Stub stage pass-through (TODO comments) | Deleted with loud serde error | Phase 2 | No silent no-ops in JSON pipeline execution |
| `compute_cyclomatic` / helpers in `audit/pipelines/helpers.rs` | Moved to `graph/metrics.rs`, re-exported | Phase 2 | Graph layer owns metric computation; no circular dep concern |

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `MemoryFileSource::new(file_map, size_map)` accepts `HashMap<String, Arc<str>>` and `HashMap<String, u64>` | Code Examples (test helper) | Compilation error in test; need to check MemoryFileSource constructor signature |
| A2 | All CLI audit paths (`tech-debt`, `complexity`, `security`, `scalability`) build and pass `Some(&graph)` to engine.run() | Architecture | If any path passes `None`, JSON pipelines using `match_pattern` for those categories would silently receive `None` workspace |

[VERIFIED A1 partial: workspace.rs line 69 shows `MemoryFileSource::new(file_map, size_map)` — file_map is `HashMap<String, Arc<str>>`, size_map is `HashMap<String, u64>`]

**D-03 note:** A2 is explicitly resolved by decision D-03 in CONTEXT.md — confirmed not a concern.

If this table is empty for verified claims: All other claims in this research were verified against the codebase.

---

## Sources

### Primary (HIGH confidence)
- `src/graph/executor.rs` — current `run_pipeline` / `execute_stage` signatures; test helpers; all existing stage implementations [VERIFIED: full file read]
- `src/graph/pipeline.rs` — `GraphStage` enum; config structs; `PipelineNode`; `MetricValue`; tests to keep/delete [VERIFIED: full file read]
- `src/audit/pipelines/helpers.rs` — `ControlFlowConfig`, `compute_cyclomatic`, `compute_cognitive`, `count_function_lines`, `compute_comment_ratio` [VERIFIED: full file read]
- `src/audit/engine.rs` lines 260-299 — `run_pipeline` call site; `json_audit.languages` access pattern [VERIFIED: file read]
- `src/workspace.rs` — `Workspace` API: `files()`, `read_file()`, `file_language()` [VERIFIED: file read]
- `src/parser.rs` — `create_parser()`, `parse_content()` signatures [VERIFIED: file read]
- `src/language.rs` — `Language::tree_sitter_language()`, `Language::as_str()` [VERIFIED: file read]
- `src/languages/rust_lang.rs` — streaming_iterator pattern for QueryMatches [VERIFIED: file read]
- `src/audit/pipelines/rust/cyclomatic.rs` — per-language ControlFlowConfig + function query pattern [VERIFIED: file read]
- CLAUDE.md — tree-sitter 0.25 streaming_iterator constraint [VERIFIED: project instructions]

### Secondary (MEDIUM confidence)
- `cargo test` output: 2559 tests pass, 0 fail [VERIFIED: live test run]

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all dependencies pre-existing, verified in Cargo.toml
- Architecture: HIGH — executor/pipeline structure fully read; no ambiguity
- Pitfalls: HIGH — identified from codebase structure + CLAUDE.md constraints
- Test strategy: HIGH — 4 required tests specified in D-09, helper pattern clear

**Research date:** 2026-04-16
**Valid until:** 2026-05-16 (stable Rust ecosystem, no external service dependencies)
