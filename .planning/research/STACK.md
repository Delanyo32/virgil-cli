# Technology Stack: JSON-Driven Audit Pipeline Format

**Project:** virgil-cli — Audit Pipeline JSON Migration
**Researched:** 2026-04-16
**Scope:** Declarative JSON/YAML rule definition formats for static analysis; what the virgil-cli JSON engine supports today; what it needs to support to replace 310 Rust pipeline files

---

## What We Are Deciding

virgil-cli already has a working JSON pipeline engine (`src/graph/pipeline.rs` + `src/audit/json_audit.rs`) with 4 working pipelines in `src/audit/builtin/`. The question is not "what technology should we use" — the Rust stack and JSON format are already chosen and working. The question is: **does the current JSON format have sufficient expressive power to represent the remaining ~306 Rust pipelines**, and what additions (if any) are needed?

This document answers that question by comparing the current format against industry tools (Semgrep, CodeQL, ast-grep, SonarQube) and auditing what the existing Rust pipelines actually do.

---

## Current JSON Engine Capabilities (Verified from Source)

The `GraphStage` enum in `src/graph/pipeline.rs` defines every available stage. The following is a complete inventory as of the code at HEAD:

### Stage Types

| Stage | Purpose | Key Config |
|-------|---------|------------|
| `select` | Entry point — load file or symbol nodes from CodeGraph | `node_type` (file/symbol/call_site), `where`, `exclude` |
| `traverse` | Follow graph edges to adjacent nodes | `edge` (calls/imports/flows_to/acquires/released_by/contains/exports/defined_in), `direction` (in/out/both), `depth` |
| `group_by` | Aggregate nodes by a field | field name string ("file") |
| `count` | Filter groups to those meeting a count threshold | `threshold` (NumericPredicate) |
| `count_edges` | Count edges of a type on each node | `edge`, `direction`, `threshold` |
| `max_depth` | BFS depth from roots along an edge type | `edge`, `skip_barrel_files`, `threshold` |
| `find_cycles` | Tarjan SCC cycle detection | `edge` |
| `filter` | Structural node filter | `no_incoming`, `no_outgoing`, `has_edge`, `direction` |
| `match_name` | Name-based filter | `glob`, `regex`, `contains`, `starts_with`, `ends_with` |
| `ratio` | Compute export/total ratio per group | `numerator.where`, `denominator.where`, `threshold` |
| `pair` | Resource acquire/release pairing | `acquire_edge`, `release_edge` |
| `flag` | Emit a finding | `pattern`, `message`, `severity`, `severity_map`, `pipeline_name` |

### WhereClause Predicates (available on `select`, `ratio`, etc.)

| Predicate | What it checks |
|-----------|---------------|
| `exported` | Symbol is exported |
| `is_test_file` | File path matches test patterns |
| `is_generated` | File path matches generated patterns |
| `is_barrel_file` | File path matches barrel file patterns |
| `is_nolint` | Reserved; not yet evaluated at runtime |
| `count` | NumericPredicate on computed count metric |
| `cycle_size` | NumericPredicate on SCC size |
| `depth` | NumericPredicate on computed depth metric |
| `edge_count` | NumericPredicate on edge count |
| `ratio` | NumericPredicate on computed ratio |
| `and`, `or`, `not` | Logical composition of sub-clauses |

### NumericPredicate operators: `gte`, `lte`, `gt`, `lt`, `eq`

### Severity system

- `flag.severity` — static string fallback
- `flag.severity_map` — ordered list of `{when: WhereClause, severity: string}` entries; first match wins; entry with no `when` is a catch-all
- Template variables in messages: `{{name}}`, `{{kind}}`, `{{file}}`, `{{line}}`, `{{language}}`, `{{count}}`, `{{depth}}`, `{{cycle_size}}`, `{{cycle_path}}`, `{{edge_count}}`, `{{ratio}}`, any metric key

### Discovery and override

- Built-ins: embedded at compile time via `include_str!`
- Project-local: `.virgil/audits/*.json` (overrides built-ins by pipeline name)
- User-global: `~/.virgil-cli/audits/*.json`
- First-seen-wins deduplication (project-local beats global beats built-in)

