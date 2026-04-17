# virgil-cli Simplification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the JSON audit DSL to cover all Rust-only capabilities, then delete the Rust pipelines, analyzers, and nested CLI that are no longer needed.

**Architecture:** JSON-first, delete-after — DSL extensions land first and are validated against existing tests before any Rust is removed. Phase A adds `taint`, `find_duplicates`, and coupling metrics to the DSL and writes 21 JSON builtin files. Phase B deletes the 18+ Rust files those JSON files replace. Phase C flattens the CLI.

**Tech Stack:** Rust / clap / serde_json / petgraph / rayon / tree-sitter

---

## File Map

### Phase A — New / Modified
| Action | Path | Purpose |
|---|---|---|
| Modify | `src/graph/pipeline.rs` | Add `TaintStage`, `FindDuplicatesStage`, new `WhereClause` fields, new `ComputeMetric` variants |
| Modify | `src/graph/executor.rs` | Add `execute_taint`, `execute_find_duplicates`, coupling metric, unreferenced/is_entry_point predicates |
| Modify | `src/graph/taint.rs` | Remove const arrays; parameterise engine with `TaintConfig`; remove mut graph requirement |
| Create | `src/audit/builtin/sql_injection_{python,go,java,javascript,typescript,php,csharp,cpp}.json` | 8 JSON security pipelines |
| Create | `src/audit/builtin/ssrf_{python,javascript,php,go,java,csharp}.json` | 6 JSON security pipelines |
| Create | `src/audit/builtin/xss_{javascript,typescript}.json` | 2 JSON security pipelines |
| Create | `src/audit/builtin/xxe_{python,java,csharp}.json` | 3 JSON security pipelines |
| Create | `src/audit/builtin/coupling.json` | Replaces `coupling.rs` |
| Create | `src/audit/builtin/dead_exports.json` | Replaces `dead_exports.rs` |
| Create | `src/audit/builtin/duplicate_symbols.json` | Replaces `duplicate_symbols.rs` |

### Phase B — Deleted
```
src/audit/pipelines/python/sql_injection.rs
src/audit/pipelines/python/ssrf.rs
src/audit/pipelines/javascript/xss_dom_injection.rs
src/audit/pipelines/javascript/ssrf.rs
src/audit/pipelines/typescript/   (entire dir)
src/audit/pipelines/go/sql_injection.rs
src/audit/pipelines/go/ssrf_open_redirect.rs
src/audit/pipelines/java/java_ssrf.rs
src/audit/pipelines/java/sql_injection.rs
src/audit/pipelines/java/xxe.rs
src/audit/pipelines/php/sql_injection.rs
src/audit/pipelines/php/ssrf.rs
src/audit/pipelines/csharp/csharp_ssrf.rs
src/audit/pipelines/csharp/sql_injection.rs
src/audit/pipelines/csharp/xxe.rs
src/audit/analyzers/coupling.rs
src/audit/analyzers/dead_exports.rs
src/audit/analyzers/duplicate_symbols.rs
src/graph/taint.rs
audit_plans/   (entire directory)
docs/superpowers/plans/2026-04-05-rust-tech-debt-pipeline-improvements.md
docs/superpowers/plans/2026-04-06-cpp-architecture-pipeline-fixes.md
```

### Phase C — Modified
| Action | Path | Purpose |
|---|---|---|
| Rewrite | `src/cli.rs` | Remove nested audit subcommands; add flat `AuditArgs` |
| Rewrite | `src/main.rs` | Remove 6-arm audit dispatch; single `Commands::Audit` arm |
| Modify | `src/audit/engine.rs` | Remove `PipelineSelector`; add `category_filter`; remove `ProjectAnalyzer` dispatch |

---

## Phase A — DSL Extensions

### Task 1: Add `TaintStage` structs to `pipeline.rs`

**Files:**
- Modify: `src/graph/pipeline.rs`

