# Phase 3: Tech Debt + Scalability JSON Migration - Pattern Map

**Mapped:** 2026-04-16
**Files analyzed:** 22 new/modified files
**Analogs found:** 22 / 22

---

## File Classification

| New/Modified File | Role | Data Flow | Closest Analog | Match Quality |
|-------------------|------|-----------|----------------|---------------|
| `src/audit/builtin/cyclomatic_complexity.json` | config (JSON pipeline) | batch/transform | `src/audit/builtin/module_size_distribution_rust.json` | role-match (same JSON schema; uses `compute_metric` stage not yet in existing JSON) |
| `src/audit/builtin/function_length.json` | config (JSON pipeline) | batch/transform | `src/audit/builtin/module_size_distribution_rust.json` | role-match |
| `src/audit/builtin/cognitive_complexity.json` | config (JSON pipeline) | batch/transform | `src/audit/builtin/module_size_distribution_rust.json` | role-match |
| `src/audit/builtin/comment_to_code_ratio.json` | config (JSON pipeline) | batch/transform | `src/audit/builtin/circular_dependencies_rust.json` (file-select pattern) | role-match |
| `src/audit/builtin/n_plus_one_queries.json` | config (JSON pipeline) | event-driven/pattern-match | `src/audit/builtin/module_size_distribution_rust.json` | partial-match (first JSON to use `match_pattern` stage) |
| `src/audit/builtin/sync_blocking_in_async_typescript.json` | config (JSON pipeline) | event-driven/pattern-match | `src/audit/builtin/n_plus_one_queries.json` (sibling) | sibling (same wave) |
| `src/audit/builtin/sync_blocking_in_async_rust.json` | config (JSON pipeline) | event-driven/pattern-match | `src/audit/builtin/n_plus_one_queries.json` (sibling) | sibling (same wave) |
| `src/audit/builtin/sync_blocking_in_async_python.json` | config (JSON pipeline) | event-driven/pattern-match | `src/audit/builtin/n_plus_one_queries.json` (sibling) | sibling (same wave) |
| `src/audit/builtin/sync_blocking_in_async_go.json` | config (JSON pipeline) | event-driven/pattern-match | `src/audit/builtin/n_plus_one_queries.json` (sibling) | sibling (same wave) |
| `src/graph/pipeline.rs` | service/model | transform | self (extension of existing WhereClause) | exact |
| `src/audit/pipelines/typescript/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/rust/mod.rs` | config/module | CRUD | self | exact |
| `src/audit/pipelines/python/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/go/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/java/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/javascript/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/c/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/cpp/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/csharp/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `src/audit/pipelines/php/mod.rs` | config/module | CRUD | `src/audit/pipelines/rust/mod.rs` | exact |
| `tests/audit_json_integration.rs` | test | request-response | self (extend existing file) | exact |
| Files to delete (up to 54 .rs files) | — (deletion) | — | see per-pipeline deletion pattern below | — |

---

## Pattern Assignments

### `src/graph/pipeline.rs` — WhereClause Extension (Rust code change, prerequisite)

**Analog:** `src/graph/pipeline.rs` lines 107-143 and 164-218

This is the ONLY Rust source file that must be modified. It is a prerequisite for all JSON pipelines to express threshold filtering via `severity_map.when`.

**Current WhereClause struct** (`src/graph/pipeline.rs` lines 107-143):
```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WhereClause {
    // Logical operators
    pub and: Option<Vec<WhereClause>>,
    pub or: Option<Vec<WhereClause>>,
    pub not: Option<Box<WhereClause>>,
    // Semantic built-ins
    pub is_test_file: Option<bool>,
    pub is_generated: Option<bool>,
    pub is_barrel_file: Option<bool>,
    pub is_nolint: Option<bool>,
    // Node property predicates
    pub exported: Option<bool>,
    // Metric predicates (for severity_map "when" clauses)
    pub count: Option<NumericPredicate>,
    pub cycle_size: Option<NumericPredicate>,
    pub depth: Option<NumericPredicate>,
    pub edge_count: Option<NumericPredicate>,
    pub ratio: Option<NumericPredicate>,
}
```

**Pattern to copy — add 4 new fields following the existing `ratio` field:**
```rust
    #[serde(default)]
    pub cyclomatic_complexity: Option<NumericPredicate>,
    #[serde(default)]
    pub function_length: Option<NumericPredicate>,
    #[serde(default)]
    pub cognitive_complexity: Option<NumericPredicate>,
    #[serde(default)]
    pub comment_to_code_ratio: Option<NumericPredicate>,
