# Pipeline Module Extraction + DSL Composability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the JSON pipeline layer into `src/pipeline/`, delete all dead Rust pipeline infrastructure, replace hardcoded metric fields in `WhereClause` with a generic `metrics` map, and decompose the monolithic `taint` stage into composable `taint_sources` / `taint_sanitizers` / `taint_sinks` stages.

**Architecture:** Create `src/pipeline/` as the single owner of the JSON DSL (`dsl.rs`), executor (`executor.rs`), and file loader (`loader.rs`). `audit/` becomes orchestration + output only. `graph/` becomes graph data structures + builder only. All JSON builtin files are migrated to the new composable DSL.

**Tech Stack:** Rust, Cargo, serde_json, tree-sitter, petgraph. Python 3 for JSON migration scripts.

---

## File Map

**Created:**
- `src/pipeline/mod.rs` — module root + re-exports
- `src/pipeline/helpers.rs` — `is_test_file`, `is_barrel_file`, `is_excluded_for_arch_analysis` (trimmed from `audit/pipelines/helpers.rs`)
- `src/pipeline/dsl.rs` — DSL structs (moved from `graph/pipeline.rs`, updated `WhereClause`)
- `src/pipeline/executor.rs` — execution engine (moved from `graph/executor.rs`, updated imports)
- `src/pipeline/loader.rs` — JSON file loading (moved from `audit/json_audit.rs`, updated imports)

**Modified:**
- `src/lib.rs` — add `pub mod pipeline`, remove old re-exports
- `src/graph/mod.rs` — remove `pub mod executor` and `pub mod pipeline`
- `src/graph/taint.rs` — update import path for taint pattern types
- `src/audit/mod.rs` — remove dead modules, remove `json_audit`
- `src/audit/engine.rs` — update import paths
- `src/main.rs` — update import paths
- `src/query_engine.rs` — update import paths
- `src/query_lang.rs` — update import path for `GraphStage`
- `src/server.rs` — replace `audit::pipeline::supported_*_languages()` with `Language::all().to_vec()`
- `src/pipeline/dsl.rs` — replace 11 named metric fields with `metrics: HashMap<String, NumericPredicate>`; add `TaintSources`, `TaintSanitizers`, `TaintSinks` variants to `GraphStage`
- `src/pipeline/executor.rs` — add `TaintContext`, handle new stages, desugar old `taint`
- `src/audit/builtin/*.json` — migrate metric fields under `metrics` key; decompose `taint` stages

**Deleted:**
- `src/graph/pipeline.rs` — moved to `src/pipeline/dsl.rs`
- `src/graph/executor.rs` — moved to `src/pipeline/executor.rs`
- `src/audit/json_audit.rs` — moved to `src/pipeline/loader.rs`
- `src/audit/pipeline.rs` — dead legacy trait hierarchy
- `src/audit/pipelines/` — dead per-language Rust pipeline implementations
- `src/audit/analyzers/` — empty stubs
- `src/audit/project_analyzer.rs` — dead trait
- `src/audit/primitives.rs` — dead helpers

---

## Task 1: Create `src/pipeline/` module and update all import paths

**Files:**
- Create: `src/pipeline/mod.rs`
- Create: `src/pipeline/helpers.rs`
- Create: `src/pipeline/dsl.rs`
- Create: `src/pipeline/executor.rs`
- Create: `src/pipeline/loader.rs`
- Modify: `src/lib.rs`
- Modify: `src/graph/mod.rs`
- Modify: `src/graph/taint.rs`
- Modify: `src/audit/mod.rs`
- Modify: `src/audit/engine.rs`
- Modify: `src/main.rs`
- Modify: `src/query_engine.rs`
- Modify: `src/query_lang.rs`
- Modify: `src/server.rs`
- Delete: `src/graph/pipeline.rs`, `src/graph/executor.rs`, `src/audit/json_audit.rs`

- [ ] **Step 1: Create `src/pipeline/helpers.rs`**

Copy only the three functions needed by the executor (the rest are used only by the dead Rust pipelines being deleted in Task 2):