- [ ] **Step 1: Add taint config structs** after the `RatioConfig` struct (around line 476):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSourcePattern {
    pub pattern: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSinkPattern {
    pub pattern: String,
    pub vulnerability: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSanitizerPattern {
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintStage {
    pub sources: Vec<TaintSourcePattern>,
    pub sinks: Vec<TaintSinkPattern>,
    pub sanitizers: Vec<TaintSanitizerPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindDuplicatesStage {
    pub by: String,
    #[serde(default = "default_min_count")]
    pub min_count: usize,
}

fn default_min_count() -> usize { 2 }
```

- [ ] **Step 2: Add new `GraphStage` variants** — in the `GraphStage` enum (around line 525), add before the closing brace:

```rust
    Taint {
        taint: TaintStage,
    },
    FindDuplicates {
        find_duplicates: FindDuplicatesStage,
    },
```

- [ ] **Step 3: Add new `WhereClause` fields** — in the `WhereClause` struct (around line 108), add after `comment_to_code_ratio`:

```rust
    // Coupling predicates (populated after compute_metric: efferent/afferent_coupling)
    #[serde(default)]
    pub efferent_coupling: Option<NumericPredicate>,
    #[serde(default)]
    pub afferent_coupling: Option<NumericPredicate>,

    // Dead-export predicates
    #[serde(default)]
    pub unreferenced: Option<bool>,
    #[serde(default)]
    pub is_entry_point: Option<bool>,
```

- [ ] **Step 4: Update `WhereClause::is_empty`** — add the four new fields to the `&&` chain in `is_empty()`:

```rust
    && self.efferent_coupling.is_none()
    && self.afferent_coupling.is_none()
    && self.unreferenced.is_none()
    && self.is_entry_point.is_none()
```

- [ ] **Step 5: Update `WhereClause::eval_metrics`** — add after the `comment_to_code_ratio` block:

```rust
    if let Some(ref pred) = self.efferent_coupling {
        if !pred.matches(node.metric_f64("efferent_coupling")) {
            return false;
        }
    }
    if let Some(ref pred) = self.afferent_coupling {
        if !pred.matches(node.metric_f64("afferent_coupling")) {
            return false;
        }
    }
    // unreferenced and is_entry_point require graph access; skip in metrics-only eval
```

- [ ] **Step 6: Update `WhereClause::eval`** — add after the `comment_to_code_ratio` block (before `true`):

```rust
    if let Some(ref pred) = self.efferent_coupling {
        if !pred.matches(node.metric_f64("efferent_coupling")) {
            return false;
        }
    }
    if let Some(ref pred) = self.afferent_coupling {
        if !pred.matches(node.metric_f64("afferent_coupling")) {
            return false;
        }
    }
    if let Some(exp) = self.unreferenced {
        // unreferenced is set as a metric by the executor; read from metrics map
        let val = node.metric_f64("unreferenced") > 0.0;
        if val != exp {
            return false;
        }
    }
    if let Some(exp) = self.is_entry_point {
        let val = node.metric_f64("is_entry_point") > 0.0;
        if val != exp {
            return false;
        }
    }
```

- [ ] **Step 7: Build to check for compile errors**

```bash
cargo build 2>&1 | head -40
```

Expected: compile errors only for the new unhandled `GraphStage` variants in executor (that's fine — we add those next).

- [ ] **Step 8: Commit**

```bash
git add src/graph/pipeline.rs
git commit -m "feat(dsl): add TaintStage, FindDuplicatesStage, coupling/unreferenced predicates to pipeline.rs"
```

---

### Task 2: Refactor `taint.rs` to accept dynamic patterns

**Files:**
- Modify: `src/graph/taint.rs`

The goal: remove the three const arrays (`SOURCES`, `SINKS`, `SANITIZERS`) and make `TaintEngine::analyze_all` accept a `TaintConfig` parameter. Also remove the `&mut CodeGraph` requirement — instead of adding edges to the graph, return findings only.

- [ ] **Step 1: Remove the three const arrays** — delete `SOURCES: &[TaintPattern]`, `SINKS: &[&str]`, and `SANITIZERS: &[&str]` from the file (lines ~14–432).

- [ ] **Step 2: Add `TaintConfig` import** at the top of `taint.rs`:

```rust
use crate::graph::pipeline::{TaintSinkPattern, TaintSourcePattern, TaintSanitizerPattern};
```

- [ ] **Step 3: Replace `TaintPattern` struct** — the old struct had a `'static str` kind enum. Replace with:

```rust
pub struct TaintConfig {
    pub sources: Vec<TaintSourcePattern>,
    pub sinks: Vec<TaintSinkPattern>,
    pub sanitizers: Vec<TaintSanitizerPattern>,
}
```

- [ ] **Step 4: Update `TaintEngine::analyze_all` signature** — change from `pub fn analyze_all(graph: &mut CodeGraph)` to:

```rust
pub fn analyze_all(graph: &CodeGraph, config: &TaintConfig) -> Vec<TaintFinding>
```

Remove all `graph.graph.add_edge(...)` calls and the `GraphEdgeAction` enum entirely — the function now only reads the graph and returns findings.

- [ ] **Step 5: Update internal pattern-matching helpers** — replace references to const `SOURCES`, `SINKS`, `SANITIZERS` arrays with references to `config.sources`, `config.sinks`, `config.sanitizers`. The `is_source_pattern(text)` helper becomes:

```rust
fn is_source_pattern(text: &str, config: &TaintConfig) -> bool {
    config.sources.iter().any(|s| text.contains(s.pattern.as_str()))
}

fn is_sink_pattern(text: &str, config: &TaintConfig) -> bool {
    config.sinks.iter().any(|s| text.contains(s.pattern.as_str()))
}

fn is_sanitizer_pattern(text: &str, config: &TaintConfig) -> bool {
    config.sanitizers.iter().any(|s| text.contains(s.pattern.as_str()))
}
```

Pass `config` through to `analyze_function` and all sub-helpers.

- [ ] **Step 6: Remove `mark_parameter_taint_sources` call** — this function added ExternalSource nodes to the graph. Delete the function and its call from `analyze_all`.

- [ ] **Step 7: Build to check compile errors**

```bash
cargo build 2>&1 | head -60
```

Fix any remaining references to deleted items. Expected: warnings about unused imports in the Rust security pipeline files (those are deleted in Phase B).

- [ ] **Step 8: Commit**

```bash
git add src/graph/taint.rs
git commit -m "refactor(taint): parameterise TaintEngine with TaintConfig; remove const source/sink/sanitizer arrays"
```

---

### Task 3: Implement `execute_taint` in `executor.rs`

**Files:**
- Modify: `src/graph/executor.rs`

- [ ] **Step 1: Add imports** at the top of executor.rs:

```rust
use crate::graph::pipeline::{TaintStage, FindDuplicatesStage};
use crate::graph::taint::{TaintConfig, TaintEngine};
```

- [ ] **Step 2: Add new match arms** in the `execute_stage` function after the `GraphStage::ComputeMetric` arm:

```rust
        GraphStage::Taint { taint } => {
            execute_taint(taint, graph)
        }
        GraphStage::FindDuplicates { find_duplicates } => {
            Ok(execute_find_duplicates(find_duplicates, nodes))
        }
```

- [ ] **Step 3: Implement `execute_taint`** — add this function to executor.rs:

```rust
fn execute_taint(
    stage: &TaintStage,
    graph: &CodeGraph,
) -> anyhow::Result<Vec<PipelineNode>> {
    use crate::graph::pipeline::{TaintSourcePattern, TaintSinkPattern, TaintSanitizerPattern};

    let config = TaintConfig {
        sources: stage.sources.clone(),
        sinks: stage.sinks.clone(),
        sanitizers: stage.sanitizers.clone(),
    };

    let findings = TaintEngine::analyze_all(graph, &config);

    let nodes = findings
        .into_iter()
        .map(|f| {
            let mut metrics = HashMap::new();
            metrics.insert("sink".to_string(), MetricValue::Text(f.sink_name.clone()));
            metrics.insert(
                "vulnerability".to_string(),
                // derive vulnerability from sink pattern match
                MetricValue::Text(
                    stage
                        .sinks
                        .iter()
                        .find(|s| f.sink_name.contains(s.pattern.as_str()))
                        .map(|s| s.vulnerability.clone())
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
            );
            metrics.insert("tainted_var".to_string(), MetricValue::Text(f.tainted_var.clone()));
            PipelineNode {
                node_idx: f.function_node,
                file_path: f.file_path.clone(),
                name: f.function_name.clone(),
                kind: "taint_finding".to_string(),
                line: f.sink_line,
                exported: false,
                language: String::new(),
                metrics,
            }
        })
        .collect();

    Ok(nodes)
}
```

- [ ] **Step 4: Implement `execute_find_duplicates`** — add this function to executor.rs:

```rust
fn execute_find_duplicates(
    stage: &FindDuplicatesStage,
    nodes: Vec<PipelineNode>,
) -> Vec<PipelineNode> {
    use std::collections::HashMap;

    // Group nodes by the `by` property (currently only "name" is supported)
    let mut groups: HashMap<String, Vec<PipelineNode>> = HashMap::new();
    for node in nodes {
        let key = match stage.by.as_str() {
            "name" => node.name.clone(),
            other => node.metrics.get(other)
                .map(|v| match v {
                    MetricValue::Text(s) => s.clone(),
                    MetricValue::Int(i) => i.to_string(),
                    MetricValue::Float(f) => f.to_string(),
                })
                .unwrap_or_default(),
        };
        groups.entry(key).or_default().push(node);
    }

    // Keep only groups meeting min_count; emit one representative node per group
    groups
        .into_iter()
        .filter(|(_, members)| members.len() >= stage.min_count)
        .map(|(key, members)| {
            let count = members.len();
            let files: Vec<String> = members.iter().map(|n| n.file_path.clone()).collect();
            let representative = members.into_iter().next().unwrap();
            let mut node = representative;
            node.metrics.insert("count".to_string(), MetricValue::Int(count as i64));
            node.metrics.insert(
                "files".to_string(),
                MetricValue::Text(files.join(", ")),
            );
            node.metrics.insert("name".to_string(), MetricValue::Text(key));
            node
        })
        .collect()
}
```

- [ ] **Step 5: Implement efferent and afferent coupling in `execute_compute_metric`** — in the existing `execute_compute_metric` function, add two new match arms. Find where metric strings are dispatched (the function currently handles `"cyclomatic_complexity"`, `"function_length"`, etc.) and add:

```rust
        "efferent_coupling" => {
            for node in &mut nodes {
                let count = graph
                    .graph
                    .edges_directed(node.node_idx, Direction::Outgoing)
                    .filter(|e| matches!(e.weight(), EdgeWeight::Imports))
                    .count();
                node.metrics.insert(
                    "efferent_coupling".to_string(),
                    MetricValue::Int(count as i64),
                );
            }
            Ok(nodes)
        }
        "afferent_coupling" => {
            for node in &mut nodes {
                let count = graph
                    .graph
                    .edges_directed(node.node_idx, Direction::Incoming)
                    .filter(|e| {
                        matches!(e.weight(), EdgeWeight::Imports | EdgeWeight::Calls)
                    })
                    .count();
                node.metrics.insert(
                    "afferent_coupling".to_string(),
                    MetricValue::Int(count as i64),
                );
            }
            Ok(nodes)
        }
```

Note: `execute_compute_metric` currently takes `workspace` but not `graph`. Change the signature to also accept `graph: &CodeGraph` and thread it through from `execute_stage`.

- [ ] **Step 6: Implement `unreferenced` and `is_entry_point` metric population** — these predicates are evaluated via the node's metrics map (see `WhereClause::eval` in Task 1). Add a new stage implementation that pre-populates these metrics before a `where` filter:

In `execute_select` (the select stage), after building each `PipelineNode` for symbols, compute and insert both values:

```rust
// unreferenced: no incoming Calls or Imports edges from outside this file
let incoming = graph.graph
    .edges_directed(sym_idx, Direction::Incoming)
    .filter(|e| {
        matches!(e.weight(), EdgeWeight::Calls | EdgeWeight::Imports)
            && node_file_path(e.source(), &graph.graph) != file_path
    })
    .count();
node.metrics.insert(
    "unreferenced".to_string(),
    MetricValue::Int(if incoming == 0 { 1 } else { 0 }),
);

// is_entry_point: file path matches known entry-point names
const ENTRY_POINT_NAMES: &[&str] = &[
    "main", "lib", "mod", "index", "__init__", "__main__",
];
let stem = std::path::Path::new(&file_path)
    .file_stem()
    .and_then(|s| s.to_str())
    .unwrap_or("");
let ep = ENTRY_POINT_NAMES.iter().any(|&e| stem == e);
node.metrics.insert(
    "is_entry_point".to_string(),
    MetricValue::Int(if ep { 1 } else { 0 }),
);
```

- [ ] **Step 7: Build and run tests**

```bash
cargo test 2>&1 | tail -30
```

Expected: all existing tests pass (no Rust has been deleted yet). Fix any compile errors.

- [ ] **Step 8: Commit**

```bash
git add src/graph/executor.rs src/graph/taint.rs
git commit -m "feat(executor): add taint stage, find_duplicates stage, coupling metrics, unreferenced/is_entry_point predicates"
```

---

### Task 4: Write SQL injection JSON pipelines (8 files)

**Files:** Create 8 files in `src/audit/builtin/`

- [ ] **Step 1: Create `sql_injection_python.json`**

```json
{
  "pipeline": "sql_injection_python",
  "category": "security",
  "description": "Detect SQL injection via taint analysis: user-controlled data reaching SQL execution sinks without sanitization",
  "languages": ["python"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "request.form",       "kind": "user_input"},
          {"pattern": "request.args",       "kind": "user_input"},
          {"pattern": "request.data",       "kind": "user_input"},
          {"pattern": "request.json",       "kind": "user_input"},
          {"pattern": "request.values",     "kind": "user_input"},
          {"pattern": "os.environ",         "kind": "env_var"},
          {"pattern": "os.environ.get",     "kind": "env_var"},
          {"pattern": "input(",             "kind": "user_input"},
          {"pattern": "sys.argv",           "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "cursor.execute",      "vulnerability": "sql_injection"},
          {"pattern": "cursor.executemany",  "vulnerability": "sql_injection"},
          {"pattern": "db.execute",          "vulnerability": "sql_injection"},
          {"pattern": "connection.execute",  "vulnerability": "sql_injection"},
          {"pattern": "session.execute",     "vulnerability": "sql_injection"},
          {"pattern": "engine.execute",      "vulnerability": "sql_injection"},
          {"pattern": "raw(",                "vulnerability": "sql_injection"},
          {"pattern": "text(",               "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "escape"},
          {"pattern": "quote"},
          {"pattern": "prepare"},
          {"pattern": "parameterize"},
          {"pattern": "bind_param"},
          {"pattern": "placeholder"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 2: Create `sql_injection_go.json`**

```json
{
  "pipeline": "sql_injection_go",
  "category": "security",
  "description": "Detect SQL injection in Go via taint analysis",
  "languages": ["go"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "r.URL.Query()",       "kind": "user_input"},
          {"pattern": "r.FormValue",         "kind": "user_input"},
          {"pattern": "r.PostFormValue",     "kind": "user_input"},
          {"pattern": "r.Header.Get",        "kind": "user_input"},
          {"pattern": "r.Body",              "kind": "user_input"},
          {"pattern": "os.Getenv",           "kind": "env_var"},
          {"pattern": "os.Args",             "kind": "user_input"},
          {"pattern": "flag.String",         "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "db.Query",            "vulnerability": "sql_injection"},
          {"pattern": "db.Exec",             "vulnerability": "sql_injection"},
          {"pattern": "db.QueryRow",         "vulnerability": "sql_injection"},
          {"pattern": "tx.Query",            "vulnerability": "sql_injection"},
          {"pattern": "tx.Exec",             "vulnerability": "sql_injection"},
          {"pattern": "Sprintf",             "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "db.Prepare"},
          {"pattern": "db.PrepareContext"},
          {"pattern": "strconv.Atoi"},
          {"pattern": "strconv.ParseInt"},
          {"pattern": "regexp.MustCompile"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 3: Create `sql_injection_java.json`**

```json
{
  "pipeline": "sql_injection_java",
  "category": "security",
  "description": "Detect SQL injection in Java via taint analysis",
  "languages": ["java"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "getParameter",        "kind": "user_input"},
          {"pattern": "getHeader",           "kind": "user_input"},
          {"pattern": "getInputStream",      "kind": "user_input"},
          {"pattern": "getReader",           "kind": "user_input"},
          {"pattern": "System.getenv",       "kind": "env_var"},
          {"pattern": "System.getProperty",  "kind": "env_var"},
          {"pattern": "args[",               "kind": "user_input"},
          {"pattern": "readLine",            "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "executeQuery",        "vulnerability": "sql_injection"},
          {"pattern": "executeUpdate",       "vulnerability": "sql_injection"},
          {"pattern": "execute(",            "vulnerability": "sql_injection"},
          {"pattern": "createStatement",     "vulnerability": "sql_injection"},
          {"pattern": "addBatch",            "vulnerability": "sql_injection"},
          {"pattern": "nativeQuery",         "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "PreparedStatement"},
          {"pattern": "prepareStatement"},
          {"pattern": "setString"},
          {"pattern": "setInt"},
          {"pattern": "parameterize"},
          {"pattern": "StringEscapeUtils.escapeSql"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 4: Create `sql_injection_javascript.json`**

```json
{
  "pipeline": "sql_injection_javascript",
  "category": "security",
  "description": "Detect SQL injection in JavaScript via taint analysis",
  "languages": ["javascript"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "req.body",            "kind": "user_input"},
          {"pattern": "req.query",           "kind": "user_input"},
          {"pattern": "req.params",          "kind": "user_input"},
          {"pattern": "req.headers",         "kind": "user_input"},
          {"pattern": "request.body",        "kind": "user_input"},
          {"pattern": "request.query",       "kind": "user_input"},
          {"pattern": "process.env",         "kind": "env_var"},
          {"pattern": "process.argv",        "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "query(",              "vulnerability": "sql_injection"},
          {"pattern": "execute(",            "vulnerability": "sql_injection"},
          {"pattern": "raw(",                "vulnerability": "sql_injection"},
          {"pattern": "db.query",            "vulnerability": "sql_injection"},
          {"pattern": "connection.query",    "vulnerability": "sql_injection"},
          {"pattern": "pool.query",          "vulnerability": "sql_injection"},
          {"pattern": "knex.raw",            "vulnerability": "sql_injection"},
          {"pattern": "sequelize.query",     "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "escape"},
          {"pattern": "sanitize"},
          {"pattern": "parameterize"},
          {"pattern": "placeholder"},
          {"pattern": "parseInt"},
          {"pattern": "parseFloat"},
          {"pattern": "Number("}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Create `sql_injection_typescript.json`** — same content as javascript variant but `"languages": ["typescript"]` and pipeline name `"sql_injection_typescript"`.

- [ ] **Step 6: Create `sql_injection_php.json`**

```json
{
  "pipeline": "sql_injection_php",
  "category": "security",
  "description": "Detect SQL injection in PHP via taint analysis",
  "languages": ["php"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "$_GET",               "kind": "user_input"},
          {"pattern": "$_POST",              "kind": "user_input"},
          {"pattern": "$_REQUEST",           "kind": "user_input"},
          {"pattern": "$_COOKIE",            "kind": "user_input"},
          {"pattern": "$_SERVER",            "kind": "user_input"},
          {"pattern": "getenv(",             "kind": "env_var"},
          {"pattern": "file_get_contents",   "kind": "user_input"},
          {"pattern": "fgets",               "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "mysqli_query",        "vulnerability": "sql_injection"},
          {"pattern": "pg_query",            "vulnerability": "sql_injection"},
          {"pattern": "mysql_query",         "vulnerability": "sql_injection"},
          {"pattern": "->query(",            "vulnerability": "sql_injection"},
          {"pattern": "->exec(",             "vulnerability": "sql_injection"},
          {"pattern": "PDO::query",          "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "mysqli_real_escape_string"},
          {"pattern": "pg_escape_string"},
          {"pattern": "addslashes"},
          {"pattern": "prepare("},
          {"pattern": "bindParam"},
          {"pattern": "bindValue"},
          {"pattern": "filter_var"},
          {"pattern": "filter_input"},
          {"pattern": "intval"},
          {"pattern": "htmlspecialchars"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 7: Create `sql_injection_csharp.json`**

```json
{
  "pipeline": "sql_injection_csharp",
  "category": "security",
  "description": "Detect SQL injection in C# via taint analysis",
  "languages": ["csharp"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "Request.Query",                     "kind": "user_input"},
          {"pattern": "Request.Form",                      "kind": "user_input"},
          {"pattern": "Request.Headers",                   "kind": "user_input"},
          {"pattern": "Request.Body",                      "kind": "user_input"},
          {"pattern": "Environment.GetEnvironmentVariable","kind": "env_var"},
          {"pattern": "Console.ReadLine",                  "kind": "user_input"},
          {"pattern": "args[",                             "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "ExecuteNonQuery",     "vulnerability": "sql_injection"},
          {"pattern": "ExecuteScalar",       "vulnerability": "sql_injection"},
          {"pattern": "ExecuteReader",       "vulnerability": "sql_injection"},
          {"pattern": "FromSqlRaw",          "vulnerability": "sql_injection"},
          {"pattern": "ExecuteSqlRaw",       "vulnerability": "sql_injection"},
          {"pattern": "SqlCommand(",         "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "SqlParameter"},
          {"pattern": "AddWithValue"},
          {"pattern": "Parameters.Add"},
          {"pattern": "HtmlEncode"},
          {"pattern": "AntiXss"},
          {"pattern": "parameterize"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 8: Create `sql_injection_cpp.json`**

```json
{
  "pipeline": "sql_injection_cpp",
  "category": "security",
  "description": "Detect SQL injection in C++ via taint analysis",
  "languages": ["cpp"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "getenv(",             "kind": "env_var"},
          {"pattern": "fgets(",              "kind": "user_input"},
          {"pattern": "scanf(",              "kind": "user_input"},
          {"pattern": "cin >>",              "kind": "user_input"},
          {"pattern": "readline(",           "kind": "user_input"},
          {"pattern": "argv[",               "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "mysql_query(",        "vulnerability": "sql_injection"},
          {"pattern": "sqlite3_exec(",       "vulnerability": "sql_injection"},
          {"pattern": "PQexec(",             "vulnerability": "sql_injection"},
          {"pattern": "execute(",            "vulnerability": "sql_injection"},
          {"pattern": "sprintf(",            "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "mysql_real_escape_string"},
          {"pattern": "sqlite3_prepare"},
          {"pattern": "PQprepare"},
          {"pattern": "snprintf"},
          {"pattern": "strlcpy"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 9: Run cargo test**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests pass. The new JSON files are discovered by the engine but don't affect existing tests.

- [ ] **Step 10: Commit**

```bash
git add src/audit/builtin/sql_injection_*.json
git commit -m "feat(pipelines): add SQL injection JSON pipelines for 8 languages"
```

---

### Task 5: Write SSRF, XSS, and XXE JSON pipelines (11 files)

**Files:** Create 11 files in `src/audit/builtin/`

- [ ] **Step 1: Create `ssrf_python.json`**

```json
{
  "pipeline": "ssrf_python",
  "category": "security",
  "description": "Detect SSRF in Python: user input reaching outbound network calls",
  "languages": ["python"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "request.form",     "kind": "user_input"},
          {"pattern": "request.args",     "kind": "user_input"},
          {"pattern": "request.data",     "kind": "user_input"},
          {"pattern": "request.json",     "kind": "user_input"},
          {"pattern": "os.environ.get",   "kind": "env_var"},
          {"pattern": "input(",           "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "requests.get",     "vulnerability": "ssrf"},
          {"pattern": "requests.post",    "vulnerability": "ssrf"},
          {"pattern": "requests.put",     "vulnerability": "ssrf"},
          {"pattern": "urllib.request",   "vulnerability": "ssrf"},
          {"pattern": "urllib.urlopen",   "vulnerability": "ssrf"},
          {"pattern": "http.client",      "vulnerability": "ssrf"},
          {"pattern": "aiohttp.ClientSession", "vulnerability": "ssrf"},
          {"pattern": "httpx.get",        "vulnerability": "ssrf"},
          {"pattern": "redirect(",        "vulnerability": "ssrf"}
        ],
        "sanitizers": [
          {"pattern": "urlparse"},
          {"pattern": "validate_url"},
          {"pattern": "is_safe_url"},
          {"pattern": "allowlist"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "ssrf",
        "message": "SSRF: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 2: Create `ssrf_javascript.json`** — same structure; sources use `req.body`, `req.query`, etc.; sinks use `fetch(`, `axios.get`, `http.get`, `request(`, `got(`, `superagent`; sanitizers use `new URL(`, `urlparse`, `allowlist`.

```json
{
  "pipeline": "ssrf_javascript",
  "category": "security",
  "description": "Detect SSRF in JavaScript: user input reaching outbound network calls",
  "languages": ["javascript"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "req.body",         "kind": "user_input"},
          {"pattern": "req.query",        "kind": "user_input"},
          {"pattern": "req.params",       "kind": "user_input"},
          {"pattern": "req.headers",      "kind": "user_input"},
          {"pattern": "process.env",      "kind": "env_var"}
        ],
        "sinks": [
          {"pattern": "fetch(",           "vulnerability": "ssrf"},
          {"pattern": "axios.get",        "vulnerability": "ssrf"},
          {"pattern": "axios.post",       "vulnerability": "ssrf"},
          {"pattern": "http.get(",        "vulnerability": "ssrf"},
          {"pattern": "http.request(",    "vulnerability": "ssrf"},
          {"pattern": "got(",             "vulnerability": "ssrf"},
          {"pattern": "request(",         "vulnerability": "ssrf"},
          {"pattern": "res.redirect",     "vulnerability": "ssrf"}
        ],
        "sanitizers": [
          {"pattern": "new URL("},
          {"pattern": "urlparse"},
          {"pattern": "allowlist"},
          {"pattern": "isAllowed"},
          {"pattern": "validateUrl"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "ssrf",
        "message": "SSRF: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 3: Create `ssrf_php.json`** — sources: `$_GET`, `$_POST`, `$_REQUEST`; sinks: `file_get_contents(`, `curl_exec(`, `curl_setopt(`, `header("Location`; sanitizers: `filter_var`, `FILTER_VALIDATE_URL`, `parse_url`.

```json
{
  "pipeline": "ssrf_php",
  "category": "security",
  "description": "Detect SSRF in PHP",
  "languages": ["php"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "$_GET",            "kind": "user_input"},
          {"pattern": "$_POST",           "kind": "user_input"},
          {"pattern": "$_REQUEST",        "kind": "user_input"},
          {"pattern": "$_COOKIE",         "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "file_get_contents", "vulnerability": "ssrf"},
          {"pattern": "curl_exec(",        "vulnerability": "ssrf"},
          {"pattern": "curl_setopt(",      "vulnerability": "ssrf"},
          {"pattern": "fopen(",            "vulnerability": "ssrf"},
          {"pattern": "header(\"Location", "vulnerability": "ssrf"}
        ],
        "sanitizers": [
          {"pattern": "filter_var"},
          {"pattern": "FILTER_VALIDATE_URL"},
          {"pattern": "parse_url"},
          {"pattern": "allowlist"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "ssrf",
        "message": "SSRF: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 4: Create `ssrf_go.json`** — sources: `r.URL.Query()`, `r.FormValue`, `r.Header.Get`, `os.Getenv`; sinks: `http.Get(`, `http.Post(`, `http.NewRequest(`, `net.Dial(`; sanitizers: `url.Parse`, `url.ParseRequestURI`.

```json
{
  "pipeline": "ssrf_go",
  "category": "security",
  "description": "Detect SSRF in Go",
  "languages": ["go"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "r.URL.Query()",    "kind": "user_input"},
          {"pattern": "r.FormValue",      "kind": "user_input"},
          {"pattern": "r.Header.Get",     "kind": "user_input"},
          {"pattern": "r.Body",           "kind": "user_input"},
          {"pattern": "os.Getenv",        "kind": "env_var"}
        ],
        "sinks": [
          {"pattern": "http.Get(",        "vulnerability": "ssrf"},
          {"pattern": "http.Post(",       "vulnerability": "ssrf"},
          {"pattern": "http.NewRequest(", "vulnerability": "ssrf"},
          {"pattern": "net.Dial(",        "vulnerability": "ssrf"},
          {"pattern": "http.Redirect(",   "vulnerability": "ssrf"}
        ],
        "sanitizers": [
          {"pattern": "url.Parse"},
          {"pattern": "url.ParseRequestURI"},
          {"pattern": "allowlist"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "ssrf",
        "message": "SSRF: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Create `ssrf_java.json`** — sources: `getParameter`, `getHeader`, `getInputStream`; sinks: `new URL(`, `HttpURLConnection`, `HttpClient.send(`, `RestTemplate`, `WebClient`; sanitizers: `URI.create`, `allowlist`.

```json
{
  "pipeline": "ssrf_java",
  "category": "security",
  "description": "Detect SSRF in Java",
  "languages": ["java"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "getParameter",     "kind": "user_input"},
          {"pattern": "getHeader",        "kind": "user_input"},
          {"pattern": "getInputStream",   "kind": "user_input"},
          {"pattern": "System.getenv",    "kind": "env_var"},
          {"pattern": "args[",            "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "new URL(",         "vulnerability": "ssrf"},
          {"pattern": "HttpURLConnection","vulnerability": "ssrf"},
          {"pattern": "HttpClient.send",  "vulnerability": "ssrf"},
          {"pattern": "RestTemplate",     "vulnerability": "ssrf"},
          {"pattern": "WebClient",        "vulnerability": "ssrf"},
          {"pattern": "response.sendRedirect", "vulnerability": "ssrf"}
        ],
        "sanitizers": [
          {"pattern": "URI.create"},
          {"pattern": "allowlist"},
          {"pattern": "isAllowed"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "ssrf",
        "message": "SSRF: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 6: Create `ssrf_csharp.json`** — sources: `Request.Query`, `Request.Form`; sinks: `HttpClient.GetAsync(`, `HttpClient.PostAsync(`, `new HttpRequestMessage(`, `WebClient.DownloadString(`; sanitizers: `Uri.IsWellFormedUriString`, `allowlist`.

```json
{
  "pipeline": "ssrf_csharp",
  "category": "security",
  "description": "Detect SSRF in C#",
  "languages": ["csharp"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "Request.Query",    "kind": "user_input"},
          {"pattern": "Request.Form",     "kind": "user_input"},
          {"pattern": "Request.Headers",  "kind": "user_input"},
          {"pattern": "Environment.GetEnvironmentVariable", "kind": "env_var"},
          {"pattern": "Console.ReadLine", "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "HttpClient.GetAsync",    "vulnerability": "ssrf"},
          {"pattern": "HttpClient.PostAsync",   "vulnerability": "ssrf"},
          {"pattern": "new HttpRequestMessage", "vulnerability": "ssrf"},
          {"pattern": "WebClient.DownloadString","vulnerability": "ssrf"},
          {"pattern": "Response.Redirect",      "vulnerability": "ssrf"}
        ],
        "sanitizers": [
          {"pattern": "Uri.IsWellFormedUriString"},
          {"pattern": "allowlist"},
          {"pattern": "isAllowed"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "ssrf",
        "message": "SSRF: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 7: Create `xss_javascript.json`**

```json
{
  "pipeline": "xss_javascript",
  "category": "security",
  "description": "Detect DOM-based XSS in JavaScript: user input reaching DOM sinks",
  "languages": ["javascript"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "req.body",          "kind": "user_input"},
          {"pattern": "req.query",         "kind": "user_input"},
          {"pattern": "req.params",        "kind": "user_input"},
          {"pattern": "location.search",   "kind": "user_input"},
          {"pattern": "location.href",     "kind": "user_input"},
          {"pattern": "document.cookie",   "kind": "user_input"},
          {"pattern": "window.location",   "kind": "user_input"},
          {"pattern": "URLSearchParams",   "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "innerHTML",                   "vulnerability": "xss"},
          {"pattern": "outerHTML",                   "vulnerability": "xss"},
          {"pattern": "document.write(",             "vulnerability": "xss"},
          {"pattern": "document.writeln(",           "vulnerability": "xss"},
          {"pattern": "dangerouslySetInnerHTML",     "vulnerability": "xss"},
          {"pattern": "insertAdjacentHTML",          "vulnerability": "xss"},
          {"pattern": "eval(",                       "vulnerability": "xss"},
          {"pattern": "Function(",                   "vulnerability": "xss"},
          {"pattern": "setTimeout(",                 "vulnerability": "xss"},
          {"pattern": "setInterval(",                "vulnerability": "xss"}
        ],
        "sanitizers": [
          {"pattern": "DOMPurify.sanitize"},
          {"pattern": "xss("},
          {"pattern": "escapeHtml"},
          {"pattern": "encodeURIComponent"},
          {"pattern": "encodeURI"},
          {"pattern": "textContent"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "xss",
        "message": "XSS: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 8: Create `xss_typescript.json`** — identical to xss_javascript.json with `"pipeline": "xss_typescript"` and `"languages": ["typescript"]`.

- [ ] **Step 9: Create `xxe_python.json`**

```json
{
  "pipeline": "xxe_python",
  "category": "security",
  "description": "Detect XXE in Python: user-controlled XML parsed without disabling external entities",
  "languages": ["python"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "request.data",     "kind": "user_input"},
          {"pattern": "request.body",     "kind": "user_input"},
          {"pattern": "request.files",    "kind": "user_input"},
          {"pattern": "request.form",     "kind": "user_input"},
          {"pattern": "input(",           "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "etree.parse(",             "vulnerability": "xxe"},
          {"pattern": "etree.fromstring(",        "vulnerability": "xxe"},
          {"pattern": "xml.etree.ElementTree.parse", "vulnerability": "xxe"},
          {"pattern": "lxml.etree.parse(",        "vulnerability": "xxe"},
          {"pattern": "minidom.parseString(",     "vulnerability": "xxe"},
          {"pattern": "sax.parse(",               "vulnerability": "xxe"},
          {"pattern": "xmltodict.parse(",         "vulnerability": "xxe"}
        ],
        "sanitizers": [
          {"pattern": "defusedxml"},
          {"pattern": "resolve_entities=False"},
          {"pattern": "XMLParser(resolve_entities=False)"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "xxe",
        "message": "XXE: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 10: Create `xxe_java.json`** — sources: `getInputStream`, `getParameter`; sinks: `DocumentBuilder.parse(`, `SAXParser.parse(`, `XMLReader.parse(`, `Unmarshaller.unmarshal(`; sanitizers: `setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)`, `setExpandEntityReferences(false)`.

```json
{
  "pipeline": "xxe_java",
  "category": "security",
  "description": "Detect XXE in Java",
  "languages": ["java"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "getInputStream",          "kind": "user_input"},
          {"pattern": "getParameter",            "kind": "user_input"},
          {"pattern": "getReader",               "kind": "user_input"},
          {"pattern": "System.in",               "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "DocumentBuilder.parse",   "vulnerability": "xxe"},
          {"pattern": "SAXParser.parse",         "vulnerability": "xxe"},
          {"pattern": "XMLReader.parse",         "vulnerability": "xxe"},
          {"pattern": "Unmarshaller.unmarshal",  "vulnerability": "xxe"},
          {"pattern": "XPathExpression.evaluate","vulnerability": "xxe"}
        ],
        "sanitizers": [
          {"pattern": "setExpandEntityReferences(false)"},
          {"pattern": "disallow-doctype-decl"},
          {"pattern": "setFeature(XMLConstants"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "xxe",
        "message": "XXE: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 11: Create `xxe_csharp.json`** — sources: `Request.Body`, `Request.Form`; sinks: `XmlDocument.Load(`, `XmlReader.Create(`, `XDocument.Load(`; sanitizers: `XmlReaderSettings { DtdProcessing = DtdProcessing.Prohibit }`, `ProhibitDtd = true`.

```json
{
  "pipeline": "xxe_csharp",
  "category": "security",
  "description": "Detect XXE in C#",
  "languages": ["csharp"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "Request.Body",     "kind": "user_input"},
          {"pattern": "Request.Form",     "kind": "user_input"},
          {"pattern": "Console.ReadLine", "kind": "user_input"}
        ],
        "sinks": [
          {"pattern": "XmlDocument.Load",   "vulnerability": "xxe"},
          {"pattern": "XmlReader.Create",   "vulnerability": "xxe"},
          {"pattern": "XDocument.Load",     "vulnerability": "xxe"},
          {"pattern": "XmlSerializer",      "vulnerability": "xxe"}
        ],
        "sanitizers": [
          {"pattern": "DtdProcessing.Prohibit"},
          {"pattern": "ProhibitDtd = true"},
          {"pattern": "XmlReaderSettings"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "xxe",
        "message": "XXE: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 12: Run cargo test**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 13: Commit**

```bash
git add src/audit/builtin/ssrf_*.json src/audit/builtin/xss_*.json src/audit/builtin/xxe_*.json
git commit -m "feat(pipelines): add SSRF (6), XSS (2), and XXE (3) JSON pipelines"
```

---

### Task 6: Write cross-file analyzer replacement JSON pipelines (3 files)

**Files:** Create 3 files in `src/audit/builtin/`

- [ ] **Step 1: Create `coupling.json`**

```json
{
  "pipeline": "coupling",
  "category": "code_style",
  "description": "Detect files with excessive outgoing imports (high efferent coupling / fan-out)",
  "graph": [
    {
      "select": "file",
      "exclude": {
        "or": [
          {"is_test_file": true},
          {"is_generated": true},
          {"is_barrel_file": true}
        ]
      }
    },
    {"compute_metric": "efferent_coupling"},
    {
      "flag": {
        "pattern": "high_efferent_coupling",
        "message": "{{file}} imports from {{efferent_coupling}} modules — high fan-out coupling",
        "severity_map": [
          {"when": {"efferent_coupling": {"gte": 15}}, "severity": "error"},
          {"when": {"efferent_coupling": {"gte": 8}},  "severity": "warning"}
        ]
      }
    }
  ]
}
```

- [ ] **Step 2: Create `dead_exports.json`**

```json
{
  "pipeline": "dead_exports",
  "category": "code_style",
  "description": "Detect exported symbols that are never referenced from outside their defining file",
  "graph": [
    {
      "select": "symbol",
      "where": {
        "and": [
          {"exported": true},
          {"is_entry_point": false}
        ]
      },
      "exclude": {
        "or": [
          {"is_test_file": true},
          {"is_generated": true}
        ]
      }
    },
    {
      "flag": {
        "pattern": "dead_export",
        "message": "{{name}} ({{kind}}) in {{file}}:{{line}} is exported but never referenced",
        "severity": "warning",
        "where": {"unreferenced": true}
      }
    }
  ]
}
```

Note: `dead_exports.json` uses a `where` clause on the `flag` stage to filter at emit time rather than as a separate stage. If the executor's flag stage does not support a `where` filter, use an explicit `Select`-like filter stage instead:

```json
    {"where": {"unreferenced": true}},
    {
      "flag": {
        "pattern": "dead_export",
        "message": "{{name}} ({{kind}}) in {{file}}:{{line}} is exported but never referenced",
        "severity": "warning"
      }
    }
```

Use whichever form compiles without adding new executor logic.

- [ ] **Step 3: Create `duplicate_symbols.json`**

```json
{
  "pipeline": "duplicate_symbols",
  "category": "code_style",
  "description": "Detect exported symbols with the same name defined across multiple files",
  "graph": [
    {
      "select": "symbol",
      "where": {"exported": true},
      "exclude": {
        "or": [
          {"is_test_file": true},
          {"is_generated": true}
        ]
      }
    },
    {"find_duplicates": {"by": "name", "min_count": 2}},
    {
      "flag": {
        "pattern": "duplicate_symbol",
        "message": "{{name}} is defined in {{count}} files: {{files}}",
        "severity": "info"
      }
    }
  ]
}
```

- [ ] **Step 4: Run cargo test**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/audit/builtin/coupling.json src/audit/builtin/dead_exports.json src/audit/builtin/duplicate_symbols.json
git commit -m "feat(pipelines): add coupling, dead_exports, duplicate_symbols JSON pipelines (replace Rust analyzers)"
```

---

## Phase B — Rust Deletion

### Task 7: Delete Rust security pipeline files

**Files:** Delete 15 files; modify 6 `mod.rs` files

- [ ] **Step 1: Delete Python security pipelines**

```bash
rm src/audit/pipelines/python/sql_injection.rs
rm src/audit/pipelines/python/ssrf.rs
```

Remove the corresponding module declarations from `src/audit/pipelines/python/mod.rs` — delete the lines `pub mod sql_injection;` and `pub mod ssrf;`.

- [ ] **Step 2: Delete JavaScript security pipelines**

```bash
rm src/audit/pipelines/javascript/xss_dom_injection.rs
rm src/audit/pipelines/javascript/ssrf.rs
```

Remove `pub mod xss_dom_injection;` and `pub mod ssrf;` from `src/audit/pipelines/javascript/mod.rs`.

- [ ] **Step 3: Delete TypeScript directory**

```bash
rm -rf src/audit/pipelines/typescript/
```

Remove `pub mod typescript;` from `src/audit/pipelines/mod.rs`.

- [ ] **Step 4: Delete Go security pipelines**

```bash
rm src/audit/pipelines/go/sql_injection.rs
rm src/audit/pipelines/go/ssrf_open_redirect.rs
```

Remove `pub mod sql_injection;` and `pub mod ssrf_open_redirect;` from `src/audit/pipelines/go/mod.rs`.

- [ ] **Step 5: Delete Java security pipelines**

```bash
rm src/audit/pipelines/java/java_ssrf.rs
rm src/audit/pipelines/java/sql_injection.rs
rm src/audit/pipelines/java/xxe.rs
```

Remove all three module declarations from `src/audit/pipelines/java/mod.rs`.

- [ ] **Step 6: Delete PHP security pipelines**

```bash
rm src/audit/pipelines/php/sql_injection.rs
rm src/audit/pipelines/php/ssrf.rs
```

Remove `pub mod sql_injection;` and `pub mod ssrf;` from `src/audit/pipelines/php/mod.rs`.

- [ ] **Step 7: Delete C# security pipelines**

```bash
rm src/audit/pipelines/csharp/csharp_ssrf.rs
rm src/audit/pipelines/csharp/sql_injection.rs
rm src/audit/pipelines/csharp/xxe.rs
```

Remove all three module declarations from `src/audit/pipelines/csharp/mod.rs`.

- [ ] **Step 8: Build and test**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass. Fix any remaining references to deleted modules.

- [ ] **Step 9: Commit**

```bash
git add -u
git commit -m "chore: delete 15 Rust security pipeline files replaced by JSON taint pipelines"
```

---

### Task 8: Delete cross-file Rust analyzers and `taint.rs`

**Files:** Delete `coupling.rs`, `dead_exports.rs`, `duplicate_symbols.rs`, `taint.rs`; modify `mod.rs` and `engine.rs`

- [ ] **Step 1: Delete the three analyzer files**

```bash
rm src/audit/analyzers/coupling.rs
rm src/audit/analyzers/dead_exports.rs
rm src/audit/analyzers/duplicate_symbols.rs
```

- [ ] **Step 2: Update `src/audit/analyzers/mod.rs`** — remove the three `pub mod` declarations and any re-exports of `CouplingAnalyzer`, `DeadExportsAnalyzer`, `DuplicateSymbolsAnalyzer`.

- [ ] **Step 3: Update `src/audit/engine.rs`** — remove the project-level analyzer dispatch block (lines ~240–270). This is the section:

```rust
// Run project-level analyzers if graph is provided
if let Some(g) = graph {
    let mut project_analyzers: Vec<Box<dyn super::project_analyzer::ProjectAnalyzer>> =
        match self.pipeline_selector {
            PipelineSelector::Architecture => analyzers::architecture_analyzers(),
            PipelineSelector::CodeStyle => analyzers::code_style_analyzers(),
            _ => Vec::new(),
        };
    ...
}
```

Delete this entire block. The `analyzers` import at the top of engine.rs can be removed if nothing else uses it.

- [ ] **Step 4: Delete `taint.rs`**

```bash
rm src/graph/taint.rs
```

- [ ] **Step 5: Update `src/graph/mod.rs`** — remove the `pub mod taint;` declaration. If any public re-export of `TaintEngine` or `TaintFinding` exists, remove it too.

- [ ] **Step 6: Build and test**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass. Fix compile errors (likely: references to deleted types in engine.rs or tests).

- [ ] **Step 7: Update integration tests** — `tests/audit_json_integration.rs` imports `PipelineSelector`. After deleting the Rust analyzers, `PipelineSelector` will still be used by tests. We leave its removal for Task 9 (CLI rewrite). For now, just ensure the tests compile.

- [ ] **Step 8: Delete stale planning docs**

```bash
rm -rf audit_plans/
rm docs/superpowers/plans/2026-04-05-rust-tech-debt-pipeline-improvements.md
rm docs/superpowers/plans/2026-04-06-cpp-architecture-pipeline-fixes.md
```

- [ ] **Step 9: Run full test suite**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 10: Commit**

```bash
git add -u
git commit -m "chore: delete coupling/dead_exports/duplicate_symbols analyzers, taint.rs, and stale planning docs"
```

---

## Phase C — CLI Simplification

### Task 9: Replace `PipelineSelector` with `category_filter` in `engine.rs`

**Files:**
- Modify: `src/audit/engine.rs`

- [ ] **Step 1: Add `category_filter` field to `AuditEngine`**

Remove `pipeline_selector: PipelineSelector` and add:

```rust
pub struct AuditEngine {
    languages: Vec<Language>,
    pipeline_filter: Vec<String>,
    category_filter: Vec<String>,   // NEW: replaces pipeline_selector
    progress: Option<indicatif::ProgressBar>,
    project_dir: Option<std::path::PathBuf>,
}
```

- [ ] **Step 2: Add builder method**

```rust
pub fn categories(mut self, cats: Vec<String>) -> Self {
    self.category_filter = cats;
    self
}
```

- [ ] **Step 3: Delete `PipelineSelector` enum and `pipeline_selector` builder** — remove the enum definition and the `pub fn pipeline_selector(...)` method.

- [ ] **Step 4: Simplify `AuditEngine::run`** — remove the `match self.pipeline_selector { ... }` block that dispatches Rust pipelines. Replace with: all languages get an empty `lang_pipelines` (since all Rust pipelines are now deleted). The pipeline_map block becomes:

```rust
let mut pipeline_map: HashMap<Language, Vec<Arc<AnyPipeline>>> = HashMap::new();
// No Rust pipelines remain — all audit logic is JSON-driven.
// pipeline_map stays empty; JSON audits are executed separately below.
```

- [ ] **Step 5: Wire `category_filter` into JSON audit filtering** — after discovering JSON audits, filter by category:

```rust
let json_audits: Vec<_> = json_audits
    .into_iter()
    .filter(|a| {
        self.category_filter.is_empty()
            || self.category_filter.iter().any(|c| c == &a.category)
    })
    .filter(|a| {
        self.pipeline_filter.is_empty()
            || self.pipeline_filter.iter().any(|p| p == &a.pipeline)
    })
    .collect();
```

- [ ] **Step 6: Build**

```bash
cargo build 2>&1 | head -40
```

Fix compile errors — `PipelineSelector` is still referenced in `tests/audit_json_integration.rs`. Update those test calls to use `.categories(vec!["architecture".to_string()])` instead of `.pipeline_selector(PipelineSelector::Architecture)`.

- [ ] **Step 7: Run tests**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/audit/engine.rs tests/audit_json_integration.rs
git commit -m "refactor(engine): replace PipelineSelector with category_filter; remove Rust pipeline dispatch"
```

---

### Task 10: Rewrite `src/cli.rs` — flat `audit` command

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Delete `AuditCommand` and `CodeQualityCommand` enums** — remove the entire `AuditCommand` enum (lines ~217–420) and `CodeQualityCommand` enum.

- [ ] **Step 2: Remove the nested audit struct** — the current `Audit` variant in `Command` has `command: Option<AuditCommand>`. Replace the entire `Audit` variant with:

```rust
/// Static analysis and tech debt detection
Audit {
    /// Root directory to analyze
    #[arg(conflicts_with = "s3")]
    dir: Option<PathBuf>,

    /// S3 URI — reads codebase directly from S3
    #[arg(long, conflicts_with = "dir")]
    s3: Option<String>,

    /// Comma-separated language filter (rs,go,py,ts,js,java,php,cs,c,cpp)
    #[arg(short, long)]
    language: Option<String>,

    /// Filter by category: security, architecture, code_style, tech_debt, complexity, scalability
    #[arg(long)]
    category: Option<String>,

    /// Comma-separated pipeline name filter
    #[arg(long)]
    pipeline: Option<String>,

    /// Output format
    #[arg(long, default_value = "table")]
    format: OutputFormat,

    /// Run a specific JSON audit file
    #[arg(long, value_name = "FILE")]
    file: Option<PathBuf>,

    /// Findings per page
    #[arg(long, default_value = "20")]
    per_page: usize,

    /// Page number (1-indexed)
    #[arg(long, default_value = "1")]
    page: usize,
},
```

- [ ] **Step 3: Remove unused imports** — `CodeQualityCommand`, `AuditCommand`, and any related ValueEnum types that were only used by the deleted enums.

- [ ] **Step 4: Build**

```bash
cargo build 2>&1 | head -40
```

Expected: compile errors in `main.rs` because it still references the old `AuditCommand` arms. Fix in Task 11.

- [ ] **Step 5: Commit (partial build OK — main.rs fixes in next task)**

```bash
git add src/cli.rs
git commit -m "refactor(cli): flatten audit command — replace 6 nested subcommands with single flat AuditArgs"
```

---

### Task 11: Rewrite audit dispatch in `src/main.rs`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Remove all old audit subcommand imports** from `main.rs`:

```rust
// DELETE these from the use statement:
use virgil_cli::cli::{AuditCommand, CodeQualityCommand, ...};
// KEEP:
use virgil_cli::cli::{Cli, Command, OutputFormat, ProjectCommand, QueryOutputFormat};
```

- [ ] **Step 2: Replace the entire audit dispatch block** — find the `Command::Audit { dir, s3, language, format, file, command }` match arm and replace it with:

```rust
Command::Audit {
    dir,
    s3,
    language,
    category,
    pipeline,
    format,
    file,
    per_page,
    page,
} => {
    let (workspace, root) = resolve_workspace(dir.as_deref(), s3.as_deref(), language.as_deref(), &[])?;

    let languages = workspace.languages().to_vec();

    let pb = indicatif::ProgressBar::new(0);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner} [{bar:40}] {pos}/{len} files")
            .unwrap()
            .progress_chars("=> "),
    );

    let mut engine = AuditEngine::new()
        .languages(languages.clone())
        .progress_bar(pb);

    if let Some(ref dir_path) = dir {
        engine = engine.project_dir(dir_path.clone());
    }

    if let Some(ref cats) = category {
        let cat_list: Vec<String> = cats.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.categories(cat_list);
    }

    if let Some(ref pipes) = pipeline {
        let pipe_list: Vec<String> = pipes.split(',').map(|s| s.trim().to_string()).collect();
        engine = engine.pipelines(pipe_list);
    }

    if let Some(ref extra_file) = file {
        // Load and run a single custom JSON audit file
        let content = std::fs::read_to_string(extra_file)?;
        let audit: virgil_cli::audit::json_audit::JsonAuditFile = serde_json::from_str(&content)?;
        let graph = GraphBuilder::new(&workspace, &languages).build()?;
        let output = virgil_cli::graph::executor::run_pipeline(
            &audit.graph,
            &graph,
            Some(&workspace),
            Some(&audit.languages.iter().map(|s| s.clone()).collect::<Vec<_>>()),
            None,
            &audit.pipeline,
        )?;
        // Print findings from the single file run
        print_pipeline_output(output, &format, per_page, page);
        return Ok(());
    }

    let graph = GraphBuilder::new(&workspace, &languages).build()?;
    let (findings, summary) = engine.run(&workspace, Some(&graph))?;

    print_audit_results(findings, summary, &format, per_page, page);
    Ok(())
}
```

Note: `resolve_workspace`, `print_audit_results`, `print_pipeline_output` are existing helpers in main.rs — reuse them or extract them if they don't exist yet.

- [ ] **Step 3: Remove all `security_pipelines_for_language`, `scalability_pipelines_for_language`, `tech_debt_pipelines_for_language`, etc. function calls** — these are now unreachable. Delete the dead code.

- [ ] **Step 4: Build and fix**

```bash
cargo build 2>&1 | head -60
```

Fix all compile errors. The main task is removing any remaining references to the deleted enum variants.

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 6: Smoke test the CLI manually**

```bash
# Build release binary
cargo build --release 2>&1 | tail -5

# Test flat audit command
./target/release/virgil audit --dir . --language rs --category architecture 2>&1 | head -20

# Test category filter
./target/release/virgil audit --dir . --language rs --category security 2>&1 | head -20

# Test pipeline filter
./target/release/virgil audit --dir . --language rs --pipeline "circular_dependencies_rust" 2>&1 | head -20

# Verify help text
./target/release/virgil audit --help
```

Expected: audit command runs and produces output. Help text shows flat flags (no subcommands).

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "refactor(main): replace 6-arm audit dispatch with single flat Commands::Audit arm"
```

---

### Task 12: Final cleanup and verification

**Files:**
- Check: `src/audit/pipeline.rs` (the `AuditEngine` reference cleanup)
- Check: any remaining imports of deleted types

- [ ] **Step 1: Search for any remaining references to deleted types**

```bash
grep -rn "PipelineSelector\|AuditCommand\|CodeQualityCommand\|CouplingAnalyzer\|DeadExportsAnalyzer\|DuplicateSymbolsAnalyzer\|TaintEngine\|security_pipelines_for_language\|scalability_pipelines_for_language" src/ tests/
```

Expected: no matches. Fix any found.

- [ ] **Step 2: Search for references to deleted pipeline files**

```bash
grep -rn "sql_injection\|xss_dom_injection\|ssrf_open_redirect\|java_ssrf\|csharp_ssrf" src/
```

Expected: no matches (only JSON filenames, which are fine).

- [ ] **Step 3: Run full test suite**

```bash
cargo test 2>&1
```

Expected: all tests pass, no warnings about unused imports.

- [ ] **Step 4: Check binary size change**

```bash
cargo build --release 2>&1 | tail -3
ls -lh target/release/virgil
```

Note the size reduction (informational only).

- [ ] **Step 5: Update `CLAUDE.md`** — in the "Non-obvious Implementation Notes" section, remove references to `taint.rs` and `PipelineSelector`. Add a note:

```markdown
**Audit pipeline model (JSON-first)**
All audit pipelines are JSON-driven. The `taint` GraphStage handles security analysis
(SQL injection, SSRF, XSS, XXE) — sources/sinks/sanitizers are declared in JSON builtin
files. The `find_duplicates` stage and `efferent_coupling`/`afferent_coupling` compute
metrics handle cross-file analysis. No `PipelineSelector` enum exists; use
`AuditEngine::categories()` to filter by category.

**Audit CLI (flat)**
`virgil audit [--dir|--s3] [--language] [--category] [--pipeline] [--format] [--per-page] [--page]`
No nested subcommands. Category values: security, architecture, code_style, tech_debt,
complexity, scalability.
```

- [ ] **Step 6: Run tests one final time**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 7: Final commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude.md): update to reflect JSON-first audit model and flat CLI"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Task |
|---|---|
| Taint stage with JSON sources/sinks/sanitizers | Task 1 (structs), Task 2 (refactor taint.rs), Task 3 (execute_taint) |
| find_duplicates stage | Task 1 (structs), Task 3 (execute_find_duplicates) |
| efferent/afferent coupling metrics | Task 3 |
| unreferenced / is_entry_point predicates | Task 1, Task 3 |
| 18 security JSON pipeline files | Tasks 4, 5 |
| coupling.json / dead_exports.json / duplicate_symbols.json | Task 6 |
| Delete 15 Rust security pipeline files | Task 7 |
| Delete 3 Rust analyzers | Task 8 |
| Delete taint.rs | Task 8 |
| Delete audit_plans/ | Task 8 |
| Remove PipelineSelector | Task 9 |
| Flat audit CLI | Task 10 |
| Simplify main.rs dispatch | Task 11 |
| Update CLAUDE.md | Task 12 |

**Placeholder scan:** No TBDs found. All code blocks contain complete implementations. JSON files have concrete patterns.

**Type consistency:**
- `TaintStage` defined in Task 1 `pipeline.rs` → imported in Task 3 `executor.rs` ✓
- `TaintConfig` defined in Task 2 `taint.rs` → used in Task 3 `execute_taint` ✓
- `FindDuplicatesStage` defined in Task 1 → used in Task 3 `execute_find_duplicates` ✓
- `MetricValue::Text/Int/Float` used consistently across Tasks 3, 6 ✓
- `AuditEngine::categories()` defined in Task 9 → called in Task 11 `main.rs` ✓
- `PipelineSelector` deleted in Task 9 → tests updated in Task 9 before main.rs rewrite ✓