```

**Current `is_empty()` method** (lines 148-162): Add 4 more `&& self.<field>.is_none()` checks following the existing `ratio` check.

**Current `eval_metrics()` match arms** (lines 205-214): Add 4 more arms following the existing `ratio` arm:
```rust
        if let Some(ref pred) = self.cyclomatic_complexity {
            if !pred.matches(node.metric_f64("cyclomatic_complexity")) {
                return false;
            }
        }
        if let Some(ref pred) = self.function_length {
            if !pred.matches(node.metric_f64("function_length")) {
                return false;
            }
        }
        if let Some(ref pred) = self.cognitive_complexity {
            if !pred.matches(node.metric_f64("cognitive_complexity")) {
                return false;
            }
        }
        if let Some(ref pred) = self.comment_to_code_ratio {
            if !pred.matches(node.metric_f64("comment_to_code_ratio")) {
                return false;
            }
        }
```

**ALSO add to `eval()` method** (lines 220-301): Same 4 arms in the identical position following `ratio`. The `eval()` method duplicates the same metric predicate checks.

**Also needed:** `kind: Option<Vec<String>>` field on `WhereClause` for D-03's `select` kind filtering. Add after the `exported` field:
```rust
    #[serde(default)]
    pub kind: Option<Vec<String>>,
```
Add a matching check in `eval()` (not `eval_metrics()` — kind is a node property, not a metric):
```rust
        if let Some(ref kinds) = self.kind {
            if !kinds.iter().any(|k| k.eq_ignore_ascii_case(&node.kind)) {
                return false;
            }
        }
