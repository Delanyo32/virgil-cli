# Benchmark Detection Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix missed-pattern detection (category bugs, nesting_depth metric, 8 new pipelines) and reduce false positives (6 JSON pipeline rewrites) to improve benchmark detection rates from 5–27% toward a trustworthy signal-to-noise ratio.

**Architecture:** Two Rust changes add `compute_nesting_depth` to `src/graph/metrics.rs` and wire it in `src/pipeline/executor.rs`. All other changes are JSON pipeline edits and additions in `src/audit/builtin/`. Tests span `src/pipeline/executor.rs` unit tests and `tests/audit_json_integration.rs` end-to-end tests.

**Tech Stack:** Rust, tree-sitter 0.25 (streaming_iterator API), serde_json, tempfile (test fixtures), `include_dir!` (builtin embedding)

---

### Task 1: Fix category in function_length.json and cyclomatic_complexity.json

**Files:**
- Modify: `src/audit/builtin/function_length.json`
- Modify: `src/audit/builtin/cyclomatic_complexity.json`
- Modify: `tests/audit_json_integration.rs` (add two tests)

- [ ] **Step 1: Write two failing integration tests**

Append to `tests/audit_json_integration.rs`:

```rust
// ── function_length (Rust) ──

#[test]
fn function_length_rust_finds_long_function() {
    let dir = tempfile::tempdir().unwrap();
    // 55 let-bindings = 57 lines including fn open/close, well above 50-line warning threshold
    let body: String = (0..55).map(|i| format!("    let _{i} = {i};\n")).collect();
    let content = format!("fn long_fn() {{\n{body}}}\n");
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .categories(vec!["complexity".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "function_too_long"),
        "expected function_too_long finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn function_length_rust_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = "fn short_fn() {\n    let x = 1;\n    let y = 2;\n    let _ = x + y;\n}\n";
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .categories(vec!["complexity".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "function_too_long"),
        "expected no function_too_long finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

// ── cyclomatic_complexity (Rust) ──

#[test]
fn cyclomatic_complexity_rust_finds_complex_function() {
    let dir = tempfile::tempdir().unwrap();
    // 10 nested ifs = CC 11, above the > 10 warning threshold
    let content = r#"fn complex(a: i32, b: i32, c: i32, d: i32) -> i32 {
    if a > 0 {
        if b > 0 {
            if c > 0 {
                if d > 0 {
                    if a > 1 {
                        if b > 1 {
                            if c > 1 {
                                if d > 1 {
                                    if a > 2 {
                                        if b > 2 { return 10; }
                                        return 9;
                                    }
                                    return 8;
                                }
                                return 7;
                            }
                            return 6;
                        }
                        return 5;
                    }
                    return 4;
                }
                return 3;
            }
            return 2;
        }
        return 1;
    }
    0
}
"#;
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .categories(vec!["complexity".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "high_cyclomatic_complexity"),
        "expected high_cyclomatic_complexity finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn cyclomatic_complexity_rust_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = "fn simple(x: i32) -> i32 { x + 1 }\n";
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .categories(vec!["complexity".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "high_cyclomatic_complexity"),
        "expected no high_cyclomatic_complexity finding"
    );
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

```bash
cargo test function_length_rust_finds_long_function cyclomatic_complexity_rust_finds_complex_function -- --nocapture 2>&1 | tail -20
```

Expected: FAIL — findings vec is empty because `"code-quality"` category never matches `"complexity"` filter.

- [ ] **Step 3: Fix the category strings in both JSON files**

In `src/audit/builtin/function_length.json`, change line 3:
```json
  "category": "complexity",
```

In `src/audit/builtin/cyclomatic_complexity.json`, change line 3:
```json
  "category": "complexity",
```

- [ ] **Step 4: Run all four tests and confirm they pass**

```bash
cargo test function_length_rust cyclomatic_complexity_rust -- --nocapture 2>&1 | tail -20
```

Expected: all 4 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/audit/builtin/function_length.json src/audit/builtin/cyclomatic_complexity.json tests/audit_json_integration.rs
git commit -m "fix(audit): change function_length and cyclomatic_complexity category from code-quality to complexity"
```