**Confidence:** HIGH — directly read from source code.

---

## Industry Format Comparison

### Semgrep (YAML, pattern-matching based)

Semgrep rules are YAML files targeting single-file AST pattern matching. The rule unit is a matched code fragment, not a graph node.

**Core structure:**
```yaml
rules:
  - id: my-rule
    languages: [python]
    severity: HIGH
    pattern: hashlib.md5(...)
    message: "Use SHA-256 instead"
    paths:
      exclude: [tests/**]
```

**Compositional operators:** `patterns` (AND), `pattern-either` (OR), `pattern-not`, `pattern-inside`, `pattern-not-inside`, `focus-metavariable`

**Metavariable filters:** `metavariable-regex`, `metavariable-pattern`, `metavariable-comparison` (numeric: `$ARG < 1024`)

**Taint mode** (separate `mode: taint`):
```yaml
mode: taint
pattern-sources:
  - pattern: request.cookies[...]
pattern-sinks:
  - pattern: pickle.loads(...)
pattern-sanitizers:
  - pattern: sanitize($X)
```

**What Semgrep cannot do that virgil-cli's format can:**
- No aggregate/group-by counting across multiple files
- No graph traversal (cycle detection, depth measurement, call-graph BFS)
- No ratio computation (exports/total)
- Cannot express "files with > 30 symbols grouped by file" as a single rule
- Taint mode requires `mode: taint` — separate rule type, not composable with structural patterns

**What Semgrep can do that virgil-cli's format cannot:**
- Full intra-function AST pattern matching with metavariable capture (`$X`, `$...ARGS`)
- Code spans across multiple AST nodes (function body pattern matching)
- Auto-fix suggestions
- Numeric metavariable comparisons against literal values
- Cross-language metavariable-pattern (match JS embedded in Python)
- `interfile` cross-file analysis (Pro feature)

**Overall:** Semgrep is better for "find this code pattern in a function body." virgil-cli's graph pipeline is better for "aggregate metrics across files and flag anomalies."

**Confidence:** HIGH — verified against official Semgrep docs.

### CodeQL (QL language, Datalog-inspired)

CodeQL is a compiled query language (QL) compiled to a relational database backend. Rules are `.ql` files, not JSON/YAML.

**Structure:** Object-oriented predicate logic. Graph traversal via recursive predicates and `+`/`*` quantifiers. Taint tracking via `DataFlow::Configuration` class with abstract `isSource`/`isSink` methods.

**Key capabilities:**
- Recursive graph traversal (`exists(Node mid | edge(src, mid) and reachable(mid, dst))`)
- Path queries with `@kind path-problem` annotation
- Type-aware analysis (resolves method dispatch)
- Cross-file inter-procedural analysis as a first-class feature
- Aggregates: `count`, `sum`, `avg`, `max`, `min` over query results

**What QL can do that virgil-cli cannot:**
- Type resolution across files (knows `foo.bar()` calls `FooClass.bar`)
- True inter-procedural taint tracking with path reconstruction
- Recursive predicates (arbitrary depth graph queries without a depth cap)
- Aggregate functions inline within query logic

**What virgil-cli's format can do that QL cannot:**
- Single JSON file shipped inside a Rust binary — no compilation step
- Language-agnostic stages (same JSON pipeline runs on all 12 languages)
- User-overridable without a build step (drop JSON in `.virgil/audits/`)
- Name-based threshold tuning via `count`/`ratio` stages

**Overall:** QL is the gold standard for security analysis requiring type resolution. virgil-cli's format targets code quality metrics, not vulnerability hunting, and wins on portability and simplicity.

**Confidence:** HIGH — verified against CodeQL official docs.

### ast-grep (YAML, AST structural matching)

ast-grep uses YAML rules with structural AST operators. Its rule object is composable:

```yaml
id: no-console-log
language: JavaScript
severity: warning
rule:
  all:
    - pattern: console.log($$$ARGS)
    - not:
        inside:
          kind: if_statement
```