```
Also add `&& self.kind.is_none()` to `is_empty()`.

---

### `src/audit/builtin/cyclomatic_complexity.json` (JSON pipeline, cross-language, compute_metric)

**Primary analog:** `src/audit/builtin/module_size_distribution_rust.json` (lines 1-31) — shows the `select → aggregate → flag` shape with `severity_map`.

**Secondary analog:** `src/audit/pipelines/typescript/cyclomatic.rs` — canonical threshold (CC > 10 = warning), pattern name (`high_cyclomatic_complexity`), message format.

**JSON schema top-level pattern** (copy from `module_size_distribution_rust.json` lines 1-6):
```json
{
  "pipeline": "cyclomatic_complexity",
  "category": "code-quality",
  "description": "Detects functions with high cyclomatic complexity. CC = 1 + decision points + logical operators + ternaries. Threshold: warning ≥ 10, error ≥ 20.",
  "graph": [...]
}
```
Note: no `"languages"` field — this pipeline is cross-language (executor runs `control_flow_config_for_language` internally).

**Stage 1 — select with kind filter** (new DSL, enabled by WhereClause extension above):
```json
{
  "select": "symbol",
  "where": {"kind": ["function", "method", "arrow_function"]},
  "exclude": {"is_test_file": true}
}
```
Note: `kind` goes inside `"where"` (a `WhereClause` field), NOT as a top-level sibling of `"select"`.

**Stage 2 — compute_metric** (Phase 2 DSL, `src/graph/executor.rs` line 857-860):
```json
{"compute_metric": "cyclomatic_complexity"}
```

**Stage 3 — flag with severity_map** (copy severity_map pattern from `module_size_distribution_rust.json` lines 22-29, adapted for CC thresholds from `cyclomatic.rs` line 108):
```json
{
  "flag": {
    "pattern": "high_cyclomatic_complexity",
    "message": "Function `{{name}}` has cyclomatic complexity of {{cyclomatic_complexity}} (threshold: 10)",
    "severity_map": [
      {"when": {"cyclomatic_complexity": {"gte": 20}}, "severity": "error"},
      {"when": {"cyclomatic_complexity": {"gt": 10}}, "severity": "warning"}
    ]
  }
}
```
Note: no catch-all severity entry — only nodes exceeding threshold are flagged. The `flag` stage emits a finding for every node that reaches it, so the select/filter before `flag` determines what gets flagged. If `severity_map` has no matching `when` clause, the node is still flagged with the fallback `severity` field. To flag ONLY nodes above threshold, use a pre-flag `select` with `where: {cyclomatic_complexity: {gt: 10}}` OR rely on the `when` matching. **Recommended:** include a fallback `"severity": "info"` and let metric values determine severity — OR use a `where` filter stage before `flag`.

**Rust pipelines to delete for cyclomatic_complexity:**
- `src/audit/pipelines/typescript/cyclomatic.rs`
- `src/audit/pipelines/javascript/cyclomatic.rs`
- `src/audit/pipelines/rust/cyclomatic.rs`
- `src/audit/pipelines/python/cyclomatic.rs`
- `src/audit/pipelines/go/cyclomatic.rs`
- `src/audit/pipelines/java/cyclomatic.rs`
- `src/audit/pipelines/c/cyclomatic.rs`
- `src/audit/pipelines/cpp/cyclomatic.rs`
- `src/audit/pipelines/csharp/cyclomatic.rs`
- `src/audit/pipelines/php/cyclomatic.rs`

**mod.rs update pattern** (from `src/audit/pipelines/typescript/mod.rs` lines 15-17 and 58-64):
Remove `pub mod cyclomatic;` (line 17) and remove `Box::new(cyclomatic::CyclomaticComplexityPipeline::new(language)?)` from `complexity_pipelines()` (line 61). Apply identical removals to all 9 other language mod.rs files. For `rust/mod.rs`, `python/mod.rs`, `go/mod.rs`: function signature differs (`no language` arg), but the pattern is the same.

---

### `src/audit/builtin/function_length.json` (JSON pipeline, cross-language, compute_metric)

**Primary analog:** `src/audit/builtin/cyclomatic_complexity.json` (sibling, same wave) — identical structure.

**Secondary analog:** `src/audit/pipelines/typescript/function_length.rs` — thresholds: lines > 50 = `function_too_long`, statements > 20 = `too_many_statements` (lines 80 and 96).

**Key difference from CC:** The executor stores `function_length` as total line count (`lines` from `count_function_lines`, `src/graph/executor.rs` line 862). Statement count is NOT separately stored as a metric — the executor only stores one value per `compute_metric` call. The JSON pipeline can only express the line threshold (> 50), not the statement threshold (> 20). This is a known precision delta; document it in the description.

```json
{
  "pipeline": "function_length",
  "category": "code-quality",
  "description": "Detects functions that are too long (> 50 lines). Note: the Rust implementation also checks statement count (> 20 statements); that check is not expressible in the current JSON DSL and is dropped.",
  "graph": [
    {"select": "symbol", "where": {"kind": ["function", "method", "arrow_function"]}, "exclude": {"is_test_file": true}},
    {"compute_metric": "function_length"},
    {
      "flag": {
        "pattern": "function_too_long",
        "message": "Function `{{name}}` is {{function_length}} lines long (threshold: 50)",
        "severity_map": [
          {"when": {"function_length": {"gte": 100}}, "severity": "error"},
          {"when": {"function_length": {"gt": 50}}, "severity": "warning"}
        ]
      }
    }
  ]
}
```

**Rust pipelines to delete for function_length:**
Same 10 files as cyclomatic, s/cyclomatic/function_length/ in filenames. Remove `pub mod function_length;` and factory entry from each mod.rs.

---

### `src/audit/builtin/cognitive_complexity.json` (JSON pipeline, cross-language, compute_metric)

**Primary analog:** `src/audit/builtin/cyclomatic_complexity.json` (sibling) — identical structure.

**Secondary analog:** `src/audit/pipelines/typescript/cognitive.rs` line 108 — threshold: cognitive > 15.

```json
{
  "pipeline": "cognitive_complexity",
  "category": "code-quality",
  "description": "Detects functions with high cognitive complexity (> 15). Cognitive complexity penalizes nesting depth beyond cyclomatic complexity.",
  "graph": [
    {"select": "symbol", "where": {"kind": ["function", "method", "arrow_function"]}, "exclude": {"is_test_file": true}},
    {"compute_metric": "cognitive_complexity"},
    {
      "flag": {
        "pattern": "high_cognitive_complexity",
        "message": "Function `{{name}}` has cognitive complexity of {{cognitive_complexity}} (threshold: 15)",
        "severity_map": [
          {"when": {"cognitive_complexity": {"gte": 30}}, "severity": "error"},
          {"when": {"cognitive_complexity": {"gt": 15}}, "severity": "warning"}
        ]
      }
    }
  ]
}
```

**Rust pipelines to delete for cognitive_complexity:**
Same 10 files, s/cyclomatic/cognitive/. Remove `pub mod cognitive;` and factory entry from each mod.rs.

---

### `src/audit/builtin/comment_to_code_ratio.json` (JSON pipeline, file-level, compute_metric)

**Primary analog for structure:** `src/audit/builtin/circular_dependencies_rust.json` — shows `select: "file"` pattern (lines 8-15). This is the only existing JSON with file-level select.

**Secondary analog for logic:** `src/audit/pipelines/typescript/comment_ratio.rs` — thresholds: ratio < 0.05 → `under_documented`, ratio > 0.60 → `over_documented` (lines 74 and 88). In the JSON DSL, ratios are stored as integer percentages (0-100) by the executor (`src/graph/executor.rs` lines 830-836), so `< 0.05` becomes `{"lt": 5}` and `> 0.60` becomes `{"gt": 60}`.

**Critical difference from other 3 complexity pipelines:** Uses `select: "file"` NOT `select: "symbol"`. The executor's `comment_to_code_ratio` special-case (`executor.rs` lines 824-839) applies the metric to the whole file tree and stores it on the node as integer percentage.

**Two-pattern approach:** The Rust version emits two distinct patterns (`under_documented` and `over_documented`). In JSON, this can be expressed as two separate `flag` stages OR two entries in `severity_map` with different `pattern` values. Since `FlagConfig.pattern` is a single string (not per-severity), use two separate JSON pipelines or a single pipeline that flags ALL files and uses message interpolation to communicate the direction.

**Simplest correct approach — single pipeline, severity_map only distinguishes severity not pattern:**
```json
{
  "pipeline": "comment_to_code_ratio",
  "category": "code-quality",
  "description": "Detects files with too few (< 5%) or too many (> 60%) comments relative to code. Value stored as integer percentage (0-100). Under-documented = ratio < 5; over-documented = ratio > 60.",
  "graph": [
    {
      "select": "file",
      "exclude": {"is_test_file": true}
    },
    {"compute_metric": "comment_to_code_ratio"},
    {
      "flag": {
        "pattern": "under_documented",
        "message": "File has comment ratio of {{comment_to_code_ratio}}% (threshold: 5%-60%)",
        "severity_map": [
          {"when": {"comment_to_code_ratio": {"lt": 5}}, "severity": "warning"},
          {"when": {"comment_to_code_ratio": {"gt": 60}}, "severity": "warning"}
        ]
      }
    }
  ]
}
```
Note: The `severity_map` `when` clauses use the new `comment_to_code_ratio` WhereClause field from the prerequisite extension. The fallback (no matching `when`) is silent — files in the 5-60% range produce no finding because no `when` clause matches and no default `severity` is set. This mirrors the Rust behavior.

**Rust pipelines to delete for comment_to_code_ratio:**
Same 10 files, s/cyclomatic/comment_ratio/. Remove `pub mod comment_ratio;` and factory entry from each mod.rs.

---

### `src/audit/builtin/n_plus_one_queries.json` (JSON pipeline, cross-language, match_pattern)

**Primary analog for structure:** `src/audit/builtin/module_size_distribution_rust.json` — overall JSON skeleton.

**Primary analog for logic:** `src/audit/pipelines/typescript/n_plus_one_queries.rs` (the most complete reference) — shows the structural pattern being approximated.
- TypeScript S-expression for method calls (lines 72-77): `(call_expression function: (member_expression ...) arguments: ...)` captures all method calls
- TypeScript loop kinds (lines 56-61): `for_statement`, `for_in_statement`, `while_statement`, `do_statement`
- Dropped: `DB_METHOD_NAMES`, `DB_OBJ_METHOD_PAIRS`, `ARRAY_LOOP_METHODS`, `BARE_CALL_PATTERNS` — name-based filtering not expressible without `#match?` predicates

