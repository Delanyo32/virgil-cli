# Feature Landscape: JSON Audit Pipeline Format

**Domain:** Declarative static-analysis pipeline DSL for multi-language code auditing
**Researched:** 2026-04-16
**Source:** Direct inspection of `src/audit/json_audit.rs`, `src/graph/pipeline.rs`, all four
             `src/audit/builtin/*.json` files, `audit_plans/` specs for Rust, Python, TypeScript,
             JavaScript, Go architectures and tech debt, and `audit_plans/cross_file_analyzers.md`.

---

## What the JSON Engine Already Supports (Baseline)

The existing `GraphStage` enum in `src/graph/pipeline.rs` defines the current pipeline vocabulary.
Understanding the gap between "what the engine parses" and "what the audit plans require" is the
central task. The engine already handles:

| Stage | What It Does | Used By |
|-------|-------------|---------|
| `select` (file/symbol/call_site) | Entry-point node selection with `where`/`exclude` predicates | All 4 builtin pipelines |
| `group_by` (field name) | Aggregate nodes by file or other attribute | `api_surface_area`, `module_size_distribution` |
| `count` + threshold | Count nodes in a group, filter by gte/lte/gt/lt/eq | `module_size_distribution` |
| `count_edges` (edge type + direction + threshold) | Count graph edges on a node | Defined in schema, not yet used in builtins |
| `max_depth` (edge, skip_barrel_files, threshold) | Topological longest-path over an edge type | `dependency_depth` |
| `find_cycles` (edge type) | Tarjan SCC cycle detection | `circular_dependencies` |
| `ratio` (numerator filter, denominator filter, threshold) | Compute filtered/total ratio | `api_surface_area` |
| `traverse` (edge, direction, depth) | BFS graph traversal | Defined in schema |
| `filter` (no_incoming, no_outgoing, has_edge) | Structural topology filter | Defined in schema |
| `match_name` (glob, regex, contains, starts_with, ends_with) | Symbol name matching | Defined in schema |
| `pair` (acquire_edge, release_edge) | Resource lifecycle pairing | Defined in schema |
| `flag` (pattern, message, severity / severity_map) | Emit findings | All 4 builtin pipelines |
| `WhereClause` predicates | `is_test_file`, `is_generated`, `is_barrel_file`, `exported`, `count`, `cycle_size`, `depth`, `edge_count`, `ratio`, `and/or/not` | All builtin pipelines |
| `severity_map` | Conditional severity based on `when` clauses | `circular_dependencies`, `dependency_depth`, `module_size_distribution` |
| Template interpolation | `{{name}}`, `{{file}}`, `{{line}}`, `{{count}}`, `{{depth}}`, `{{cycle_path}}`, `{{ratio}}`, `{{cycle_size}}`, `{{edge_count}}`, `{{kind}}`, `{{language}}` | All flag stages |

---

## Table Stakes

Capabilities every pipeline that migrates from Rust needs. Without these, the JSON format cannot
express the full detection scope of the current codebase.

### 1. Per-Symbol / Per-Function Pattern Matching via tree-sitter queries

**Why expected:** Every single Rust pipeline (cyclomatic_complexity, function_length,
cognitive_complexity, dead_code, duplicate_code, coupling, panic_detection, etc.) works by running
a tree-sitter S-expression query against a function/class/symbol node and then applying a
threshold or structural check. The JSON engine currently only operates on graph nodes — it has no
mechanism to embed a tree-sitter query inline and match specific AST patterns.

**What this means for JSON:** Every category-2 tech debt pipeline (complexity, code-style,
tech-debt) is a per-AST-node check, not a graph traversal. To migrate `cyclomatic_complexity`,
the format needs a way to say "for each function in this file, compute its CC and flag if > 10."
The current JSON format has no `compute_metric` or `tree_sitter_query` stage.

**Complexity:** High — requires a new stage type or a separate lightweight execution path.
This is the single biggest gap between current JSON engine capability and what all tech debt
pipelines require.

| Feature | Complexity | Notes |
|---------|------------|-------|
| Named metric computation (cyclomatic_complexity, cognitive_complexity, function_length, comment_ratio) | High | Per-function AST walk; language-specific decision point lists |
| Threshold-on-computed-metric + flag | Medium | Extends existing `count`/`flag` composition |
| Severity graduation on computed metric | Low | `severity_map` already supports this |