**Operators:** `all` (AND), `any` (OR), `not`, `inside`, `has`, `follows`, `precedes`, `kind`, `pattern`, `regex`

**Key capabilities:**
- Relational: `inside`, `has`, `follows`, `precedes` — structural containment queries
- `nthChild` (v0.23+) — positional child matching (e.g., match methods with exactly 3 params)
- `$$$` variadic metavariables for arbitrary argument lists
- Auto-fix via `fix` and `rewriters`
- Severity: `hint`, `info`, `warning`, `error`

**What ast-grep cannot do:**
- No aggregate/count across multiple files — single-file matching only
- No graph traversal or cycle detection
- No ratio/threshold computation
- No severity graduation based on a computed metric value

**What ast-grep does better than virgil-cli:**
- Richer single-file AST pattern composition (relational operators `inside`, `has`, `follows`)
- `nthChild` positional matching
- Variadic metavariables for argument lists
- Auto-fix generation

**Overall:** ast-grep is the closest format to what virgil-cli's JSON is trying to do, but still single-file. virgil-cli's graph pipeline is complementary — graph operations that ast-grep cannot express at all.

**Confidence:** MEDIUM-HIGH — verified against ast-grep docs and GitHub discussions.

### SonarQube (Java plugin + JSON metadata)

SonarQube rules are Java code (AST visitor pattern) with JSON metadata sidecar files:

```json
{
  "title": "Rule title",
  "type": "CODE_SMELL",
  "status": "ready",
  "defaultSeverity": "MAJOR",
  "tags": ["performance"]
}
```

Detection logic is in Java, not declarative JSON. The JSON is metadata only. Custom rules require a compiled Java plugin.

**Takeaway:** SonarQube's pattern is what virgil-cli was migrating away from — imperative code with declarative metadata sidecar. Not a model to follow.

**Confidence:** MEDIUM — from community docs and official SonarQube docs.

---

## Gap Analysis: Current JSON Format vs. Remaining Pipelines

The 310 Rust pipeline files across 12 languages fall into these analysis categories:

### Category 1: Graph-aggregate pipelines (well-served by current format)

These are the 4 architecture pipelines already migrated, plus the cross-file analyzers described in `audit_plans/cross_file_analyzers.md`. They use `select → group_by → count → flag` or `select → find_cycles → flag` patterns. Current format handles them fully.

**Examples already working:** `module_size_distribution`, `api_surface_area`, `circular_dependencies`, `dependency_graph_depth`

**Gap:** None for this category.

### Category 2: Per-function complexity metrics (partially served)

Pipelines like `cyclomatic_complexity`, `cognitive_complexity`, `function_length` count decision points or lines within individual functions. The current JSON format can select symbols and filter on `lines` ranges. What it cannot do:

- **Compute cyclomatic complexity from the AST**: The current `GraphStage` has no stage for "walk the function body's CFG and count branch edges." The existing Rust implementations use a `compute_cyclomatic()` helper that walks tree-sitter AST nodes with language-specific `decision_point_kinds` arrays. This is a computation that produces a number, not a graph traversal. The current format has no equivalent.
- **Compute cognitive complexity**: Similarly requires weighted nesting-level counting.

**Verdict:** These pipelines cannot be expressed in the current JSON format without a new stage type. Two options:
1. Add a `compute_metric` stage that calls a named Rust function (e.g., `"compute_metric": "cyclomatic_complexity"`) and stores the result as a named metric for `flag` to use.
2. Retain these as Rust pipelines and treat them as out of scope for JSON migration (they are `NodePipeline` types already, operating per-function with no graph).

**Recommendation:** Add `compute_metric: {metric: "cyclomatic_complexity" | "cognitive_complexity" | "function_length" | "comment_ratio"}` as a new stage that stores the computed value into the node's metrics map. The flag stage already has access to all metrics. This keeps the JSON format the rule authoring surface while Rust provides the computation primitives.

### Category 3: Simple pattern-match pipelines (NOT served by current format)