**match_pattern stage DSL** (`src/graph/executor.rs` lines 714-783): Accepts a raw tree-sitter S-expression string. Compiles per language (silently skips languages whose grammar rejects the query). Emits one `PipelineNode` per AST capture per match.

**Cross-language S-expression strategy:** Write a single query that works in TypeScript, JavaScript, Python, Rust, Go, Java. The broadest cross-language pattern for "call inside for loop" in TypeScript/JavaScript grammar:
```
(for_statement
  body: (_) @call_site)
```
This is too broad (captures any statement). The target is call_expression inside loop body. A multi-alternation query using `[...]` can handle multiple loop kinds:

**Recommended match_pattern query:**
```
[
  (for_statement body: (_
    (expression_statement
      (call_expression) @call)))
  (for_in_statement body: (_
    (expression_statement
      (call_expression) @call)))
  (while_statement body: (_
    (expression_statement
      (call_expression) @call)))
  (do_statement body: (_
    (expression_statement
      (call_expression) @call)))
]
```
Note: This will only compile successfully in languages whose grammar defines these exact node kinds (TypeScript, JavaScript, Java). Rust, Python, and Go have different loop node kind names. Per D-05/D-06, the executor silently skips files in languages whose grammar rejects the query — so a TypeScript-grammar query safely skips Rust/Python/Go files.

```json
{
  "pipeline": "n_plus_one_queries",
  "category": "scalability",
  "description": "Detects call expressions inside loop bodies (potential N+1 query pattern). This JSON version uses structural pattern matching only — it does NOT filter by DB/ORM/HTTP method names (findOne, axios.get, etc.) as the Rust version did. Expect higher false positive rate. Dropped patterns: findOne, findUnique, findMany, find, findById, query, execute, axios.get/post/put/delete/patch, http.get/post/request, fetch, request, Model.find, db.collection.",
  "graph": [
    {
      "match_pattern": "[ (for_statement body: (_ (expression_statement (call_expression) @call))) (for_in_statement body: (_ (expression_statement (call_expression) @call))) (while_statement body: (_ (expression_statement (call_expression) @call))) (do_statement body: (_ (expression_statement (call_expression) @call))) ]"
    },
    {
      "flag": {
        "pattern": "query_in_loop",
        "message": "Call expression inside loop body — potential N+1 query pattern",
        "severity": "warning"
      }
    }
  ]
}
```