```rust
// src/pipeline/helpers.rs

/// Returns true if the file path indicates test code (language-agnostic).
pub fn is_test_file(file_path: &str) -> bool {
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    if file_name.ends_with("_test.rs") || file_name.ends_with("_test.go") { return true; }
    if (file_name.starts_with("test_") && file_name.ends_with(".py"))
        || file_name.ends_with("_test.py")
        || file_name == "conftest.py" { return true; }
    if file_name.ends_with("Test.java") || file_name.ends_with("Tests.java") || file_name.ends_with("Spec.java") { return true; }
    if file_name.ends_with("Tests.cs") || file_name.ends_with("Test.cs") || file_name.ends_with("Spec.cs") { return true; }
    if file_name.ends_with("Test.php") { return true; }
    if file_name.ends_with("_test.cpp") || file_name.ends_with("_test.cc") || file_name.ends_with("_unittest.cpp") { return true; }
    if file_name.ends_with("Test.cpp") && file_name.len() > "Test.cpp".len() { return true; }
    if (file_name.starts_with("test_") && file_name.ends_with(".cpp"))
        || (file_name.starts_with("test_") && file_name.ends_with(".cc")) { return true; }
    let lower = file_name.to_lowercase();
    if lower.contains(".test.") || lower.contains(".spec.") { return true; }
    let path = file_path.replace('\\', "/");
    path.contains("/tests/") || path.starts_with("tests/")
        || path.contains("/test/") || path.starts_with("test/")
        || path.contains("/__tests__/") || path.starts_with("__tests__/")
        || path.contains("/testing/") || path.starts_with("testing/")
        || path.contains("/testdata/") || path.starts_with("testdata/")
}

/// Returns true if the file should be excluded from cross-file architecture analysis.
pub fn is_excluded_for_arch_analysis(path: &str) -> bool {
    if is_test_file(path) { return true; }
    let p = path.replace('\\', "/");
    if p.ends_with(".pb.go") || p.ends_with("_gen.go") || p.ends_with("_generated.go")
        || p.ends_with(".pb.h") || p.ends_with(".pb.cc")
        || p.contains("/generated/") || p.starts_with("generated/") { return true; }
    p.contains("/vendor/") || p.starts_with("vendor/")
        || p.contains("/third_party/") || p.starts_with("third_party/")
        || p.contains("/node_modules/") || p.starts_with("node_modules/")
        || p.contains("/_deps/") || p.starts_with("_deps/")
}

/// Returns true if the file is a barrel / re-export aggregator by name.
pub fn is_barrel_file(path: &str) -> bool {
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    matches!(
        file_name,
        "index.ts" | "index.tsx" | "index.js" | "index.jsx" | "__init__.py" | "mod.rs"
    )
}
```

- [ ] **Step 2: Create `src/pipeline/dsl.rs`**

Copy `src/graph/pipeline.rs` to `src/pipeline/dsl.rs` verbatim, then prepend this module doc comment at the top of the file (before all `use` statements):

```rust
//! JSON audit pipeline DSL.
//!
//! A pipeline is a `Vec<GraphStage>`. Stages compose left-to-right:
//! `select` → `compute_metric` / `taint_sources` / `taint_sanitizers` / `taint_sinks` → `flag`.
//! Each stage reads from and writes to a shared `Vec<PipelineNode>` carried through the run.
```

No other changes to the file content in this step.

- [ ] **Step 3: Create `src/pipeline/executor.rs`**

Copy `src/graph/executor.rs` to `src/pipeline/executor.rs`, then make these import changes at the top of the file:

Replace:
```rust
use crate::audit::pipelines::helpers::{
    is_barrel_file, is_excluded_for_arch_analysis, is_test_file,
};
use crate::graph::pipeline::{
    EdgeType, GraphStage, MetricValue, PipelineNode, interpolate_message,
};
```

With:
```rust
use crate::pipeline::helpers::{
    is_barrel_file, is_excluded_for_arch_analysis, is_test_file,
};
use crate::pipeline::dsl::{
    EdgeType, GraphStage, MetricValue, PipelineNode, interpolate_message,
};
```

Also replace all remaining fully-qualified `crate::graph::pipeline::` references within the file body:

```bash
# Run from repo root to find remaining references after the top-level import change:
grep -n "crate::graph::pipeline::" src/pipeline/executor.rs
```

Replace each `crate::graph::pipeline::` occurrence with `crate::pipeline::dsl::`.

- [ ] **Step 4: Create `src/pipeline/loader.rs`**

Copy `src/audit/json_audit.rs` to `src/pipeline/loader.rs`, then:

1. Replace the import at the top:
```rust
// Remove:
use crate::graph::pipeline::GraphStage;
// Add:
use crate::pipeline::dsl::GraphStage;
```

2. Prepend this module doc comment before the `use` lines:
```rust
//! JSON audit file loading and discovery.
//!
//! Discovery order: project-local (`.virgil/audits/`) → user-global (`~/.virgil-cli/audits/`) → built-ins.
//! Files with the same pipeline name AND the same language filter deduplicate (project-local wins).
//! Files with the same pipeline name but different language filters are all included (per-language variants).
```

- [ ] **Step 5: Create `src/pipeline/mod.rs`**

```rust
//! JSON pipeline layer: DSL, execution engine, and audit file loading.

pub mod dsl;
pub mod executor;
pub mod helpers;
pub mod loader;

pub use dsl::{
    EdgeDirection, EdgeType, FlagConfig, FindDuplicatesStage, GraphStage, MetricValue,
    NodeType, NumericPredicate, PipelineNode, SeverityEntry, TaintSanitizerPattern,
    TaintSinkPattern, TaintSourcePattern, TaintStage, WhereClause, interpolate_message,
};
pub use executor::{PipelineOutput, run_pipeline};
pub use loader::{JsonAuditFile, discover_json_audits};
```

- [ ] **Step 6: Update `src/lib.rs`**

Replace:
```rust
pub use audit::json_audit;
pub use graph::executor;
pub use graph::pipeline;
```

With:
```rust
pub mod pipeline;
```

- [ ] **Step 7: Update `src/graph/mod.rs`**

Remove these two lines:
```rust
pub mod executor;
pub mod pipeline;
```

- [ ] **Step 8: Update `src/graph/taint.rs`**