### 2. `is_nolint` / `is_noqa` / Suppression Comment Check

**Why expected:** All audit plans (python, typescript, go, rust, javascript) identify the absence
of suppression awareness as a critical problem with current Rust implementations. The `WhereClause`
struct already has an `is_nolint` field (commented as "reserved for future executor integration"),
confirming the engine knows it needs this. The `helpers.rs` `is_noqa_suppressed()` function exists
for Python pipelines.

**What this means for JSON:** The `select` or `filter` stage needs `is_nolint: true` in its
`exclude` predicate to be a live filter, not a no-op. Alternatively, a `check_suppression` stage
that reads comment nodes adjacent to a finding and suppresses it. The Python plans require
`# noqa`, Go plans require `//nolint:errcheck`, TypeScript plans require `// @ts-ignore` /
`// eslint-disable-next-line`, Rust plans require `#[allow(...)]`.

**Complexity:** Medium — requires reading comment nodes near a flagged line. The `is_nolint` hook
in `WhereClause` is already plumbed; the executor just needs to implement it.

| Feature | Complexity | Notes |
|---------|------------|-------|
| `is_nolint: true` / `is_noqa: true` in `exclude` predicate | Medium | Executor must scan comment siblings for suppression markers |
| Language-specific suppression token list | Low | Configurable via pipeline JSON or hardcoded per language |

### 3. Graduated Severity via `severity_map` on Any Computed Metric

**Why expected:** Every audit plan identifies flat severity as a deficiency. The `severity_map`
mechanism already exists and works for `cycle_size`, `depth`, and `count`. The gap is that it
cannot currently reference a computed metric like `cyclomatic_complexity` or `line_count` unless
those metrics are first computed and stored on the `PipelineNode`.

**What this means for JSON:** Once metric computation stages (see item 1) store their results in
`PipelineNode.metrics`, `severity_map` with `when: {metric_name: {gte: X}}` works without any
changes to the `severity_map` machinery. This is effectively free once the compute-metric stage
exists.

**Complexity:** Low (dependent on item 1 being implemented first).

### 4. Language-Specific File Exclusion Predicates

**Why expected:** All audit plans require excluding generated files (`*_pb2.py`, `*_generated.ts`,
`*.pb.go`), vendor directories, migration files (`migrations/*.py`), and declaration files
(`*.d.ts`). The `is_generated` predicate exists in `WhereClause` but its implementation in
`helpers.rs` only checks a limited set of patterns.

**What this means for JSON:** The `is_generated` helper needs to be expanded to cover all
language-specific generated file conventions across all 9 languages. Alternatively, an
`exclude_path_glob` stage field on `select` would allow each JSON pipeline to specify its own
exclusion globs inline without modifying the helper.

**Complexity:** Low-to-Medium. Expanding `is_generated` is low effort but a global change.
A per-pipeline `exclude_path_glob` field in `select` is cleaner and scoped to each rule.

| Feature | Complexity | Notes |
|---------|------------|-------|
| Expanded `is_generated` covering all 9 language patterns | Low | Edit helpers.rs, affects all pipelines equally |
| Per-pipeline `exclude_path_glob` in select stage | Low | New field on `SelectConfig`, evaluated by executor |

### 5. `languages` Filter per Pipeline

**Why expected:** All existing builtin JSONs have a top-level `languages` field (confirmed in
`JsonAuditFile` struct). Tech debt pipelines are language-specific — `panic_detection` only applies
to Rust, `var_usage` only to JavaScript. This is already implemented at the `JsonAuditFile` level.

**Status:** Already supported. No new work needed. Each JSON file can declare its language scope.

---

## Differentiators

Capabilities that some pipelines need but not all. Not every JSON pipeline will use these, but
enough of the ~300 Rust pipelines require them that the migration cannot complete without them.

### 1. Inline Name-Pattern Matching (match_name stage)

**Why valuable:** Dead code detection, coupling analysis, any pipeline that checks symbol names
(e.g., `is_entry_file` checks for `main.rs`, `index.ts`, `__init__.py`). The `match_name` stage
is already defined in `GraphStage` but not used in any builtin JSON yet.