The majority of the remaining Rust pipelines are simple tree-sitter query pipelines: search for a specific AST pattern, emit a finding at each match. Examples:

- `panic_detection.rs` — find `.unwrap()`, `.expect()`, `panic!()` calls in Rust
- `dead_code.rs` — find unused variables (tree-sitter query for declarations never referenced)
- `command_injection.rs` — find `exec(user_input)` call patterns
- `n_plus_one_queries.rs` — find database calls inside loop bodies

These pipelines are not graph operations at all. They are `Pipeline` or `GraphPipeline` trait implementations that write a tree-sitter S-expression query string and iterate matches.

**The current JSON format has no `tree_sitter_query` or `pattern_match` stage.** There is no way to express "find all call expressions where the callee is `unwrap`" in the current `GraphStage` vocabulary without a graph edge traversal.

**This is the most important gap.** Most of the 306 remaining pipelines fall here.

**The path forward:** Add a `tree_sitter` stage (or `match_pattern` stage) to `GraphStage` that accepts a tree-sitter S-expression pattern string, optionally scoped to specific node types or parent contexts. This is a separate stage from graph traversal — it operates on raw AST.

Example target syntax:
```json
{
  "pipeline": "panic_detection",
  "category": "code-quality",
  "languages": ["rust"],
  "graph": [
    {"select": "file", "exclude": {"is_test_file": true}},
    {
      "match_pattern": {
        "query": "(call_expression function: (field_expression field: (field_identifier) @method) (#match? @method \"^(unwrap|expect)$\")) @call",
        "capture": "call",
        "name_capture": "method"
      }
    },
    {
      "flag": {
        "pattern": "panic_risk",
        "message": "Call to .{{name}}() may panic at runtime in {{file}}:{{line}}",
        "severity": "warning"
      }
    }
  ]
}
```

### Category 4: Duplicate code detection (not directly served)

`duplicate_code.rs` pipelines compute similarity between code blocks, often using token hashing or rolling hash algorithms. This is not expressible in any declarative format without either:
- A dedicated `detect_duplicates` stage
- Pre-computed duplicate annotations in the graph

**Recommendation:** Mark duplicate code detection as out of scope for JSON migration in this milestone. It is a specialized analysis that requires algorithmic primitives the current graph model does not have. Retain as Rust pipelines.

### Category 5: Language-specific security pipelines (partially served)

Security pipelines like `sql_injection`, `command_injection`, `path_traversal` follow the taint pattern: source (user input) flows to sink (dangerous function) without passing through a sanitizer. The CodeGraph already has `FlowsTo` and `SanitizedBy` edges (see `taint.rs`).

The current JSON format has a `traverse` stage with `flows_to` edge support. In principle:
```json
{"select": "symbol", "where": {"is_source": true}},
{"traverse": {"edge": "flows_to"}},
{"filter": {"no_sanitizer": true}},
{"flag": {...}}
```

However, `WhereClause` has no `is_source` or `is_sink` predicate, and `FilterConfig` has no `no_sanitizer` option. The taint graph is built but not queryable from JSON stages.

**Recommendation:** Add `is_taint_source` and `is_taint_sink` predicates to `WhereClause`, and add a `filter` option `no_sanitized_path: true` that checks for absence of `SanitizedBy` edges on the traversal path. This enables security pipelines in JSON.

---

## Recommended JSON Format Extensions

These are additions to the existing `GraphStage` enum and `WhereClause` in `src/graph/pipeline.rs`. All are additive and backward-compatible.

### Extension 1: `match_pattern` stage (HIGH priority)

Enables the ~200+ simple pattern-matching pipelines.

```rust
MatchPattern {
    match_pattern: MatchPatternConfig,
}

pub struct MatchPatternConfig {
    /// Tree-sitter S-expression query string
    pub query: String,
    /// Which capture name to use as the result node (default: first capture)
    pub capture: Option<String>,
    /// Which capture name to use as the symbol name (for {{name}} in messages)
    pub name_capture: Option<String>,
    /// Scope to specific language (overrides pipeline-level `languages` filter)
    pub language: Option<String>,
}
```