**Rust pipelines to delete for n_plus_one_queries (all 10 languages):**
- `src/audit/pipelines/typescript/n_plus_one_queries.rs`
- `src/audit/pipelines/javascript/n_plus_one_queries.rs`
- `src/audit/pipelines/rust/n_plus_one_queries.rs`
- `src/audit/pipelines/python/n_plus_one_queries.rs`
- `src/audit/pipelines/go/n_plus_one_queries.rs`
- `src/audit/pipelines/java/n_plus_one_queries.rs`
- `src/audit/pipelines/c/n_plus_one_queries.rs`
- `src/audit/pipelines/cpp/n_plus_one_queries.rs`
- `src/audit/pipelines/csharp/n_plus_one_queries.rs` (if exists)
- `src/audit/pipelines/php/n_plus_one_queries.rs`

Remove `pub mod n_plus_one_queries;` and `scalability_pipelines()` factory entry from each mod.rs.

---

### `src/audit/builtin/sync_blocking_in_async_typescript.json` (JSON pipeline, TypeScript/JavaScript, match_pattern)

**Primary analog:** `src/audit/pipelines/typescript/sync_blocking_in_async.rs` — canonical patterns.

Key patterns from Rust version:
- `SYNC_SUFFIX_METHODS` (lines 14-33): methods ending in "Sync" — `readFileSync`, `writeFileSync`, etc.
- `BLOCKING_OBJ_METHOD_PAIRS` (lines 36-62): `fs.readFileSync`, `child_process.execSync`, etc.
- Detection: walk parent chain to find `async function_declaration`/`arrow_function`/`function_expression`/`method_definition`
- Pattern name: `sync_call_in_async`

The match_pattern S-expression cannot replicate "parent chain has async keyword" without `#match?` predicate. The simplified approach matches Sync-suffix method properties inside any call (no async containment check):

```json
{
  "pipeline": "sync_blocking_in_async",
  "category": "scalability",
  "description": "Detects synchronous blocking calls (*Sync functions, fs.readFileSync, etc.) in TypeScript/JavaScript. Note: this JSON version cannot verify the call is inside an async function (requires text predicate #match?). All *Sync calls are flagged regardless of async context. Dropped: async-function containment check.",
  "languages": ["typescript", "javascript"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (member_expression property: (property_identifier) @method) @call)"
    },
    {
      "flag": {
        "pattern": "sync_call_in_async",
        "message": "Synchronous call `{{name}}` detected — use async equivalent to avoid blocking the event loop",
        "severity": "warning"
      }
    }
  ]
}
```
Note on `#match?` predicate feasibility: The RESEARCH.md (Open Question 3, assumption A1) indicates `#eq?` predicates work in `execute_match_pattern`. If `#match?` also works (tree-sitter supports it at query compilation level), add `(#match? @method "Sync$")` to narrow to Sync-suffix methods only. If not, all method calls are emitted (very high false positive rate). **Planner should test #match? support and use it if available.**

Improved query if `#match?` works:
```
(call_expression
  function: (member_expression
    property: (property_identifier) @method
    (#match? @method "Sync$"))) @call
```

**Rust pipelines to delete for sync_blocking_in_async TypeScript:**
- `src/audit/pipelines/typescript/sync_blocking_in_async.rs`
- `src/audit/pipelines/javascript/sync_blocking_in_async.rs`
Remove `pub mod sync_blocking_in_async;` and factory entries from both mod.rs files.

---

### `src/audit/builtin/sync_blocking_in_async_rust.json` (JSON pipeline, Rust, match_pattern)

**Primary analog:** `src/audit/pipelines/rust/sync_blocking_in_async.rs` — canonical patterns.

Key patterns from Rust version (`sync_blocking_in_async.rs` lines 13-26):
- `BLOCKING_SCOPED_PREFIXES`: `std::fs::read`, `std::thread::sleep`, `std::net::`, etc.
- Two queries: `(call_expression function: (scoped_identifier) @scoped_fn)` and method call query
- Async detection: checks function_item source text starts with "async"

The match_pattern for scoped_identifier calls (targeting `std::fs` / `std::thread::sleep`):
```
(call_expression
  function: (scoped_identifier) @fn) @call
```
This emits ALL scoped calls. Without `#match?` for path prefix filtering, the false positive rate will be very high. Accept per D-07/D-08.

```json
{
  "pipeline": "sync_blocking_in_async",
  "category": "scalability",
  "description": "Detects scoped calls (std::fs::*, std::thread::sleep, std::net::*) inside async fn bodies in Rust. JSON version cannot filter by path prefix (no #match? for scoped_identifier text) and cannot verify async containment. All scoped calls flagged. Dropped: BLOCKING_SCOPED_PREFIXES filtering, async fn containment check.",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (scoped_identifier) @fn)"
    },
    {
      "flag": {
        "pattern": "blocking_io_in_async",
        "message": "Scoped call `{{name}}` detected — verify this is not a blocking I/O call inside an async fn",
        "severity": "info"
      }
    }
  ]
}
```
Note: Severity downgraded to `"info"` given the high false positive rate (all scoped calls, not just blocking ones).