**Coverage:** Architecture pipelines (`anemic_module` entry-file exclusion), dead code detection
(symbols named `test_*` or `mock_*`), coupling (barrel file detection by name pattern).

**Complexity:** Low — stage already exists in schema. Needs executor implementation.

### 2. Edge Count Threshold Patterns

**Why valuable:** Coupling pipelines (`high_efferent_coupling`, `high_afferent_coupling`,
`hub_module_bidirectional`) all need to count incoming or outgoing `Imports` / `Calls` edges on
a file or symbol node. The `count_edges` stage is already defined in `GraphStage` with `edge`,
`direction`, and `threshold` fields.

**Coverage:** Architecture `coupling` pipeline, cross-file `CouplingAnalyzer` replacement,
`barrel_file_reexport` detection.

**Complexity:** Low — stage definition exists. Needs executor implementation.

### 3. Visitor-Level Structural Checks (AST parent-chain walking)

**Why valuable:** Several tech debt patterns require checking an AST node's parent chain:
- `is_inside_defer` (Go error_swallowing) — parent is `defer_statement`
- `is_inside_test` — parent chain reaches a test function/method
- `var_at_module_scope` vs `var_in_function` — distance to `program` root
- Callback depth counting (JavaScript `callback_hell`) — recursive nesting count

**Coverage:** Go, JavaScript, TypeScript tech debt pipelines. These are per-node structural
checks that the graph cannot answer — only the AST can.

**Complexity:** High — this requires embedding an AST traversal description in JSON, which is
fundamentally harder to express declaratively than graph queries. The current JSON format has no
"walk parent chain" stage.

**Recommendation:** Express parent-chain checks as a set of named `context` predicates (e.g.,
`"context": "inside_defer"`, `"context": "module_scope"`) that the executor maps to concrete
AST checks. This keeps JSON declarative while letting the executor contain the tree-walking logic.

### 4. Compound Structural Patterns (multi-capture tree-sitter queries)

**Why valuable:** Many Rust pipelines match multiple captures from one tree-sitter query (e.g.,
the cyclomatic pipeline captures `@fn_name`, `@fn_body`, and `@func` simultaneously). These
multi-capture patterns allow correlating information within the same AST subtree (function name +
body + outer node for line reporting).

**Coverage:** Every complexity pipeline, function_length, dead_code (needs symbol + usage count
in same scope), duplicate_code (needs two matching subtrees).

**Complexity:** Very High for full generality. For the specific patterns needed (function name +
body), a set of named built-in compound checks (`find_function_with_body`, `find_class_with_methods`)
is more tractable than a general multi-capture stage.

**Recommendation:** Provide named compute-metric stages for the most common compound patterns
rather than a general-purpose multi-capture query embedding.

### 5. Suppression Comment Token Configuration

**Why valuable:** Different languages use different suppression idioms. If the executor hardcodes
`// @ts-ignore` as the only TypeScript suppression token, Go's `//nolint` will not be recognized.
Each pipeline JSON should be able to specify which comment patterns count as inline suppression.

**Coverage:** All 9 language groups per the audit plans.

**Complexity:** Low — add an optional `suppress_if_comment_matches` array to the pipeline JSON
top-level or to the `flag` stage.

### 6. Project-Relative Threshold Computation

**Why valuable:** `coupling`, `module_size_distribution`, and `dependency_depth` audit plans all
call out absolute thresholds as a primary deficiency (Arch-1, Arch-12). The correct approach is
`mean + N*stddev` across the project.

**Coverage:** Architecture pipelines and any pipeline where threshold relevance depends on project
size.

**Complexity:** Very High — requires a two-pass execution model (first pass computes project
statistics, second pass applies dynamic thresholds). The current JSON engine is single-pass
per-file with no accumulation stage.

**Recommendation:** Defer project-relative thresholds entirely. Use the improved fixed thresholds
specified in the audit plans (graduated vs. flat) as the migration target. Document this as a
future enhancement in the pipeline format spec.

---

## Anti-Features

Things the JSON pipeline format should explicitly NOT attempt to support. These belong in Rust,
not in JSON declarations.

### 1. CFG-Based Data Flow (FlowsTo / Taint Propagation)