This stage parses the query string into a `tree_sitter::Query`, runs it against each file's AST (already parsed in `PipelineContext`), and emits one `PipelineNode` per match. Subsequent `flag` stages consume those nodes normally.

### Extension 2: `compute_metric` stage (MEDIUM priority)

Enables cyclomatic complexity, cognitive complexity, function length, comment ratio.

```rust
ComputeMetric {
    compute_metric: ComputeMetricConfig,
}

pub struct ComputeMetricConfig {
    /// Named metric to compute: "cyclomatic_complexity" | "cognitive_complexity" |
    /// "function_length" | "comment_ratio" | "line_count"
    pub metric: String,
    /// Threshold — nodes below threshold are dropped from the pipeline
    pub threshold: Option<NumericPredicate>,
}
```

The metric is stored in `node.metrics["<metric_name>"]` and is available in flag messages as `{{cyclomatic_complexity}}` etc.

### Extension 3: Taint predicates in `WhereClause` (MEDIUM priority)

```rust
pub struct WhereClause {
    // existing fields...
    #[serde(default)]
    pub is_taint_source: Option<bool>,
    #[serde(default)]
    pub is_taint_sink: Option<bool>,
    #[serde(default)]
    pub has_unsanitized_path: Option<bool>,
}
```

Enables security pipelines to express source-to-sink flows declaratively.

### Extension 4: `kind` predicate in `WhereClause` (LOW-MEDIUM priority)

Multiple pipelines need to filter by symbol kind (function vs. class vs. method). `WhereClause` currently has no `kind` predicate.

```rust
#[serde(default)]
pub kind: Option<Vec<String>>,   // ["function", "arrow_function", "method"]
```

### Extension 5: `name` predicate in `WhereClause` (LOW priority)

Some pipelines filter on symbol name patterns beyond what `match_name` stage provides. Adding `name` to `WhereClause` removes the need for a separate `match_name` stage in some pipelines.

```rust
#[serde(default)]
pub name: Option<MatchNameConfig>,
```

---

## Format Stability Assessment

The existing 4 JSON pipelines demonstrate that the current format is stable for graph-aggregate patterns. The format follows a consistent "linear pipeline" model — each stage transforms the node list, and the final `flag` stage emits findings. This is:

- **More readable** than CodeQL's predicate logic for non-experts
- **More powerful than Semgrep** for aggregate/cross-file analysis
- **More portable than SonarQube** (no Java compilation)
- **Comparable to ast-grep** for single-file patterns, with better cross-file support

The untagged serde enum approach for `GraphStage` works well for JSON authoring — each stage is visually identifiable by its unique top-level key. This should be preserved.

### Key decisions to preserve

1. The `graph` key as a linear stage array — not a DAG, not a nested tree. Linear pipelines are easier to author and debug.
2. `severity_map` with `when` clauses — this is virgil-cli's answer to CodeQL's aggregate threshold escalation and is better designed than Semgrep's flat severity fields.
3. The `exclude` clause on `select` — this is the correct place for test/generated file exclusion, matching how the 4 existing pipelines handle it.
4. `{{metric_name}}` interpolation — dynamic message templates are essential for useful finding messages.

---

## What to Not Implement

| Feature | Why to Skip |
|---------|-------------|
| Auto-fix suggestions | Out of scope — virgil-cli reports findings, does not rewrite code |
| Metavariable capture across patterns | Too complex; the `match_name` stage serves simpler name-matching needs |
| Recursive predicates (QL-style) | The `depth` parameter on `traverse` handles bounded recursion; unbounded recursion adds implementation complexity without clear use cases in the 306 remaining pipelines |
| YAML format | JSON is already implemented, serde_json is already a dependency; YAML would require adding serde_yaml and does not add expressive power |
| Rule templating/inheritance | Premature; no evidence that the 306 pipelines share enough structure to warrant inheritance |
| Cross-language taint propagation | Requires type resolution; out of scope given the name-based (not type-based) call graph |

---

## Rust Engine Requirements