**Rust pipeline to delete:**
- `src/audit/pipelines/rust/sync_blocking_in_async.rs`
Remove `pub mod sync_blocking_in_async;` and scalability_pipelines() entry from `rust/mod.rs`.

---

### `src/audit/builtin/sync_blocking_in_async_python.json` (JSON pipeline, Python, match_pattern)

**Primary analog:** `src/audit/pipelines/python/sync_blocking_in_async.rs` — canonical patterns.

Key patterns (lines 12-32):
- `BLOCKING_ATTR_CALLS`: `time.sleep`, `requests.get`, `subprocess.run`, `socket.connect`, etc.
- `BLOCKING_BARE_CALLS`: `open`, `input`, `sleep`
- Uses `GraphPipeline` (not `Pipeline`) — important for deletion (see Pitfall 5 in RESEARCH.md)

Python tree-sitter call expression:
```
(call function: (attribute
  object: (identifier) @obj
  attribute: (identifier) @method)) @call
```

```json
{
  "pipeline": "sync_blocking_in_async",
  "category": "scalability",
  "description": "Detects blocking calls (time.sleep, requests.get, subprocess.run, open, etc.) in Python async def functions. JSON version cannot verify async containment — all attribute calls flagged. Dropped: BLOCKING_ATTR_CALLS filtering, async def containment check.",
  "languages": ["python"],
  "graph": [
    {
      "match_pattern": "(call function: (attribute object: (identifier) @obj attribute: (identifier) @method)) @call"
    },
    {
      "flag": {
        "pattern": "blocking_in_async_def",
        "message": "Call `{{name}}` detected — verify this is not a blocking call inside an async def",
        "severity": "info"
      }
    }
  ]
}
```

**CRITICAL — Python uses GraphPipeline, not Pipeline** (`python/mod.rs` line 91):
```rust
AnyPipeline::Graph(Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new()?))
```
The deletion from `scalability_pipelines()` in `python/mod.rs` must remove this `AnyPipeline::Graph(...)` entry (not `Box::new` as a bare Pipeline). The factory function signature is `-> Result<Vec<AnyPipeline>>` (different from TypeScript/Go which return `-> Result<Vec<Box<dyn Pipeline>>>`).

**Rust pipeline to delete:**
- `src/audit/pipelines/python/sync_blocking_in_async.rs`
Remove `pub mod sync_blocking_in_async;` and `scalability_pipelines()` entry from `python/mod.rs`.

---

### `src/audit/builtin/sync_blocking_in_async_go.json` (JSON pipeline, Go, match_pattern)

**Primary analog:** `src/audit/pipelines/go/sync_blocking_in_async.rs` — canonical patterns.

Key patterns (lines 14-28): `time.Sleep`, `http.Get`, `net.Dial`, `os.Open`, `ioutil.ReadAll`, etc. inside goroutine bodies (`go_statement > func_literal > body`).

Go tree-sitter grammar: `go_statement` contains a `call_expression` whose `function` is a `func_literal`. Inside the func_literal's `body` are the blocking calls.

```
(go_statement
  (call_expression
    function: (func_literal
      body: (block
        (expression_statement
          (call_expression) @call)))))
```

```json
{
  "pipeline": "sync_blocking_in_async",
  "category": "scalability",
  "description": "Detects blocking calls inside goroutine bodies in Go (time.Sleep, http.Get, net.Dial, os.Open, etc.). JSON version cannot filter by package.Method name (no #match? for selector_expression text). All call expressions inside goroutines flagged. Dropped: BLOCKING_CALLS pkg/method filtering.",
  "languages": ["go"],
  "graph": [
    {
      "match_pattern": "(go_statement (call_expression function: (func_literal body: (block (expression_statement (call_expression) @call)))))"
    },
    {
      "flag": {
        "pattern": "blocking_in_goroutine",
        "message": "Call expression inside goroutine body — verify this is not a blocking I/O call",
        "severity": "info"
      }
    }
  ]
}
```

**Rust pipeline to delete:**
- `src/audit/pipelines/go/sync_blocking_in_async.rs`
Remove `pub mod sync_blocking_in_async;` and `scalability_pipelines()` entry from `go/mod.rs`.

---

### `tests/audit_json_integration.rs` — New Test Functions (extend existing file)

**Analog:** `tests/audit_json_integration.rs` lines 24-47 (module_size_distribution_rust test pair). Copy this exact structure for each of the 6 new pipelines.