**Why avoid in JSON:** Security pipelines (taint analysis, injection detection, SQL injection, XSS)
require following `FlowsTo` edges from a `source` node through a call graph to a `sink` node,
checking for `SanitizedBy` interruptions. This is inherently a stateful graph traversal with
conditional logic (sanitizer checks, path sensitivity). The existing `TaintEngine` in
`src/graph/taint.rs` implements this correctly in Rust.

**What to do instead:** Security pipelines that require taint analysis remain as `GraphPipeline`
Rust implementations. The JSON format expresses what it can (scope exclusion, pattern matching,
severity rules) but does not try to encode taint propagation paths.

**Marker:** If an audit plan's "Replacement Pipeline Design" says the target trait is
`GraphPipeline` AND requires `FlowsTo` edge traversal, that pipeline is a Rust pipeline, not
a JSON pipeline for this milestone.

### 2. Cyclomatic Complexity and Cognitive Complexity Computation

**Why avoid in JSON:** Computing CC requires walking every control-flow construct in a function
body and counting decision points, then counting logical operators in binary expressions, then
counting ternaries. This is a per-language O(AST-size) walk with language-specific node kind
lists. It is not a graph query and cannot be expressed as a composition of select/filter/count
stages.

**What to do instead:** These remain as Rust `NodePipeline` implementations. The JSON format can
declare thresholds and severity maps, but the computation itself stays in Rust as a named metric
provider. If a future "compute_metric" stage is added, complexity metrics become expressible in
JSON — but that stage does not exist yet and is out of scope for this migration.

**Marker:** `cyclomatic_complexity`, `cognitive_complexity`, `comment_to_code_ratio` pipelines
do NOT migrate to JSON in this milestone. They remain as Rust `NodePipeline` files.

### 3. Deep AST Parent-Chain Walking with Arbitrary Depth

**Why avoid in JSON:** Patterns like "find all `callback_hell` cases where arrow functions are
nested more than 3 levels deep inside `arguments` nodes" require a recursive AST walk with a
depth accumulator. Expressing general recursion in a declarative JSON stage language would require
a loop/recursion primitive that contradicts the "linear stage pipeline" design of the engine.

**What to do instead:** Callback nesting depth detection remains a Rust `NodePipeline`. If the
pattern can be approximated as "symbol has depth metric > N" using graph data, the JSON approach
works. Otherwise, keep it in Rust.

### 4. Cross-Symbol Duplicate Code Detection

**Why avoid in JSON:** `duplicate_code` detection (detecting functionally equivalent code blocks)
requires hashing AST subtrees, then comparing hashes across all functions in a file or across
files. This is an O(N*M) operation across all function pairs. It cannot be expressed as a linear
sequence of graph stages over individual nodes.

**What to do instead:** `duplicate_code` pipeline stays as Rust. It is already a well-defined
`Pipeline` implementation and does not gain anything from JSON migration.

### 5. Resource Lifecycle Pair Analysis (Acquire/Release)

**Why avoid in JSON:** The `pair` stage exists in `GraphStage` but resource lifecycle analysis
requires pairing `Acquires` and `ReleasedBy` edges across CFG paths — not just counting them.
A resource acquired in one CFG branch but not released in all branches is the target pattern.
This path-sensitive analysis is implemented in `src/graph/resource.rs` and cannot be expressed
as a declarative JSON pipeline.

**What to do instead:** Resource lifecycle pipelines (`memory_leak_indicators`,
`sync_blocking_in_async`) remain as Rust `GraphPipeline` implementations. The `pair` stage in
the JSON schema may be repurposed for simpler acquire/release counting patterns in the future.

### 6. Multi-File Similarity / Clone Detection

**Why avoid in JSON:** Detecting structurally similar code across multiple files (a sophisticated
form of duplicate_code) requires comparing AST fingerprints across file boundaries. This is a
cross-file batch analysis that the current JSON engine's per-file execution model cannot support.

### 7. Return Type Inspection and Signature Analysis

**Why avoid in JSON:** Patterns like "function returns `error` as last return value" (Go
`error_swallowing`) or "function returns a concrete type instead of an interface" (Go
`concrete_return_type`) require inspecting the function's return type signature from the AST.
The graph's `SymbolInfo` stores a signature string but does not parse it into typed return values.
Implementing this as a JSON predicate would require either a very specific built-in predicate
(language-specific) or a full expression language, both of which are scope-creep.