Replace:
```rust
use crate::graph::pipeline::{TaintSanitizerPattern, TaintSinkPattern, TaintSourcePattern};
```

With:
```rust
use crate::pipeline::dsl::{TaintSanitizerPattern, TaintSinkPattern, TaintSourcePattern};
```

- [ ] **Step 9: Update `src/audit/mod.rs`**

Replace the entire file:
```rust
pub mod engine;
pub mod format;
pub mod models;
pub mod pipeline;
pub mod pipelines;
pub mod primitives;
pub mod project_analyzer;
pub mod project_index;
pub mod analyzers;
```

With (remove `json_audit` only — dead modules stay until Task 2):
```rust
pub mod analyzers;
pub mod engine;
pub mod format;
pub mod models;
pub mod pipeline;
pub mod pipelines;
pub mod primitives;
pub mod project_analyzer;
pub mod project_index;
```

- [ ] **Step 10: Update `src/audit/engine.rs`**

Replace:
```rust
crate::audit::json_audit::discover_json_audits(
```
With:
```rust
crate::pipeline::loader::discover_json_audits(
```

Replace:
```rust
crate::graph::executor::run_pipeline(
```
With:
```rust
crate::pipeline::executor::run_pipeline(
```

Replace:
```rust
Ok(crate::graph::executor::PipelineOutput::Findings(new_findings)) => {
```
With:
```rust
Ok(crate::pipeline::executor::PipelineOutput::Findings(new_findings)) => {
```

Replace:
```rust
Ok(crate::graph::executor::PipelineOutput::Results(_)) => {
```
With:
```rust
Ok(crate::pipeline::executor::PipelineOutput::Results(_)) => {
```

- [ ] **Step 11: Update `src/query_engine.rs`**

Run:
```bash
grep -n "crate::graph::executor\|crate::graph::pipeline" src/query_engine.rs
```

For each occurrence replace the prefix:
- `crate::graph::executor::` → `crate::pipeline::executor::`
- `crate::graph::pipeline::` → `crate::pipeline::dsl::`

- [ ] **Step 12: Update `src/query_lang.rs`**

Replace:
```rust
pub graph: Option<Vec<crate::graph::pipeline::GraphStage>>,
```

With:
```rust
pub graph: Option<Vec<crate::pipeline::dsl::GraphStage>>,
```

- [ ] **Step 13: Update `src/main.rs`**

Run:
```bash
grep -n "graph::executor\|audit::json_audit\|graph::pipeline" src/main.rs
```

For each occurrence:
- `virgil_cli::graph::executor::run_pipeline` → `virgil_cli::pipeline::executor::run_pipeline`
- `virgil_cli::graph::executor::PipelineOutput` → `virgil_cli::pipeline::executor::PipelineOutput`
- `virgil_cli::audit::json_audit::JsonAuditFile` → `virgil_cli::pipeline::loader::JsonAuditFile`
- `virgil_cli::audit::models::AuditFinding` → unchanged (stays in audit)
- `virgil_cli::audit::models::AuditSummary` → unchanged

- [ ] **Step 14: Update `src/server.rs`**

The server calls five `audit::pipeline::supported_*_languages()` functions that are dead code (all return the same all-languages list). Replace all occurrences with `Language::all().to_vec()`.

Run to see all call sites:
```bash
grep -n "audit::pipeline::supported" src/server.rs
```

Each call like:
```rust
filter_languages(user_languages, audit::pipeline::supported_audit_languages())
filter_languages(user_languages, audit::pipeline::supported_complexity_languages())
filter_languages(user_languages, audit::pipeline::supported_code_style_languages())
filter_languages(user_languages, audit::pipeline::supported_security_languages())
filter_languages(user_languages, audit::pipeline::supported_scalability_languages())
```

Becomes:
```rust
filter_languages(user_languages, Language::all().to_vec())
```

(Keep the `filter_languages` call — just replace the second argument. There are multiple call sites; apply to all of them.)

- [ ] **Step 15: Verify no remaining stale references**

```bash
grep -rn "graph::executor\|graph::pipeline\|audit::json_audit" src/ --include="*.rs"
```

Expected: zero matches. If any remain, fix them before proceeding.

- [ ] **Step 16: Delete the source files that were moved**

```bash
rm src/graph/pipeline.rs src/graph/executor.rs src/audit/json_audit.rs
```

- [ ] **Step 17: Run the test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass. The test suite in `src/pipeline/dsl.rs` (was `graph/pipeline.rs`) and `src/pipeline/loader.rs` (was `audit/json_audit.rs`) should run unchanged.

- [ ] **Step 18: Commit**

```bash
git add src/pipeline/ src/lib.rs src/graph/mod.rs src/graph/taint.rs \
        src/audit/mod.rs src/audit/engine.rs src/main.rs \
        src/query_engine.rs src/query_lang.rs src/server.rs
git commit -m "refactor: extract pipeline/ module (dsl, executor, loader)"
```

---

## Task 2: Delete dead Rust pipeline infrastructure

**Files:**
- Delete: `src/audit/pipeline.rs`, `src/audit/pipelines/`, `src/audit/analyzers/`, `src/audit/project_analyzer.rs`, `src/audit/primitives.rs`
- Modify: `src/audit/mod.rs`