To execute the extensions above, `src/audit/engine.rs` and the JSON pipeline executor need these changes:

1. **`match_pattern` execution path**: Load the `tree_sitter::Query` from the JSON stage's query string. Run against the `Tree` already in scope. Emit one `PipelineNode` per `QueryMatch` for the selected capture. Language must be known at this point (from `JsonAuditFile.languages` or workspace file extension).

2. **`compute_metric` execution path**: Call the appropriate Rust function (`compute_cyclomatic`, `compute_cognitive`, etc.) from `src/audit/pipelines/helpers.rs`. These functions already exist and take `(&Tree, &[u8], file_path)`. Store result in `PipelineNode.metrics`. Apply threshold filter.

3. **Taint predicate evaluation**: When `WhereClause.is_taint_source` or `is_taint_sink` is set, look up the node in `CodeGraph.taint_sources` / `CodeGraph.taint_sinks` (need to verify these index structures exist). `has_unsanitized_path` requires a path existence check on `FlowsTo`/`SanitizedBy` edges.

4. **`kind` predicate evaluation in `WhereClause.eval()`**: String comparison of `node.kind` against the `kind` list. Simple addition.

---

## Remaining Rust-Only Categories (Out of Scope for JSON Migration)

These categories should remain as Rust pipelines and be explicitly excluded from JSON migration scope:

| Category | Reason | Pipeline Count (estimate) |
|----------|--------|--------------------------|
| Duplicate code detection | Requires specialized similarity algorithm; no declarative equivalent | ~12 (1 per language) |
| Cognitive/cyclomatic complexity | Possible via `compute_metric` extension, but complex to add in one milestone | ~24 |
| Deep taint tracking (security) | Possible via taint predicates, but requires graph index verification | ~30 |

**Practical migration scope for this milestone:** The graph-aggregate, simple pattern-match, and function-length categories. Estimated: ~220 of 306 remaining pipelines are migratable with the `match_pattern` and basic `compute_metric` extensions.

---

## Sources

- Source code verified: `src/graph/pipeline.rs`, `src/audit/json_audit.rs`, `src/audit/builtin/*.json`, `src/audit/engine.rs`, `src/audit/pipeline.rs`, `src/audit/pipelines/typescript/cyclomatic.rs`, `src/audit/pipelines/rust/panic_detection.rs`
- Semgrep rule syntax: [https://semgrep.dev/docs/writing-rules/rule-syntax](https://semgrep.dev/docs/writing-rules/rule-syntax)
- Semgrep taint mode: [https://semgrep.dev/docs/writing-rules/data-flow/taint-mode/](https://semgrep.dev/docs/writing-rules/data-flow/taint-mode/)
- ast-grep YAML format: [https://ast-grep.github.io/reference/yaml.html](https://ast-grep.github.io/reference/yaml.html)
- ast-grep node counting discussion: [https://github.com/ast-grep/ast-grep/discussions/1201](https://github.com/ast-grep/ast-grep/discussions/1201)
- ast-grep YAML vs DSL analysis: [https://ast-grep.github.io/blog/yaml-vs-dsl.html](https://ast-grep.github.io/blog/yaml-vs-dsl.html)
- CodeQL query language: [https://codeql.github.com/docs/writing-codeql-queries/about-codeql-queries/](https://codeql.github.com/docs/writing-codeql-queries/about-codeql-queries/)
- CodeQL path queries: [https://codeql.github.com/docs/writing-codeql-queries/creating-path-queries/](https://codeql.github.com/docs/writing-codeql-queries/creating-path-queries/)
- SonarQube custom rules: [https://docs.sonarsource.com/sonarqube-server/2025.3/extension-guide/adding-coding-rules](https://docs.sonarsource.com/sonarqube-server/2025.3/extension-guide/adding-coding-rules)
- SonarQube Java custom rules 101: [https://github.com/SonarSource/sonar-java/blob/master/docs/CUSTOM_RULES_101.md](https://github.com/SonarSource/sonar-java/blob/master/docs/CUSTOM_RULES_101.md)