**Test structure pattern** (lines 24-47, verbatim template):
```rust
#[test]
fn <pipeline>_<lang>_<positive_case>() {
    let dir = tempfile::tempdir().unwrap();
    // <describe fixture: what makes this trigger>
    let content = r#"<source code snippet>"#;
    std::fs::write(dir.path().join("<file>.<ext>"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::<Lang>], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::<Lang>]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::<Lang>])
        .pipeline_selector(PipelineSelector::<Category>)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "<pipeline_name>" && f.pattern == "<pattern>"),
        "expected <pattern> finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

**Imports** (lines 15-20, already present — do not duplicate):
```rust
use virgil_cli::{
    audit::engine::{AuditEngine, PipelineSelector},
    graph::builder::GraphBuilder,
    language::Language,
    workspace::Workspace,
};
```

**Per-pipeline fixture design:**

1. `cyclomatic_complexity` (TypeScript, `PipelineSelector::Complexity`):
   - Positive: function with 11 `if` statements (CC = 12 > 10). From `cyclomatic.rs` test lines 147-167.
   - Negative: function with 1 `if` (CC = 2).
   - Pattern to assert: `f.pipeline == "cyclomatic_complexity" && f.pattern == "high_cyclomatic_complexity"`

2. `function_length` (TypeScript, `PipelineSelector::Complexity`):
   - Positive: function with 52 `let x = y;` statements (> 50 lines). From `function_length.rs` test lines 133-147.
   - Negative: 3-line function.
   - Pattern: `f.pipeline == "function_length" && f.pattern == "function_too_long"`

3. `cognitive_complexity` (TypeScript, `PipelineSelector::Complexity`):
   - Positive: deeply nested if/for/while/while/if/if (cognitive > 15). From `cognitive.rs` test lines 146-163.
   - Negative: single if function.
   - Pattern: `f.pipeline == "cognitive_complexity" && f.pattern == "high_cognitive_complexity"`

4. `comment_to_code_ratio` (TypeScript, `PipelineSelector::Complexity`):
   - Positive: 30 lines of `const x = y;` with no comments (ratio 0% < 5%). From `comment_ratio.rs` test lines 122-134.
   - Negative: 5 comment lines + 5 code lines (ratio 50%, within 5-60%). From `comment_ratio.rs` test lines 136-154.
   - Pattern: `f.pipeline == "comment_to_code_ratio" && f.pattern == "under_documented"`

5. `n_plus_one_queries` (TypeScript, `PipelineSelector::Scalability`):
   - Positive: `for (let i = 0; ...) { db.findOne(...); }` — call inside for loop.
   - Negative: `const user = db.findOne(1);` outside any loop.
   - Pattern: `f.pipeline == "n_plus_one_queries" && f.pattern == "query_in_loop"`

6. `sync_blocking_in_async` (TypeScript, `PipelineSelector::Scalability`):
   - Positive: `async function load() { fs.readFileSync('x'); }` — Sync call present.
   - Negative: `async function load() { await fs.promises.readFile('x'); }` — no Sync call.
   - Pattern: `f.pipeline == "sync_blocking_in_async" && f.pattern == "sync_call_in_async"`
   - Note: if `#match?` is NOT supported and query is broadened, the negative test may need adjustment.

---

## Shared Patterns

### JSON Pipeline Top-Level Skeleton
**Source:** Any file in `src/audit/builtin/*.json`
**Apply to:** All 9 new JSON files in `src/audit/builtin/`
```json
{
  "pipeline": "<name matching Rust Pipeline::name()>",
  "category": "<code-quality|scalability>",
  "description": "<human description>",
  "languages": ["<optional — omit for cross-language>"],
  "graph": [ ...stages... ]
}
```
Rule: `"pipeline"` value must exactly match the Rust pipeline's `fn name()` return value. This is the suppression key used in `src/audit/engine.rs` line 111.

### severity_map Pattern
**Source:** `src/audit/builtin/module_size_distribution_rust.json` lines 22-29
**Apply to:** All compute_metric JSON pipelines (cyclomatic_complexity, function_length, cognitive_complexity)
```json
"severity_map": [
  {"when": {"<metric_name>": {"gte": <high_threshold>}}, "severity": "error"},
  {"when": {"<metric_name>": {"gt": <low_threshold>}}, "severity": "warning"}
]
```
Note: Requires WhereClause extension (see pipeline.rs pattern assignment above).

### select: "file" Pattern
**Source:** `src/audit/builtin/circular_dependencies_rust.json` lines 8-15
**Apply to:** `comment_to_code_ratio.json` only
```json
{
  "select": "file",
  "exclude": {"is_test_file": true}
}
```