**What to do instead:** These remain as Rust `Pipeline` implementations that use tree-sitter
queries to inspect return type nodes directly.

---

## Feature Dependencies

```
Suppression check (is_nolint) → WhereClause.is_nolint implementation in executor [currently a no-op]

severity_map on computed metrics → compute_metric stage (item 1 in Table Stakes)
  — severity_map machinery already works once metric is stored in PipelineNode.metrics

Edge count threshold (count_edges stage) → executor implementation of CountEdges stage
  — stage struct exists in GraphStage, executor just needs to handle the variant

Graduated severity → either severity_map (already works) OR compute_metric (new) for custom metrics

match_name filtering → executor implementation of MatchName stage
  — stage struct exists in GraphStage, executor just needs to handle the variant

per-pipeline exclude_path_glob → new field on SelectConfig in pipeline.rs
  — small schema addition + executor check
```

---

## MVP Recommendation for This Migration

The goal is to migrate the ~300 Rust pipeline files to JSON. Given the constraint that
`json_audit.rs` engine internals are not to be rewritten, the achievable migration splits into
two tiers:

**Tier 1 — Migrate to JSON immediately (engine already supports):**
- All 4 architecture pipelines that use only: `select`, `group_by`, `count`, `ratio`,
  `count_edges`, `max_depth`, `find_cycles`, `flag` with `severity_map`
- The migration of per-language architecture pipelines (`module_size_distribution`,
  `api_surface_area`, `circular_dependencies`, `dependency_depth`) across all 9 language groups
  by adding `languages: ["typescript"]` etc. to the shared pipeline JSONs
- Any pipeline reducible to symbol counting + threshold + severity graduation

**Tier 2 — Requires minor engine additions before migration:**
- Pipelines needing `is_nolint` suppression (implement the no-op in executor)
- Pipelines needing `match_name` or `count_edges` (implement executor variants — structs exist)
- Pipelines needing expanded `is_generated` patterns (edit helpers.rs)

**Stay in Rust (out of scope for JSON migration):**
- `cyclomatic_complexity`, `cognitive_complexity`, `comment_to_code_ratio` — metric computation
- Security pipelines using `FlowsTo` / taint traversal
- `duplicate_code` — hash-based multi-subtree comparison
- `memory_leak_indicators` — resource lifecycle path analysis
- `callback_hell`, `nested_callbacks` — recursive AST depth accumulation
- Any pipeline whose audit plan specifies a `GraphPipeline` target AND requires `FlowsTo` edges

**Defer:**
- Project-relative threshold computation (mean + stddev baseline)
- A general-purpose `compute_metric` stage embedding arbitrary tree-sitter walks
- The `pair` stage's full CFG path-sensitivity

---

## Sources

- `src/audit/json_audit.rs` — JsonAuditFile schema and discovery logic (direct read)
- `src/graph/pipeline.rs` — GraphStage enum and all stage config structs (direct read)
- `src/audit/builtin/*.json` — 4 existing JSON pipelines (direct read)
- `src/audit/pipeline.rs` — Pipeline/NodePipeline/GraphPipeline trait definitions (direct read)
- `src/audit/pipelines/typescript/cyclomatic.rs` — Representative Rust pipeline (direct read)
- `src/audit/pipelines/helpers.rs` — compute_cyclomatic, compute_cognitive, is_noqa_suppressed (direct read)
- `audit_plans/rust_architecture.md` — Rust architecture pipeline analysis (direct read)
- `audit_plans/python_architecture.md` — Python architecture pipeline analysis (direct read)
- `audit_plans/typescript_architecture.md` — TypeScript architecture pipeline analysis (direct read)
- `audit_plans/javascript_tech_debt.md` — JavaScript tech debt pipeline analysis (direct read)
- `audit_plans/go_tech_debt.md` — Go tech debt pipeline analysis (direct read)
- `audit_plans/cross_file_analyzers.md` — Cross-file ProjectAnalyzer analysis (direct read)
- `audit_plans/architecture_rubrics.md` — Arch-1 through Arch-15 rubric definitions (direct read)
- `.planning/PROJECT.md` — Migration milestone requirements (direct read)
- `.planning/codebase/ARCHITECTURE.md` — System architecture analysis (direct read)