---

### Task 2: Add `compute_nesting_depth` to metrics.rs

**Files:**
- Modify: `src/graph/metrics.rs`
- Modify: `src/pipeline/executor.rs` (add unit test)

- [ ] **Step 1: Write a failing unit test in executor.rs**

In `src/pipeline/executor.rs`, inside the `#[cfg(test)]` module, append:

```rust
#[test]
fn test_compute_metric_nesting_depth_detects_deep_nesting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    // 4 levels of nesting: for > if > for > if
    std::fs::write(
        src_dir.join("lib.rs"),
        r#"fn deeply_nested(items: &[i32]) -> i32 {
    let mut sum = 0;
    for item in items {
        if *item > 0 {
            for _ in 0..*item {
                if *item > 5 {
                    sum += 1;
                }
            }
        }
    }
    sum
}
"#,
    )
    .unwrap();
    let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Rust], None).unwrap();

    let mut graph = CodeGraph::new();
    let file_idx = graph.graph.add_node(NodeWeight::File {
        path: "src/lib.rs".to_string(),
        language: Language::Rust,
    });
    graph.file_nodes.insert("src/lib.rs".to_string(), file_idx);
    let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
        name: "deeply_nested".to_string(),
        kind: crate::models::SymbolKind::Function,
        file_path: "src/lib.rs".to_string(),
        start_line: 1,
        end_line: 13,
        exported: false,
    });
    graph.symbol_nodes.insert(("src/lib.rs".to_string(), 1), sym_idx);

    let stages = vec![
        GraphStage::Select {
            select: crate::pipeline::dsl::NodeType::Symbol,
            filter: None,
            exclude: None,
        },
        GraphStage::ComputeMetric {
            compute_metric: "nesting_depth".to_string(),
        },
    ];
    let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "nesting_depth_test").unwrap();
    match out {
        PipelineOutput::Results(results) => {
            assert_eq!(results.len(), 1, "expected 1 result for deeply_nested function");
            let depth = results[0]
                .metrics
                .get("nesting_depth")
                .and_then(|v| if let MetricValue::Int(n) = v { Some(*n) } else { None })
                .expect("nesting_depth metric should be present");
            assert!(depth >= 4, "expected nesting_depth >= 4 for 4-level nesting, got {}", depth);
        }
        _ => panic!("expected Results (no Flag stage)"),
    }
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
cargo test test_compute_metric_nesting_depth -- --nocapture 2>&1 | tail -10
```

Expected: compile error or FAIL with "unknown metric 'nesting_depth'".

- [ ] **Step 3: Add `compute_nesting_depth` to metrics.rs**

In `src/graph/metrics.rs`, after the `compute_cognitive` function (line ~115), insert:

```rust
/// Compute maximum control flow nesting depth for a function body node.
///
/// Counts how deeply nested `nesting_increments` nodes are within each other.
/// Returns the maximum depth reached (0 = no nesting).
pub fn compute_nesting_depth(body: Node, config: &ControlFlowConfig) -> usize {
    let mut max_depth: usize = 0;
    let mut stack: Vec<(Node, usize)> = Vec::new();
    let mut cursor = body.walk();
    let children: Vec<_> = body.children(&mut cursor).collect();
    for child in children.into_iter().rev() {
        stack.push((child, 0));
    }
    while let Some((node, depth)) = stack.pop() {
        let kind = node.kind();
        let next_depth = if config.nesting_increments.contains(&kind) {
            let new_depth = depth + 1;
            if new_depth > max_depth {
                max_depth = new_depth;
            }
            new_depth
        } else {
            depth
        };
        let mut child_cursor = node.walk();
        let node_children: Vec<_> = node.children(&mut child_cursor).collect();
        for child in node_children.into_iter().rev() {
            stack.push((child, next_depth));
        }
    }
    max_depth
}
```

- [ ] **Step 4: Wire `nesting_depth` in executor.rs**

In `src/pipeline/executor.rs`, in `execute_compute_metric`, find the `match metric_name` block (around line 943). Replace the `other =>` arm with:

```rust
            "nesting_depth" => {
                crate::graph::metrics::compute_nesting_depth(body, &config) as i64
            }
            other => {
                anyhow::bail!(
                    "compute_metric: unknown metric '{}' -- supported: cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio, nesting_depth, efferent_coupling, afferent_coupling",
                    other
                );
            }
```

- [ ] **Step 5: Run the unit test and confirm it passes**

```bash
cargo test test_compute_metric_nesting_depth -- --nocapture 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/graph/metrics.rs src/pipeline/executor.rs
git commit -m "feat(metrics): add compute_nesting_depth metric and wire in executor"
```

---

### Task 3: Create 9 deep_nesting JSON pipelines + integration tests

**Files:**
- Delete+Replace: `src/audit/builtin/deep_nesting_python.json`
- Create: `src/audit/builtin/deep_nesting_rust.json`
- Create: `src/audit/builtin/deep_nesting_typescript.json`
- Create: `src/audit/builtin/deep_nesting_javascript.json`
- Create: `src/audit/builtin/deep_nesting_go.json`
- Create: `src/audit/builtin/deep_nesting_java.json`
- Create: `src/audit/builtin/deep_nesting_c.json`
- Create: `src/audit/builtin/deep_nesting_cpp.json`
- Create: `src/audit/builtin/deep_nesting_csharp.json`
- Modify: `src/pipeline/loader.rs` (update builtin count assertion from 36 → 44)
- Modify: `tests/audit_json_integration.rs` (add 2 tests)

- [ ] **Step 1: Write two failing integration tests**

Append to `tests/audit_json_integration.rs`:

```rust
// ── deep_nesting (Rust) ──

#[test]
fn deep_nesting_rust_finds_deeply_nested_function() {
    let dir = tempfile::tempdir().unwrap();
    // 4 levels: for > if > for > if — hits the gte:4 warning threshold
    let content = r#"fn deeply_nested(items: &[i32]) -> i32 {
    let mut sum = 0;
    for item in items {
        if *item > 0 {
            for _ in 0..*item {
                if *item > 5 {
                    sum += 1;
                }
            }
        }
    }
    sum
}
"#;
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .categories(vec!["complexity".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "excessive_nesting_depth"),
        "expected excessive_nesting_depth finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn deep_nesting_rust_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    // Only 1 level of nesting — under the threshold
    let content = "fn shallow(items: &[i32]) -> i32 {\n    let mut s = 0;\n    for i in items { s += i; }\n    s\n}\n";
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .categories(vec!["complexity".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "excessive_nesting_depth"),
        "expected no excessive_nesting_depth finding"
    );
}
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test deep_nesting_rust -- --nocapture 2>&1 | tail -10
```

Expected: FAIL — no findings because no `deep_nesting_rust.json` pipeline exists yet.

- [ ] **Step 3: Replace deep_nesting_python.json**

Overwrite `src/audit/builtin/deep_nesting_python.json` with:

```json
{
  "pipeline": "deep_nesting_python",
  "category": "complexity",
  "description": "Detects Python functions with excessive control flow nesting depth (if, for, while, with, try). Threshold: warning >= 4, error >= 6.",
  "languages": ["python"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 4: Create deep_nesting_rust.json**

Create `src/audit/builtin/deep_nesting_rust.json`:

```json
{
  "pipeline": "deep_nesting_rust",
  "category": "complexity",
  "description": "Detects Rust functions with excessive control flow nesting depth (if, for, while, loop, match, closure). Threshold: warning >= 4, error >= 6.",
  "languages": ["rust"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 5: Create deep_nesting_typescript.json**

Create `src/audit/builtin/deep_nesting_typescript.json`:

```json
{
  "pipeline": "deep_nesting_typescript",
  "category": "complexity",
  "description": "Detects TypeScript functions with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "languages": ["typescript"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method", "arrow_function"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 6: Create deep_nesting_javascript.json**

Create `src/audit/builtin/deep_nesting_javascript.json`:

```json
{
  "pipeline": "deep_nesting_javascript",
  "category": "complexity",
  "description": "Detects JavaScript functions with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "languages": ["javascript"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method", "arrow_function"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 7: Create deep_nesting_go.json**

Create `src/audit/builtin/deep_nesting_go.json`:

```json
{
  "pipeline": "deep_nesting_go",
  "category": "complexity",
  "description": "Detects Go functions with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "languages": ["go"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 8: Create deep_nesting_java.json**

Create `src/audit/builtin/deep_nesting_java.json`:

```json
{
  "pipeline": "deep_nesting_java",
  "category": "complexity",
  "description": "Detects Java methods with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "languages": ["java"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 9: Create deep_nesting_c.json**

Create `src/audit/builtin/deep_nesting_c.json`:

```json
{
  "pipeline": "deep_nesting_c",
  "category": "complexity",
  "description": "Detects C functions with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "languages": ["c"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 10: Create deep_nesting_cpp.json**

Create `src/audit/builtin/deep_nesting_cpp.json`:

```json
{
  "pipeline": "deep_nesting_cpp",
  "category": "complexity",
  "description": "Detects C++ functions with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "languages": ["cpp"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 11: Create deep_nesting_csharp.json**

Create `src/audit/builtin/deep_nesting_csharp.json`:

```json
{
  "pipeline": "deep_nesting_csharp",
  "category": "complexity",
  "description": "Detects C# methods with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "languages": ["csharp"],
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "excessive_nesting_depth",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

- [ ] **Step 12: Update loader builtin count assertion**

In `src/pipeline/loader.rs`, find the test `test_builtin_audits_returns_four` (line ~157). Change the assertion:

```rust
assert!(audits.len() >= 44, "Expected at least 44 built-in audits, got {}", audits.len());
```

Also update the comment on the next line from `// Per-language pipeline names (representative samples from the 36 built-ins)` to `// Per-language pipeline names (representative samples from the 44 built-ins)`.

- [ ] **Step 13: Run all deep_nesting tests and confirm they pass**

```bash
cargo test deep_nesting_rust test_builtin_audits_returns_four -- --nocapture 2>&1 | tail -15
```

Expected: all PASS.

- [ ] **Step 14: Commit**

```bash
git add src/audit/builtin/deep_nesting_python.json \
        src/audit/builtin/deep_nesting_rust.json \
        src/audit/builtin/deep_nesting_typescript.json \
        src/audit/builtin/deep_nesting_javascript.json \
        src/audit/builtin/deep_nesting_go.json \
        src/audit/builtin/deep_nesting_java.json \
        src/audit/builtin/deep_nesting_c.json \
        src/audit/builtin/deep_nesting_cpp.json \
        src/audit/builtin/deep_nesting_csharp.json \
        src/pipeline/loader.rs \
        tests/audit_json_integration.rs
git commit -m "feat(audit): add nesting_depth deep_nesting pipelines for all 9 languages"
```

---

### Task 4: Fix cpp_buffer_overflow_cpp.json (FP — fires on all calls)

**Files:**
- Modify: `src/audit/builtin/cpp_buffer_overflow_cpp.json`
- Modify: `tests/audit_json_integration.rs` (add 2 tests)

- [ ] **Step 1: Write two failing integration tests**

Append to `tests/audit_json_integration.rs`:

```rust
// ── cpp_buffer_overflow (C++) ──

#[test]
fn cpp_buffer_overflow_finds_strcpy() {
    let dir = tempfile::tempdir().unwrap();
    let content = r#"#include <string.h>
void copy_name(char *dst, const char *src) {
    strcpy(dst, src);
}
"#;
    std::fs::write(dir.path().join("util.cpp"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "buffer_overflow_risk"),
        "expected buffer_overflow_risk for strcpy; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_buffer_overflow_no_false_positive_on_safe_call() {
    let dir = tempfile::tempdir().unwrap();
    // snprintf and std::string are safe — should not fire
    let content = r#"#include <stdio.h>
#include <string>
void safe_fn(const std::string &s) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%s", s.c_str());
}
"#;
    std::fs::write(dir.path().join("safe.cpp"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "buffer_overflow_risk"),
        "expected no buffer_overflow_risk for safe code; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Run to confirm first test passes, second fails**

```bash
cargo test cpp_buffer_overflow -- --nocapture 2>&1 | tail -15
```

Expected: `cpp_buffer_overflow_finds_strcpy` likely PASS (current pipeline fires on all calls including strcpy). `cpp_buffer_overflow_no_false_positive_on_safe_call` FAIL (current pipeline fires on `snprintf` too).

- [ ] **Step 3: Rewrite cpp_buffer_overflow_cpp.json**

Overwrite `src/audit/builtin/cpp_buffer_overflow_cpp.json` with:

```json
{
  "pipeline": "cpp_buffer_overflow",
  "category": "security",
  "description": "Detects calls to known unsafe C/C++ buffer functions that lack bounds checking. Flags: strcpy, strcat, sprintf, vsprintf, gets, memcpy, memmove, wcscpy, wcscat, scanf, strdup.",
  "languages": ["cpp"],
  "graph": [
    {
      "match_pattern": "[(call_expression function: (identifier) @fn_name) (call_expression function: (field_expression field: (field_identifier) @fn_name)) (call_expression function: (qualified_identifier name: (identifier) @fn_name))] (#match? @fn_name \"^(strcpy|strcat|sprintf|vsprintf|gets|memcpy|memmove|wcscpy|wcscat|scanf|strdup)$\")"
    },
    {
      "flag": {
        "pattern": "buffer_overflow_risk",
        "message": "Call to unsafe buffer function `{{name}}` -- use bounds-checked alternatives: strncpy/strlcpy, snprintf, fgets, std::string",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 4: Run both tests and confirm they pass**

```bash
cargo test cpp_buffer_overflow -- --nocapture 2>&1 | tail -10
```

Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add src/audit/builtin/cpp_buffer_overflow_cpp.json tests/audit_json_integration.rs
git commit -m "fix(audit): restrict cpp_buffer_overflow to known unsafe functions via tree-sitter predicate"
```

---

### Task 5: Fix callback_hell_javascript.json (FP — fires on .map/.filter)

**Files:**
- Modify: `src/audit/builtin/callback_hell_javascript.json`
- Modify: `tests/audit_json_integration.rs` (add 2 tests)

- [ ] **Step 1: Write two failing integration tests**

Append to `tests/audit_json_integration.rs`:

```rust
// ── callback_hell (JavaScript) ──

#[test]
fn callback_hell_javascript_no_false_positive_on_map() {
    let dir = tempfile::tempdir().unwrap();
    // .map() with arrow function is idiomatic — should not fire
    let content = r#"
function processItems(items) {
    return items
        .filter(item => item.active)
        .map(item => ({ id: item.id, name: item.name }));
}
"#;
    std::fs::write(dir.path().join("service.js"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .categories(vec!["code_style".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "nested_callback"),
        "expected no nested_callback for idiomatic .map().filter(); got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn callback_hell_javascript_finds_async_nesting() {
    let dir = tempfile::tempdir().unwrap();
    // setTimeout inside setTimeout is callback hell
    let content = r#"
function fetchData(id, callback) {
    setTimeout(function() {
        getData(id, function(err, data) {
            if (err) callback(err);
            else callback(null, data);
        });
    }, 100);
}
"#;
    std::fs::write(dir.path().join("legacy.js"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .categories(vec!["code_style".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "nested_callback"),
        "expected nested_callback for setTimeout nesting; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Run to confirm finds_async_nesting fails**

```bash
cargo test callback_hell_javascript -- --nocapture 2>&1 | tail -15
```

Expected: `callback_hell_javascript_finds_async_nesting` FAIL (current pipeline uses `"code-quality"` category, not `"code_style"`, so it never runs under the `code_style` filter). `callback_hell_javascript_no_false_positive_on_map` passes vacuously (no pipeline runs → no findings).

- [ ] **Step 3: Rewrite callback_hell_javascript.json**

Overwrite `src/audit/builtin/callback_hell_javascript.json` with:

```json
{
  "pipeline": "callback_hell",
  "category": "code_style",
  "description": "Detects JavaScript callback hell: functions passed as arguments to non-functional-array methods. Excludes idiomatic array methods (map, filter, reduce, forEach, find, findIndex, some, every, flatMap) and Promise chain methods (then, catch, finally).",
  "languages": ["javascript"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (member_expression property: (property_identifier) @method_name) arguments: (arguments [(arrow_function) (function_expression)] @callback)) (#not-match? @method_name \"^(map|filter|reduce|forEach|find|findIndex|some|every|flatMap|then|catch|finally)$\")"
    },
    {
      "flag": {
        "pattern": "nested_callback",
        "message": "Callback passed to `{{name}}` -- if deeply nested, consider async/await or named functions",
        "severity": "info"
      }
    }
  ]
}
```

- [ ] **Step 5: Run both tests and confirm they pass**

```bash
cargo test callback_hell_javascript -- --nocapture 2>&1 | tail -10
```

Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/callback_hell_javascript.json tests/audit_json_integration.rs
git commit -m "fix(audit): exclude .map/.filter and Promise methods from callback_hell detector"
```

---

### Task 6: Fix anemic_domain_model_csharp.json (FP — fires on all classes)

**Files:**
- Modify: `src/audit/builtin/anemic_domain_model_csharp.json`
- Modify: `tests/audit_json_integration.rs` (add 2 tests)

- [ ] **Step 1: Write two failing integration tests**

Append to `tests/audit_json_integration.rs`:

```rust
// ── anemic_domain_model (C#) ──

#[test]
fn anemic_domain_model_csharp_no_false_positive_on_controller() {
    let dir = tempfile::tempdir().unwrap();
    let content = r#"public class OrderController {
    public IActionResult Index() { return Ok(); }
    public IActionResult Create() { return Ok(); }
}
"#;
    std::fs::write(dir.path().join("OrderController.cs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .categories(vec!["code_style".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "anemic_class"),
        "expected no anemic_class for Controller; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn anemic_domain_model_csharp_finds_domain_class() {
    let dir = tempfile::tempdir().unwrap();
    // A plain domain class without excluded suffix should be flagged
    let content = r#"public class Order {
    public int Id { get; set; }
    public string Status { get; set; }
}
"#;
    std::fs::write(dir.path().join("Order.cs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .categories(vec!["code_style".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "anemic_class"),
        "expected anemic_class for plain domain class; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Run to confirm finds_domain_class fails**

```bash
cargo test anemic_domain_model_csharp -- --nocapture 2>&1 | tail -15
```

Expected: `anemic_domain_model_csharp_finds_domain_class` FAIL (current pipeline uses `"code-quality"` category, not `"code_style"`, so it never runs). `anemic_domain_model_csharp_no_false_positive_on_controller` passes vacuously.

- [ ] **Step 3: Rewrite anemic_domain_model_csharp.json**

Overwrite `src/audit/builtin/anemic_domain_model_csharp.json` with:

```json
{
  "pipeline": "anemic_domain_model",
  "category": "code_style",
  "description": "Detects C# classes that look like domain objects but contain only properties (anemic domain model). Excludes infrastructure classes: Controller, Repository, Middleware, Handler, Service, Factory, Validator, Filter, Converter, Builder, Provider, Manager, Context, Config, Options.",
  "languages": ["csharp"],
  "graph": [
    {
      "match_pattern": "((class_declaration name: (identifier) @class_name) (#not-match? @class_name \"(?i)(Controller|Repository|Middleware|Handler|Service|Factory|Validator|Filter|Converter|Builder|Provider|Manager|Context|Config|Options)$\"))"
    },
    {
      "flag": {
        "pattern": "anemic_class",
        "message": "Class `{{name}}` may be anemic -- verify it has domain behavior, not just properties",
        "severity": "info"
      }
    }
  ]
}
```

- [ ] **Step 4: Run both tests and confirm they pass**

```bash
cargo test anemic_domain_model_csharp -- --nocapture 2>&1 | tail -10
```

Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add src/audit/builtin/anemic_domain_model_csharp.json tests/audit_json_integration.rs
git commit -m "fix(audit): exclude infrastructure classes from anemic_domain_model C# detector"
```

---

### Task 7: Fix any_escape_hatch_typescript.json (FP + wrong line numbers)

**Files:**
- Modify: `src/audit/builtin/any_escape_hatch_typescript.json`
- Modify: `tests/audit_json_integration.rs` (add 2 tests)

- [ ] **Step 1: Write two failing integration tests**

Append to `tests/audit_json_integration.rs`:

```rust
// ── any_escape_hatch (TypeScript) ──

#[test]
fn any_annotation_typescript_no_false_positive_on_string_type() {
    let dir = tempfile::tempdir().unwrap();
    // Uses string, number, boolean — should not fire
    let content = "function greet(name: string, age: number, active: boolean): string { return name; }\n";
    std::fs::write(dir.path().join("util.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .categories(vec!["code_style".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "any_annotation"),
        "expected no any_annotation for string/number/boolean params; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn any_annotation_typescript_finds_any_type_and_reports_correct_line() {
    let dir = tempfile::tempdir().unwrap();
    // `any` is on line 2, not line 1
    let content = "function safe(x: string): string { return x; }\nfunction risky(data: any): any { return data; }\n";
    std::fs::write(dir.path().join("api.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .categories(vec!["code_style".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();

    let any_findings: Vec<_> = findings.iter().filter(|f| f.pattern == "any_annotation").collect();
    assert!(
        !any_findings.is_empty(),
        "expected any_annotation findings"
    );
    // All reported lines must be line 2 (where `any` actually appears)
    for f in &any_findings {
        assert_eq!(
            f.line, 2,
            "any_annotation finding should be on line 2 (where `any` appears), got line {}",
            f.line
        );
    }
}
```

- [ ] **Step 2: Run to confirm finds_any_type fails**

```bash
cargo test any_annotation_typescript -- --nocapture 2>&1 | tail -15
```

Expected: `any_annotation_typescript_finds_any_type_and_reports_correct_line` FAIL (current pipeline uses `"code-quality"` category, not `"code_style"`, so it never runs). `any_annotation_typescript_no_false_positive_on_string_type` passes vacuously.

- [ ] **Step 3: Fix any_escape_hatch_typescript.json**

Overwrite `src/audit/builtin/any_escape_hatch_typescript.json` with:

```json
{
  "pipeline": "any_escape_hatch",
  "category": "code_style",
  "description": "Detects `any` type annotations in TypeScript, which bypass the type system. Uses #eq? predicate to match only the `any` keyword, not other predefined types like string/number/boolean. Reports the exact source line of the `any` keyword.",
  "languages": ["typescript"],
  "graph": [
    {
      "match_pattern": "((predefined_type) @ty (#eq? @ty \"any\"))"
    },
    {
      "flag": {
        "pattern": "any_annotation",
        "message": "`any` type -- bypasses TypeScript type safety; prefer `unknown` if the type is truly dynamic",
        "severity": "warning"
      }
    }
  ]
}
```

- [ ] **Step 4: Run both tests and confirm they pass**

```bash
cargo test any_annotation_typescript -- --nocapture 2>&1 | tail -10
```

Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add src/audit/builtin/any_escape_hatch_typescript.json tests/audit_json_integration.rs
git commit -m "fix(audit): restrict any_annotation to only match any keyword; fixes false positives and line numbers"
```

---

### Task 8: Raise excessive_public_api threshold in 9 api_surface_area files

**Files:**
- Modify: `src/audit/builtin/api_surface_area_rust.json`
- Modify: `src/audit/builtin/api_surface_area_javascript.json`
- Modify: `src/audit/builtin/api_surface_area_python.json`
- Modify: `src/audit/builtin/api_surface_area_go.json`
- Modify: `src/audit/builtin/api_surface_area_java.json`
- Modify: `src/audit/builtin/api_surface_area_c.json`
- Modify: `src/audit/builtin/api_surface_area_cpp.json`
- Modify: `src/audit/builtin/api_surface_area_csharp.json`
- Modify: `src/audit/builtin/api_surface_area_php.json`
- Modify: `tests/audit_json_integration.rs` (update existing test)

- [ ] **Step 1: Update the existing integration test to match new threshold**

In `tests/audit_json_integration.rs`, find `fn api_surface_area_typescript_finds_excessive` (around line 77). Change `(0..11)` to `(0..21)` and update the comment:

```rust
fn api_surface_area_typescript_finds_excessive() {
    let dir = tempfile::tempdir().unwrap();
    // 21 exported functions in one file, all exported = 100% ratio > 80% threshold, count > 20 threshold
    let content: String = (0..21)
        .map(|i| format!("export function handler_{i}() {{}}\n"))
        .collect();
```

- [ ] **Step 2: Run to confirm the existing test now fails (proves the threshold change is needed)**

```bash
cargo test api_surface_area_typescript_finds_excessive -- --nocapture 2>&1 | tail -10
```

Expected: FAIL — 21 functions but threshold is still 10.

- [ ] **Step 3: Raise threshold in all 9 JSON files**

In each of the 9 files below, find `"gte": 10` inside the `threshold.and` array and change it to `"gte": 20`.

The `"gte": 10` appears in the `metrics.count` predicate. The `"gte": 0.8` for ratio stays unchanged.

Files to edit (all have identical structure, only the `"languages"` field differs):
- `src/audit/builtin/api_surface_area_rust.json`
- `src/audit/builtin/api_surface_area_javascript.json`
- `src/audit/builtin/api_surface_area_python.json`
- `src/audit/builtin/api_surface_area_go.json`
- `src/audit/builtin/api_surface_area_java.json`
- `src/audit/builtin/api_surface_area_c.json`
- `src/audit/builtin/api_surface_area_cpp.json`
- `src/audit/builtin/api_surface_area_csharp.json`
- `src/audit/builtin/api_surface_area_php.json`

In each file, change:
```json
            {
              "metrics": {
                "count": {
                  "gte": 10
                }
              }
            }
```
to:
```json
            {
              "metrics": {
                "count": {
                  "gte": 20
                }
              }
            }
```

- [ ] **Step 4: Run the test to confirm it passes**

```bash
cargo test api_surface_area_typescript_finds_excessive api_surface_area_typescript_clean_file -- --nocapture 2>&1 | tail -10
```

Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add src/audit/builtin/api_surface_area_rust.json \
        src/audit/builtin/api_surface_area_javascript.json \
        src/audit/builtin/api_surface_area_python.json \
        src/audit/builtin/api_surface_area_go.json \
        src/audit/builtin/api_surface_area_java.json \
        src/audit/builtin/api_surface_area_c.json \
        src/audit/builtin/api_surface_area_cpp.json \
        src/audit/builtin/api_surface_area_csharp.json \
        src/audit/builtin/api_surface_area_php.json \
        tests/audit_json_integration.rs
git commit -m "fix(audit): raise excessive_public_api count threshold from 10 to 20 across all 9 languages"
```

---

### Task 9: Lower argument_mutation severity (follow-up marker)

**Files:**
- Modify: `src/audit/builtin/argument_mutation_javascript.json`

- [ ] **Step 1: Update severity and description**

Overwrite `src/audit/builtin/argument_mutation_javascript.json` with:

```json
{
  "pipeline": "argument_mutation",
  "category": "code_style",
  "description": "Detects member property assignments inside function bodies that may mutate caller data. False positive rate is HIGH -- local variables constructed inside the function body will also trigger. A proper fix requires scope analysis (lhs_is_parameter DSL primitive) which is a planned follow-up. Manual triage required.",
  "languages": ["javascript"],
  "graph": [
    {
      "match_pattern": "[(assignment_expression left: (member_expression) @lhs) (augmented_assignment_expression left: (member_expression) @lhs)] @assign"
    },
    {
      "flag": {
        "pattern": "argument_mutation",
        "message": "Member expression assignment -- manually verify the object is a function parameter, not a local variable (false positives possible)",
        "severity": "info"
      }
    }
  ]
}
```

- [ ] **Step 2: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src/audit/builtin/argument_mutation_javascript.json
git commit -m "fix(audit): lower argument_mutation severity to info; document scope-analysis limitation"
```

---

### Final verification

- [ ] **Run the complete test suite**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests PASS, zero failures.

- [ ] **Verify builtin count**

```bash
ls src/audit/builtin/*.json | wc -l
```

Expected: 293 (285 original + 8 new deep_nesting files).