### select: "symbol" with kind filter
**Source:** New DSL enabled by WhereClause extension (no existing analog — this is a new capability)
**Apply to:** `cyclomatic_complexity.json`, `function_length.json`, `cognitive_complexity.json`
```json
{
  "select": "symbol",
  "kind": ["function", "method", "arrow_function"],
  "exclude": {"is_test_file": true}
}
```

### match_pattern Stage
**Source:** `src/graph/executor.rs` lines 714-783 (implementation), `src/audit/pipelines/rust/n_plus_one_queries.rs` lines 43-49 (S-expression template source)
**Apply to:** `n_plus_one_queries.json`, all `sync_blocking_in_async_*.json`
```json
{"match_pattern": "<single-line tree-sitter S-expression string>"}
```
Rules:
- Entire query is a single JSON string value (newlines escaped or removed)
- Uses `[...]` for alternations across multiple loop/construct kinds
- Each capture `@name` emits one `PipelineNode` — use exactly ONE capture per query pattern to avoid duplicate findings
- Query compilation is attempted per-language; silently skipped if grammar rejects it

### mod.rs Deletion Pattern
**Source:** `src/audit/pipelines/typescript/mod.rs` lines 15-64, `src/audit/pipelines/rust/mod.rs` lines 14-91
**Apply to:** All 10 language mod.rs files for each pipeline deletion
Two-step per pipeline name:
1. Remove `pub mod <pipeline_module_name>;` from the module declarations
2. Remove `Box::new(<TypeName>::new(...)?)`  or `AnyPipeline::<Variant>(Box::new(...))` from the factory function

Python special case — `complexity_pipelines()` returns `Vec<AnyPipeline>` and wraps with `AnyPipeline::Node(...)`:
```rust
// python/mod.rs complexity_pipelines():
AnyPipeline::Node(Box::new(cyclomatic::CyclomaticComplexityPipeline::new()?))
// Remove this entry, not Box::new(...)? directly
```
Rust special case — factory functions take no language argument (`new()` not `new(language)`).

### Integration Test Pattern
**Source:** `tests/audit_json_integration.rs` lines 24-47
**Apply to:** All 6 × 2 = 12 new test functions in `tests/audit_json_integration.rs`
```rust
#[test]
fn <pipeline>_<lang>_<scenario>() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("<file>"), <content>).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::<Lang>], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::<Lang>]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::<Lang>])
        .pipeline_selector(PipelineSelector::<Category>)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(findings.iter().any(|f| f.pipeline == "<name>" && f.pattern == "<pattern>"), ...);
}
```

---

## Deletion Summary — All Rust Pipeline Files to Remove

The following Rust pipeline files are deleted when their JSON replacement is verified:

**4 complexity pipelines × 10 language directories = 40 files:**
```
src/audit/pipelines/{typescript,javascript,rust,python,go,java,c,cpp,csharp,php}/cyclomatic.rs
src/audit/pipelines/{typescript,javascript,rust,python,go,java,c,cpp,csharp,php}/function_length.rs
src/audit/pipelines/{typescript,javascript,rust,python,go,java,c,cpp,csharp,php}/cognitive.rs
src/audit/pipelines/{typescript,javascript,rust,python,go,java,c,cpp,csharp,php}/comment_ratio.rs
```

**n_plus_one_queries × 10 = 10 files** (check csharp — may be absent):
```
src/audit/pipelines/{typescript,javascript,rust,python,go,java,c,cpp,php}/n_plus_one_queries.rs
```

**sync_blocking_in_async for the 4 targeted languages = 4 files:**
```
src/audit/pipelines/typescript/sync_blocking_in_async.rs
src/audit/pipelines/javascript/sync_blocking_in_async.rs   (if JavaScript uses same pipeline)
src/audit/pipelines/python/sync_blocking_in_async.rs
src/audit/pipelines/rust/sync_blocking_in_async.rs
src/audit/pipelines/go/sync_blocking_in_async.rs
```
Note: C, C++, C#, Java, PHP sync_blocking_in_async Rust files remain (not targeted by Phase 3).

---

## No Analog Found

All files have adequate analogs. The only "no analog" situation is:

| File | Role | Data Flow | Reason |
|------|------|-----------|--------|
| `src/audit/builtin/*.json` using `compute_metric` | config | batch/transform | No existing JSON file uses `compute_metric` or `match_pattern` stage — these are Phase 2 additions. The executor implementation is the reference, not an existing JSON file. |
| `WhereClause.kind` filter field | model | — | No existing JSON uses `kind` in a select stage — new DSL capability. |

---

## Metadata

**Analog search scope:** `src/audit/builtin/`, `src/audit/pipelines/`, `src/graph/pipeline.rs`, `src/graph/executor.rs`, `src/graph/metrics.rs`, `tests/audit_json_integration.rs`
**Files scanned:** ~30
**Pattern extraction date:** 2026-04-16
**Confidence:** HIGH — all key source files read directly; executor implementation verified at line level