- [ ] **Step 1: Delete the dead files and directories**

```bash
rm src/audit/pipeline.rs
rm src/audit/project_analyzer.rs
rm src/audit/primitives.rs
rm -rf src/audit/pipelines/
rm -rf src/audit/analyzers/
```

- [ ] **Step 2: Update `src/audit/mod.rs`**

Replace the entire file content with:
```rust
pub mod engine;
pub mod format;
pub mod models;
pub mod project_index;
```

- [ ] **Step 3: Verify no remaining references to deleted modules**

```bash
grep -rn "audit::pipeline\|audit::pipelines\|audit::analyzers\|audit::project_analyzer\|audit::primitives" src/ --include="*.rs"
```

Expected: zero matches.

- [ ] **Step 4: Run the test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/audit/
git commit -m "chore: delete dead Rust pipeline infrastructure (audit/pipeline.rs, audit/pipelines/, analyzers, project_analyzer, primitives)"
```

---

## Task 3: Generic metrics map in `WhereClause` + JSON migration

**Files:**
- Modify: `src/pipeline/dsl.rs` — replace 11 named metric fields with `metrics: HashMap<String, NumericPredicate>`
- Modify: `src/audit/builtin/*.json` — migrate `severity_map.when` and `ratio.threshold` clauses

- [ ] **Step 1: Write the failing test**

Add this test to the `#[cfg(test)] mod tests` block in `src/pipeline/dsl.rs` (after the existing tests):

```rust
#[test]
fn test_where_clause_generic_metrics_deserialization() {
    let json = r#"{"metrics": {"cyclomatic_complexity": {"gte": 10}, "function_length": {"gt": 50}}}"#;
    let wc: WhereClause = serde_json::from_str(json).unwrap();
    assert!(!wc.metrics.is_empty());
    assert!(wc.metrics.contains_key("cyclomatic_complexity"));
    assert!(wc.metrics.contains_key("function_length"));
}

#[test]
fn test_where_clause_generic_metrics_eval() {
    let node_pass = make_node(vec![
        ("cyclomatic_complexity", MetricValue::Int(15)),
        ("function_length", MetricValue::Int(60)),
    ]);
    let node_fail = make_node(vec![
        ("cyclomatic_complexity", MetricValue::Int(5)),
        ("function_length", MetricValue::Int(60)),
    ]);

    let json = r#"{"metrics": {"cyclomatic_complexity": {"gte": 10}}}"#;
    let wc: WhereClause = serde_json::from_str(json).unwrap();
    assert!(wc.eval_metrics(&node_pass));
    assert!(!wc.eval_metrics(&node_fail));
}

#[test]
fn test_where_clause_old_named_metric_fields_no_longer_exist() {
    // The old field names are gone — they parse as unknown fields (ignored by default) or error
    // This test verifies the metrics map is the only way to specify numeric predicates
    let json = r#"{"metrics": {"efferent_coupling": {"gte": 8}}}"#;
    let wc: WhereClause = serde_json::from_str(json).unwrap();
    assert!(wc.metrics.contains_key("efferent_coupling"));
    assert!(!wc.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail (the struct doesn't have `metrics` yet)**

```bash
cargo test test_where_clause_generic_metrics 2>&1 | tail -15
```

Expected: compile error — field `metrics` does not exist on type `WhereClause`.

- [ ] **Step 3: Update `WhereClause` in `src/pipeline/dsl.rs`**

Add `use std::collections::HashMap;` to the imports at the top of the file if not already present.

Replace the metric predicate fields in `WhereClause` (the block from `// Metric predicates (for severity_map "when" clauses)` through the last named metric field) with the `metrics` map:

Remove these fields:
```rust
// Metric predicates (for severity_map "when" clauses)
#[serde(default)]
pub count: Option<NumericPredicate>,
#[serde(default)]
pub cycle_size: Option<NumericPredicate>,
#[serde(default)]
pub depth: Option<NumericPredicate>,
#[serde(default)]
pub edge_count: Option<NumericPredicate>,
#[serde(default)]
pub ratio: Option<NumericPredicate>,

// Symbol kind filter (for select stage kind filtering per D-03)
// ... (kind stays)

// Compute-metric predicates (for severity_map when clauses)
#[serde(default)]
pub cyclomatic_complexity: Option<NumericPredicate>,
#[serde(default)]
pub function_length: Option<NumericPredicate>,
#[serde(default)]
pub cognitive_complexity: Option<NumericPredicate>,
#[serde(default)]
pub comment_to_code_ratio: Option<NumericPredicate>,

// Coupling predicates (populated after compute_metric: efferent/afferent_coupling)
#[serde(default)]
pub efferent_coupling: Option<NumericPredicate>,
#[serde(default)]
pub afferent_coupling: Option<NumericPredicate>,
```

Add in their place (after the `kind` field, before the `unreferenced` field):
```rust
/// Generic computed-metric predicates. Keys are any metric name produced by a
/// `compute_metric` stage (e.g. "cyclomatic_complexity", "efferent_coupling").
/// Any metric can be filtered without changing the Rust schema.
#[serde(default)]
pub metrics: HashMap<String, NumericPredicate>,
```

The `unreferenced` and `is_entry_point` fields stay unchanged (they are boolean flags, not numeric thresholds).

- [ ] **Step 4: Update `WhereClause::is_empty`**

Replace the 11 individual `is_none()` checks for the removed fields with a single `metrics.is_empty()` check. The updated method:

```rust
pub fn is_empty(&self) -> bool {
    self.and.is_none()
        && self.or.is_none()
        && self.not.is_none()
        && self.is_test_file.is_none()
        && self.is_generated.is_none()
        && self.is_barrel_file.is_none()
        && self.is_nolint.is_none()
        && self.exported.is_none()
        && self.kind.is_none()
        && self.unreferenced.is_none()
        && self.is_entry_point.is_none()
        && self.metrics.is_empty()
}
```

- [ ] **Step 5: Update `WhereClause::eval_metrics`**

Remove the 11 individual metric predicate checks (the blocks for `count`, `cycle_size`, `depth`, `edge_count`, `ratio`, `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`, `efferent_coupling`, `afferent_coupling`).

Add in their place (after the `exported` check, before the final `true`):
```rust
// Generic metric predicates — any key produced by compute_metric
for (metric_name, pred) in &self.metrics {
    if !pred.matches(node.metric_f64(metric_name)) {
        return false;
    }
}
```

- [ ] **Step 6: Update `WhereClause::eval`**

Same replacement as Step 5, applied to the `eval` method. Remove the 11 named metric blocks and replace with:
```rust
for (metric_name, pred) in &self.metrics {
    if !pred.matches(node.metric_f64(metric_name)) {
        return false;
    }
}
```

Place this block after the `kind` check and before the `unreferenced` check.

- [ ] **Step 7: Run the new tests to verify they pass**

```bash
cargo test test_where_clause_generic_metrics 2>&1 | tail -10
```

Expected: 3 tests pass.

- [ ] **Step 8: Write and run the JSON migration script**

Save this script as `scripts/migrate_metrics.py`:

```python
#!/usr/bin/env python3
"""Migrate WhereClause metric fields to the metrics map in builtin JSON audit files."""
import json
import sys
from pathlib import Path

METRIC_FIELDS = {
    'cyclomatic_complexity', 'function_length', 'cognitive_complexity',
    'comment_to_code_ratio', 'efferent_coupling', 'afferent_coupling',
    'count', 'cycle_size', 'depth', 'edge_count', 'ratio',
}

def migrate_where_clause(wc):
    if not isinstance(wc, dict):
        return wc
    metrics = {}
    result = {}
    for key, value in wc.items():
        if key in METRIC_FIELDS:
            metrics[key] = value
        elif key == 'and':
            result[key] = [migrate_where_clause(c) for c in value]
        elif key == 'or':
            result[key] = [migrate_where_clause(c) for c in value]
        elif key == 'not':
            result[key] = migrate_where_clause(value)
        else:
            result[key] = value
    if metrics:
        result['metrics'] = metrics
    return result

def migrate_flag(flag):
    result = dict(flag)
    if 'severity_map' in result:
        new_map = []
        for entry in result['severity_map']:
            e = dict(entry)
            if 'when' in e and e['when'] is not None:
                e['when'] = migrate_where_clause(e['when'])
            new_map.append(e)
        result['severity_map'] = new_map
    return result

def migrate_ratio_config(ratio):
    result = dict(ratio)
    if 'threshold' in result and result['threshold'] is not None:
        result['threshold'] = migrate_where_clause(result['threshold'])
    if 'numerator' in result and isinstance(result['numerator'], dict):
        num = dict(result['numerator'])
        if 'where' in num and num['where'] is not None:
            num['where'] = migrate_where_clause(num['where'])
        result['numerator'] = num
    if 'denominator' in result and isinstance(result['denominator'], dict):
        den = dict(result['denominator'])
        if 'where' in den and den['where'] is not None:
            den['where'] = migrate_where_clause(den['where'])
        result['denominator'] = den
    return result

def migrate_stage(stage):
    result = {}
    for key, value in stage.items():
        if key == 'flag':
            result[key] = migrate_flag(value)
        elif key == 'ratio':
            result[key] = migrate_ratio_config(value)
        elif key == 'where':
            result[key] = migrate_where_clause(value)
        elif key == 'exclude':
            result[key] = migrate_where_clause(value)
        else:
            result[key] = value
    return result

def migrate_file(path):
    with open(path) as f:
        data = json.load(f)
    if 'graph' in data:
        data['graph'] = [migrate_stage(s) for s in data['graph']]
    with open(path, 'w') as f:
        json.dump(data, f, indent=2)
        f.write('\n')

if __name__ == '__main__':
    builtin_dir = Path('src/audit/builtin')
    files = sorted(builtin_dir.glob('*.json'))
    for p in files:
        migrate_file(p)
    print(f"Migrated {len(files)} files.")
```

Run it:
```bash
python3 scripts/migrate_metrics.py
```

Expected output: `Migrated N files.` (N ≈ 230)

- [ ] **Step 9: Spot-check migration output**

```bash
# cyclomatic_complexity.json should have metrics key in when clauses
grep -A2 '"when"' src/audit/builtin/cyclomatic_complexity.json | head -20

# coupling.json should have metrics.efferent_coupling
grep -A3 '"when"' src/audit/builtin/coupling.json | head -15

# module_size_distribution_rust.json should have metrics.count
grep -A3 '"when"' src/audit/builtin/module_size_distribution_rust.json | head -15

# api_surface_area_rust.json should have metrics.count and metrics.ratio in threshold
cat src/audit/builtin/api_surface_area_rust.json
```

Expected for `cyclomatic_complexity.json`:
```json
"when": {
  "metrics": { "cyclomatic_complexity": {"gte": 30} }
}
```

- [ ] **Step 10: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass. The `test_builtin_audits_returns_four` test in `src/pipeline/loader.rs` verifies all builtins parse successfully.

- [ ] **Step 11: Commit**

```bash
git add src/pipeline/dsl.rs src/audit/builtin/ scripts/migrate_metrics.py
git commit -m "feat(dsl): replace named metric fields with generic metrics map; migrate builtin JSON files"
```

---

## Task 4: Taint stage decomposition

**Files:**
- Modify: `src/pipeline/dsl.rs` — add `TaintSources`, `TaintSanitizers`, `TaintSinks` variants to `GraphStage`
- Modify: `src/pipeline/executor.rs` — add `TaintContext`, implement new stage handlers, desugar old `taint`
- Modify: `src/audit/builtin/sql_injection_*.json`, `ssrf_*.json`, `xss_*.json`, `xxe_*.json` (20 files)

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/pipeline/dsl.rs`:

```rust
#[test]
fn test_taint_sources_stage_deserializes() {
    let json = r#"{"taint_sources": [{"pattern": "request.form", "kind": "user_input"}]}"#;
    let stage: GraphStage = serde_json::from_str(json).unwrap();
    match stage {
        GraphStage::TaintSources { taint_sources } => {
            assert_eq!(taint_sources.len(), 1);
            assert_eq!(taint_sources[0].pattern, "request.form");
            assert_eq!(taint_sources[0].kind, "user_input");
        }
        _ => panic!("expected TaintSources stage"),
    }
}

#[test]
fn test_taint_sanitizers_stage_deserializes() {
    let json = r#"{"taint_sanitizers": [{"pattern": "escape"}, {"pattern": "quote"}]}"#;
    let stage: GraphStage = serde_json::from_str(json).unwrap();
    match stage {
        GraphStage::TaintSanitizers { taint_sanitizers } => {
            assert_eq!(taint_sanitizers.len(), 2);
        }
        _ => panic!("expected TaintSanitizers stage"),
    }
}

#[test]
fn test_taint_sinks_stage_deserializes() {
    let json = r#"{"taint_sinks": [{"pattern": "cursor.execute", "vulnerability": "sql_injection"}]}"#;
    let stage: GraphStage = serde_json::from_str(json).unwrap();
    match stage {
        GraphStage::TaintSinks { taint_sinks } => {
            assert_eq!(taint_sinks.len(), 1);
            assert_eq!(taint_sinks[0].vulnerability, "sql_injection");
        }
        _ => panic!("expected TaintSinks stage"),
    }
}

#[test]
fn test_decomposed_taint_pipeline_deserializes() {
    let json = r#"[
        {"taint_sources": [{"pattern": "request.form", "kind": "user_input"}]},
        {"taint_sanitizers": [{"pattern": "escape"}]},
        {"taint_sinks": [{"pattern": "cursor.execute", "vulnerability": "sql_injection"}]},
        {"flag": {"pattern": "sql_injection", "message": "found at {{file}}:{{line}}", "severity": "error"}}
    ]"#;
    let stages: Vec<GraphStage> = serde_json::from_str(json).unwrap();
    assert_eq!(stages.len(), 4);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_taint_sources_stage test_taint_sanitizers_stage test_taint_sinks_stage test_decomposed_taint_pipeline 2>&1 | tail -10
```

Expected: compile error — variants `TaintSources`, `TaintSanitizers`, `TaintSinks` do not exist.

- [ ] **Step 3: Add new variants to `GraphStage` in `src/pipeline/dsl.rs`**

In the `GraphStage` enum, add three new variants after the existing `Taint` variant:

```rust
TaintSources {
    taint_sources: Vec<TaintSourcePattern>,
},
TaintSanitizers {
    taint_sanitizers: Vec<TaintSanitizerPattern>,
},
TaintSinks {
    taint_sinks: Vec<TaintSinkPattern>,
},
```

The full enum (just the new additions shown in context):
```rust
Taint {
    taint: TaintStage,
},
TaintSources {
    taint_sources: Vec<TaintSourcePattern>,
},
TaintSanitizers {
    taint_sanitizers: Vec<TaintSanitizerPattern>,
},
TaintSinks {
    taint_sinks: Vec<TaintSinkPattern>,
},
FindDuplicates {
    find_duplicates: FindDuplicatesStage,
},
```

- [ ] **Step 4: Run the DSL tests to verify they pass**

```bash
cargo test test_taint_sources_stage test_taint_sanitizers_stage test_taint_sinks_stage test_decomposed_taint_pipeline 2>&1 | tail -10
```

Expected: 4 tests pass. (The executor won't handle the new variants yet — it will panic/error at runtime, but the DSL tests don't invoke the executor.)

- [ ] **Step 5: Add `TaintContext` to `src/pipeline/executor.rs`**

Find the section near the top of `executor.rs` where `PipelineOutput` is defined (around line 30). Add this struct immediately after `PipelineOutput`:

```rust
/// Accumulated taint configuration built up by `TaintSources` and `TaintSanitizers` stages.
/// Consumed when a `TaintSinks` stage executes.
#[derive(Default)]
struct TaintContext {
    sources: Vec<crate::pipeline::dsl::TaintSourcePattern>,
    sanitizers: Vec<crate::pipeline::dsl::TaintSanitizerPattern>,
}
```

- [ ] **Step 6: Initialize `TaintContext` in `run_pipeline` and thread it through**

In the `run_pipeline` function, find where the stage loop begins. Add `TaintContext` initialization before the loop:

```rust
let mut taint_ctx = TaintContext::default();
```

The `taint_ctx` variable must be declared in the same scope as the stage loop so it persists between stage iterations.

- [ ] **Step 7: Add handlers for the three new stages in `run_pipeline`**

In the `match stage` block inside `run_pipeline`, find the existing `GraphStage::Taint { taint }` arm. Add three new arms immediately after it:

```rust
GraphStage::TaintSources { taint_sources } => {
    taint_ctx.sources.extend(taint_sources.iter().cloned());
    // nodes unchanged — sources are accumulated into context, not applied yet
}
GraphStage::TaintSanitizers { taint_sanitizers } => {
    taint_ctx.sanitizers.extend(taint_sanitizers.iter().cloned());
    // nodes unchanged
}
GraphStage::TaintSinks { taint_sinks } => {
    // Run taint analysis using the context accumulated by prior TaintSources/TaintSanitizers stages
    let config = crate::graph::taint::TaintConfig {
        sources: taint_ctx.sources.clone(),
        sinks: taint_sinks.clone(),
        sanitizers: taint_ctx.sanitizers.clone(),
    };
    nodes = execute_taint_with_config(&config, graph, workspace, lang_filter, pipeline_name)?;
}
```

The existing `GraphStage::Taint { taint }` arm should be refactored to call the same underlying helper (see Step 8).

- [ ] **Step 8: Extract `execute_taint_with_config` helper and update the existing `Taint` arm**

Find the existing function `execute_taint` (around line 960 in `executor.rs`) which takes `stage: &crate::pipeline::dsl::TaintStage`. Rename it to `execute_taint_with_config` and change its signature to accept a `TaintConfig` directly:

```rust
fn execute_taint_with_config(
    config: &crate::graph::taint::TaintConfig,
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    lang_filter: Option<&[String]>,
    pipeline_name: &str,
) -> Result<Vec<PipelineNode>> {
    // body unchanged — it already creates a TaintConfig internally;
    // update it to use the passed-in config instead of constructing one
}
```

Update the existing `GraphStage::Taint { taint }` arm to desugar and call the helper:

```rust
GraphStage::Taint { taint } => {
    // Desugar: inline sources/sanitizers/sinks into a TaintConfig and run directly.
    // This preserves backward compatibility for external pipeline files that use the old form.
    let config = crate::graph::taint::TaintConfig {
        sources: taint.sources.clone(),
        sinks: taint.sinks.clone(),
        sanitizers: taint.sanitizers.clone(),
    };
    nodes = execute_taint_with_config(&config, graph, workspace, lang_filter, pipeline_name)?;
}
```

- [ ] **Step 9: Run the test suite to verify the executor handles all new stages**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass. The existing taint-based builtin JSON files still use `taint` (old form) which now desugars — they should continue to produce findings.

- [ ] **Step 10: Write and run the taint migration script**

Save as `scripts/migrate_taint.py`:

```python
#!/usr/bin/env python3
"""Decompose monolithic taint stage into taint_sources + taint_sanitizers + taint_sinks."""
import json
from pathlib import Path

def migrate_taint_stage(stage):
    """If the stage is a taint stage, explode it into 3 stages. Otherwise return as-is."""
    if 'taint' not in stage:
        return [stage]
    taint = stage['taint']
    result = []
    if taint.get('sources'):
        result.append({'taint_sources': taint['sources']})
    if taint.get('sanitizers'):
        result.append({'taint_sanitizers': taint['sanitizers']})
    if taint.get('sinks'):
        result.append({'taint_sinks': taint['sinks']})
    return result

def migrate_file(path):
    with open(path) as f:
        data = json.load(f)
    if 'graph' not in data:
        return False
    new_graph = []
    changed = False
    for stage in data['graph']:
        replacement = migrate_taint_stage(stage)
        new_graph.extend(replacement)
        if len(replacement) != 1 or replacement[0] is not stage:
            changed = True
    if changed:
        data['graph'] = new_graph
        with open(path, 'w') as f:
            json.dump(data, f, indent=2)
            f.write('\n')
    return changed

if __name__ == '__main__':
    builtin_dir = Path('src/audit/builtin')
    count = 0
    for p in sorted(builtin_dir.glob('*.json')):
        if migrate_file(p):
            count += 1
            print(f"  Migrated: {p.name}")
    print(f"\nMigrated {count} files.")
```

Run it:
```bash
python3 scripts/migrate_taint.py
```

Expected output: lists ~20 migrated files (all `sql_injection_*.json`, `ssrf_*.json`, `xss_*.json`, `xxe_*.json`).

- [ ] **Step 11: Spot-check a migrated file**

```bash
cat src/audit/builtin/sql_injection_python.json
```

Expected structure:
```json
{
  "pipeline": "sql_injection_python",
  "category": "security",
  "graph": [
    {"taint_sources": [{"pattern": "request.form", "kind": "user_input"}, ...]},
    {"taint_sanitizers": [{"pattern": "escape"}, ...]},
    {"taint_sinks": [{"pattern": "cursor.execute", "vulnerability": "sql_injection"}, ...]},
    {"flag": {"pattern": "sql_injection", "message": "...", "severity": "error"}}
  ]
}
```

- [ ] **Step 12: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass. The `test_builtin_audits_returns_four` test verifies all migrated files still parse.

- [ ] **Step 13: Commit**

```bash
git add src/pipeline/dsl.rs src/pipeline/executor.rs src/audit/builtin/ scripts/migrate_taint.py
git commit -m "feat(dsl): decompose taint stage into taint_sources/taint_sanitizers/taint_sinks; migrate 20 builtin JSON files"
```

---

## Task 5: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update the module layout description in CLAUDE.md**

Find the section describing the codebase structure. Replace the module descriptions to reflect the new layout.

The updated module descriptions (replace whatever currently describes `audit/` and `graph/`):

```markdown
## Module Layout

- `src/pipeline/` — JSON pipeline layer (single owner of the DSL, executor, and file loading)
  - `dsl.rs` — `GraphStage`, `WhereClause`, `PipelineNode` and all DSL types
  - `executor.rs` — `run_pipeline` execution engine
  - `loader.rs` — `discover_json_audits` (project-local → user-global → built-ins)
  - `helpers.rs` — `is_test_file`, `is_barrel_file`, `is_excluded_for_arch_analysis`
- `src/audit/` — orchestration and output only
  - `engine.rs` — `AuditEngine` (discovers + runs JSON pipelines, collects findings)
  - `format.rs` — finding output formatting (table/json/csv)
  - `models.rs` — `AuditFinding`, `AuditSummary`
  - `project_index.rs` — `ProjectIndex` (used by `graph/mod.rs` compat methods)
- `src/graph/` — graph data structures and builder
  - `mod.rs` — `CodeGraph`, `NodeWeight`, `EdgeWeight`
  - `builder.rs` — `GraphBuilder` (parses workspace into `CodeGraph`)
  - `taint.rs` — `TaintEngine`, `TaintConfig` (internal engine used by `pipeline/executor.rs`)
  - `metrics.rs` — metric computation (cyclomatic complexity, function length, etc.)
  - `cfg.rs` / `cfg_languages/` — control flow graph construction
```

- [ ] **Step 2: Update the Audit pipeline model section**

Find the `**Audit pipeline model (JSON-first)**` section. Replace it with:

```markdown
**Audit pipeline model (JSON-first)**
All audit logic is JSON-driven. `src/pipeline/` owns the DSL, executor, and builtin file loading.
`AuditEngine` in `src/audit/engine.rs` discovers JSON files and calls `run_pipeline`.
No Rust pipeline code exists — `audit/pipeline.rs`, `audit/pipelines/`, and the legacy trait
hierarchy (`Pipeline`, `NodePipeline`, `GraphPipeline`) have been deleted.

**DSL composability**
`WhereClause` uses a generic `metrics: HashMap<String, NumericPredicate>` field — any metric
computed by a `compute_metric` stage is filterable without changing the Rust schema:
  `{"when": {"metrics": {"cyclomatic_complexity": {"gte": 15}}}}`

The `taint` stage is decomposed into `taint_sources` + `taint_sanitizers` + `taint_sinks`
stages that accumulate into a shared context. The old combined `taint` form continues to work
(desugared by the executor) for backward compatibility with external pipeline files.
```

- [ ] **Step 3: Remove stale references**

Search for and remove any remaining mentions of:
- `PipelineSelector` (deleted in a prior refactor)
- `audit/pipelines/` or `audit/pipeline.rs`
- `Pipeline` trait, `NodePipeline`, `GraphPipeline`, `AnyPipeline`
- `pipelines_for_language`, `complexity_pipelines_for_language`, etc.
- `graph/pipeline.rs` or `graph/executor.rs` (now in `pipeline/`)

```bash
grep -n "PipelineSelector\|audit/pipelines\|NodePipeline\|GraphPipeline\|AnyPipeline\|pipelines_for_language\|graph/pipeline\|graph/executor" CLAUDE.md
```

Remove each matched line or update the surrounding paragraph.

- [ ] **Step 4: Run tests one final time**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude.md): update to reflect pipeline/ module, generic metrics map, decomposed taint stages"
```
