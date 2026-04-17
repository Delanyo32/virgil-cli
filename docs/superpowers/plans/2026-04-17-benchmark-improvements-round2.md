# Benchmark Improvements Round 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix JavaScript/C++/C# metric pipeline gaps, add `lhs_is_parameter` executor primitive, and add hardcoded_secrets/print_instead_of_logging/CommonJS dead_export/high_coupling/excessive_public_api JSON pipelines across all supported languages.

**Architecture:** Part 1 is all Rust — fix `find_function_body_at_line` for JS/C++/C#, extend C++ symbol query for qualified names, add `lhs_is_parameter` filtering in the `MatchPattern` executor stage. Part 2 is JSON-only — new and rewritten pipeline files consumed by the existing `AuditEngine`. CommonJS dead_export requires one additional Rust parser change in the JS extraction path.

**Tech Stack:** Rust, tree-sitter 0.25 (streaming_iterator API), serde_json, tempfile (tests)

---

## File Map

**Part 1 — Rust changes:**
- Modify: `src/pipeline/executor.rs` — `find_function_body_at_line`, `execute_match_pattern`
- Modify: `src/graph/metrics.rs` — `function_node_kinds_for_language`, `body_field_for_language`
- Modify: `src/pipeline/dsl.rs` — `WhereClause` (add `lhs_is_parameter`), `GraphStage::MatchPattern` (add `when`)
- Modify: `src/languages/cpp.rs` — extend symbol query for qualified names, skip forward declarations
- Modify: `src/languages/typescript.rs` — CommonJS export detection (JS path only)

**Part 2 — JSON pipelines:**
- Modify: `src/audit/builtin/api_surface_area_go.json`
- Modify: `src/audit/builtin/coupling_java.json`
- Modify: `src/audit/builtin/coupling_php.json` (if investigation reveals fix needed)
- Modify: `src/audit/builtin/argument_mutation_javascript.json`
- Create: `src/audit/builtin/print_in_production_python.json`
- Create: `src/audit/builtin/hardcoded_secrets_python.json`
- Create: `src/audit/builtin/hardcoded_secrets_javascript.json`
- Create: `src/audit/builtin/hardcoded_secrets_typescript.json`
- Create: `src/audit/builtin/hardcoded_secrets_java.json`
- Create: `src/audit/builtin/hardcoded_secrets_go.json`
- Create: `src/audit/builtin/hardcoded_secrets_rust.json`
- Create: `src/audit/builtin/hardcoded_secrets_csharp.json`
- Create: `src/audit/builtin/hardcoded_secrets_php.json`
- Create: `src/audit/builtin/hardcoded_secrets_c.json`
- Create: `src/audit/builtin/hardcoded_secrets_cpp.json`

---

### Task 1: JavaScript Arrow Function — Diagnostic Test

The JS metric pipelines miss all manifest entries despite the start_line fix. Write a failing test to expose exactly where the breakdown happens.

**Files:**
- Modify: `src/pipeline/executor.rs` (tests section)

- [ ] **Step 1: Write the failing test**

Find the `#[cfg(test)]` block near line 1800 in `src/pipeline/executor.rs` and add:

```rust
#[test]
fn test_compute_metric_nesting_depth_javascript_arrow_function() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("controller.js"),
        r#"const createComment = async (req, res) => {
    if (req.body) {
        if (req.user) {
            if (req.params.id) {
                if (req.body.parentId) {
                    return req.body;
                }
            }
        }
    }
};
"#,
    )
    .unwrap();
    let ws = crate::workspace::Workspace::load(dir.path(), &[Language::JavaScript], None).unwrap();

    let mut graph = CodeGraph::new();
    let file_idx = graph.graph.add_node(NodeWeight::File {
        path: "src/controller.js".to_string(),
        language: Language::JavaScript,
    });
    graph.file_nodes.insert("src/controller.js".to_string(), file_idx);
    let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
        name: "createComment".to_string(),
        kind: crate::models::SymbolKind::ArrowFunction,
        file_path: "src/controller.js".to_string(),
        start_line: 1,
        end_line: 11,
        exported: false,
    });
    graph.symbol_nodes.insert(("src/controller.js".to_string(), 1), sym_idx);

    let is_test_fn = |path: &str| is_test_file(path);
    let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
    let is_barrel_fn = |path: &str| is_barrel_file(path);
    let mut taint_ctx = TaintContext::default();

    let select_stage = GraphStage::Select {
        select: crate::pipeline::dsl::NodeType::Symbol,
        filter: None,
        exclude: None,
    };
    let nodes = execute_stage(
        &select_stage, Vec::new(), &graph, Some(&ws), None,
        "js_nesting_test", &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
    ).unwrap();

    let metric_stage = GraphStage::ComputeMetric {
        compute_metric: "nesting_depth".to_string(),
    };
    let result_nodes = execute_stage(
        &metric_stage, nodes, &graph, Some(&ws), None,
        "js_nesting_test", &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
    ).unwrap();

    assert_eq!(result_nodes.len(), 1, "expected 1 node");
    let depth = result_nodes[0].metrics.get("nesting_depth")
        .expect("nesting_depth metric should be present");
    // 4 levels: if > if > if > if
    assert_eq!(*depth, MetricValue::Int(4), "expected nesting depth of 4");
}
```

- [ ] **Step 2: Run the test to see it fail**

```bash
cargo test test_compute_metric_nesting_depth_javascript_arrow_function -- --nocapture 2>&1 | head -40
```

Expected: FAIL. Note the exact failure message — it will be either:
- `"Warning: compute_metric: no function body at line 1"` → `find_function_body_at_line` can't find the arrow_function node
- `assertion failed: nesting_depth metric should be present` → body found but metric not attached
- A panic with an unexpected value → body found but metric computed wrong

Record the actual failure message — it determines which fix to apply in Task 2.

- [ ] **Step 3: Commit the failing test**

```bash
git add src/pipeline/executor.rs
git commit -m "test(executor): add failing JS arrow function nesting depth test"
```

---

### Task 2: JavaScript Arrow Function — Fix

**Files:**
- Modify: `src/pipeline/executor.rs` — `find_function_body_at_line`
- Modify: `src/graph/metrics.rs` — `body_field_for_language` (if needed)

- [ ] **Step 1: Investigate the tree-sitter AST for the JS arrow function**

Add this temporary debug test to see what tree-sitter actually produces (add near Task 1's test):

```rust
#[test]
fn debug_js_arrow_function_ast() {
    use streaming_iterator::StreamingIterator;
    let source = b"const createComment = async (req, res) => {\n    if (req.body) { return req.body; }\n};\n";
    let mut parser = crate::parser::create_parser(Language::JavaScript).unwrap();
    let tree = parser.parse(source, None).unwrap();
    // Print all nodes at row 0 (the arrow function line)
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.start_position().row == 0 {
            eprintln!(
                "kind={} row={} field_body={:?}",
                node.kind(),
                node.start_position().row,
                node.child_by_field_name("body").map(|b| b.kind())
            );
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}
```

```bash
cargo test debug_js_arrow_function_ast -- --nocapture 2>&1 | grep "kind="
```

Read the output. You will see every node at row 0 and whether it has a "body" field. This tells you:
- Whether `arrow_function` appears at row 0 ✓ or not ✗
- Whether `child_by_field_name("body")` returns something ✓ or None ✗

- [ ] **Step 2: Apply the appropriate fix**

**If the debug output shows `arrow_function` IS at row 0 but `field_body=None`** — the JS grammar doesn't name the body field `"body"`. Fix by checking the actual body field name and adding a JS-specific override:

In `src/graph/metrics.rs`, change `body_field_for_language`:
```rust
pub fn body_field_for_language(lang: Language) -> &'static str {
    // All supported languages use "body" as the body field name.
    "body"
}
```
to:
```rust
pub fn body_field_for_language(lang: Language) -> &'static [&'static str] {
    // Most languages use "body"; JS arrow_function also accepts "body" but
    // the field may be named differently for expression-body arrows.
    &["body"]
}
```

And update `find_function_body_at_line` in `src/pipeline/executor.rs` to try all candidate field names:
```rust
fn find_function_body_at_line(
    root: tree_sitter::Node,
    target_line: usize,
    lang: crate::language::Language,
) -> Option<tree_sitter::Node> {
    let func_kinds = crate::graph::metrics::function_node_kinds_for_language(lang);
    let body_fields = crate::graph::metrics::body_field_for_language(lang);

    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if func_kinds.contains(&current.kind()) && current.start_position().row == target_line {
            for field in body_fields {
                if let Some(body) = current.child_by_field_name(field) {
                    return Some(body);
                }
            }
            // Fallback: return the node itself as its own body (expression-body arrow)
            return Some(current);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
    None
}
```

**If the debug output shows `arrow_function` is NOT at row 0** — the `async` keyword shifts the node position. Fix by also searching for `arrow_function` as a descendant of `lexical_declaration` at the target row:

In `find_function_body_at_line`, before the stack.push loop, add:
```rust
// For JS async arrow functions, the arrow_function node may start after
// the async keyword on the same row but have a different start column.
// Accept any func_kinds node whose start ROW matches, regardless of column.
if func_kinds.contains(&current.kind()) && current.start_position().row == target_line {
    if let Some(body) = current.child_by_field_name(body_field) {
        return Some(body);
    }
}
```
(This is already the existing logic — if it's still failing, the issue is `child_by_field_name` returning None. In that case, change the fallback to traverse named children until finding a block/statement_block node.)

- [ ] **Step 3: Delete the debug test, run the failing test**

```bash
# Remove debug_js_arrow_function_ast test, then:
cargo test test_compute_metric_nesting_depth_javascript_arrow_function -- --nocapture
```

Expected: PASS with nesting_depth = 4.

- [ ] **Step 4: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 5: Commit**

```bash
git add src/pipeline/executor.rs src/graph/metrics.rs
git commit -m "fix(metrics): resolve JS arrow function body lookup in find_function_body_at_line"
```

---

### Task 3: C++ Metric Fix — Qualified Name Symbol Query

C++ functions defined outside their class (`int Foo::process(int x) { ... }`) have a `qualified_identifier` as the inner declarator, not a plain `identifier`. The current `CPP_SYMBOL_QUERY` misses these entirely.

**Files:**
- Modify: `src/languages/cpp.rs`
- Modify: `src/pipeline/executor.rs` (add test)

- [ ] **Step 1: Write failing test**

In `src/pipeline/executor.rs` tests, add:

```rust
#[test]
fn test_compute_metric_nesting_depth_cpp_qualified_method() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("processor.cpp"),
        r#"int DataProcessor::process(int x, int y) {
    if (x > 0) {
        if (y > 0) {
            if (x > y) {
                if (x > 10) {
                    return x;
                }
            }
        }
    }
    return 0;
}
"#,
    )
    .unwrap();
    let ws = crate::workspace::Workspace::load(dir.path(), &[Language::Cpp], None).unwrap();

    let mut graph = CodeGraph::new();
    let file_idx = graph.graph.add_node(NodeWeight::File {
        path: "src/processor.cpp".to_string(),
        language: Language::Cpp,
    });
    graph.file_nodes.insert("src/processor.cpp".to_string(), file_idx);
    let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
        name: "process".to_string(),
        kind: crate::models::SymbolKind::Function,
        file_path: "src/processor.cpp".to_string(),
        start_line: 1,
        end_line: 12,
        exported: false,
    });
    graph.symbol_nodes.insert(("src/processor.cpp".to_string(), 1), sym_idx);

    let is_test_fn = |path: &str| is_test_file(path);
    let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
    let is_barrel_fn = |path: &str| is_barrel_file(path);
    let mut taint_ctx = TaintContext::default();

    let nodes = execute_stage(
        &GraphStage::Select { select: crate::pipeline::dsl::NodeType::Symbol, filter: None, exclude: None },
        Vec::new(), &graph, Some(&ws), None, "cpp_nesting_test",
        &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
    ).unwrap();

    let result_nodes = execute_stage(
        &GraphStage::ComputeMetric { compute_metric: "nesting_depth".to_string() },
        nodes, &graph, Some(&ws), None, "cpp_nesting_test",
        &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
    ).unwrap();

    assert_eq!(result_nodes.len(), 1, "expected symbol to be present in graph");
    let depth = result_nodes[0].metrics.get("nesting_depth")
        .expect("nesting_depth metric should be present");
    assert_eq!(*depth, MetricValue::Int(4));
}
```

- [ ] **Step 2: Run the test to see it fail**

```bash
cargo test test_compute_metric_nesting_depth_cpp_qualified_method -- --nocapture 2>&1 | head -20
```

Note: this test constructs the symbol manually, so it bypasses the symbol query. If it fails with "no function body", the issue is in `find_function_body_at_line`. If the assert `expected symbol to be present` fires... the test itself is wrong (symbol IS in the graph since we added it manually). Either way, the failure message guides the next step.

- [ ] **Step 3: Extend CPP_SYMBOL_QUERY to match qualified names**

In `src/languages/cpp.rs`, find `CPP_SYMBOL_QUERY` (around line 14) and add after the first `function_definition` rule:

```rust
const CPP_SYMBOL_QUERY: &str = r#"
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition

(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (identifier) @name))) @definition

(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @name))) @definition

(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (qualified_identifier
        name: (identifier) @name)))) @definition

(declaration
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition

(declaration
  declarator: (init_declarator
    declarator: (identifier) @name)) @definition

(declaration
  declarator: (identifier) @name) @definition

(struct_specifier
  name: (type_identifier) @name
  body: (field_declaration_list)) @definition

(union_specifier
  name: (type_identifier) @name
  body: (field_declaration_list)) @definition

(enum_specifier
  name: (type_identifier) @name
  body: (enumerator_list)) @definition

(class_specifier
  name: (type_identifier) @name
  body: (field_declaration_list)) @definition
"#;
```

- [ ] **Step 4: Run failing test again**

```bash
cargo test test_compute_metric_nesting_depth_cpp_qualified_method -- --nocapture 2>&1 | head -20
```

(The test manually constructs the symbol, so this tests the metric computation path. The query extension is tested via the full audit path.)

- [ ] **Step 5: Run full tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 6: Commit**

```bash
git add src/languages/cpp.rs src/pipeline/executor.rs
git commit -m "fix(cpp): extend symbol query to match qualified-name function definitions"
```

---

### Task 4: Skip C++ Forward Declarations + C# Diagnostic

Forward declarations (`int foo(int x);`) get symbolized as `Function` kind in C++, creating silent compute_metric warnings for bodyless nodes. Also add a C# metric test.

**Files:**
- Modify: `src/languages/cpp.rs` — `determine_cpp_kind`
- Modify: `src/pipeline/executor.rs` (add C# test)

- [ ] **Step 1: Write failing C# metric test**

In `src/pipeline/executor.rs` tests, add:

```rust
#[test]
fn test_compute_metric_function_length_csharp_method() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    // Write a C# method longer than 50 lines
    let mut body = String::from("public class MyService {\n    public void ProcessOrder(Order order) {\n");
    for i in 0..55 {
        body.push_str(&format!("        var step{} = order.Id + {};\n", i, i));
    }
    body.push_str("    }\n}\n");
    std::fs::write(src_dir.join("service.cs"), &body).unwrap();

    let ws = crate::workspace::Workspace::load(dir.path(), &[Language::CSharp], None).unwrap();
    let mut graph = CodeGraph::new();
    let file_idx = graph.graph.add_node(NodeWeight::File {
        path: "src/service.cs".to_string(),
        language: Language::CSharp,
    });
    graph.file_nodes.insert("src/service.cs".to_string(), file_idx);
    // Method starts at line 2 (1-indexed)
    let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
        name: "ProcessOrder".to_string(),
        kind: crate::models::SymbolKind::Method,
        file_path: "src/service.cs".to_string(),
        start_line: 2,
        end_line: 59,
        exported: false,
    });
    graph.symbol_nodes.insert(("src/service.cs".to_string(), 2), sym_idx);

    let is_test_fn = |path: &str| is_test_file(path);
    let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
    let is_barrel_fn = |path: &str| is_barrel_file(path);
    let mut taint_ctx = TaintContext::default();

    let nodes = execute_stage(
        &GraphStage::Select { select: crate::pipeline::dsl::NodeType::Symbol, filter: None, exclude: None },
        Vec::new(), &graph, Some(&ws), None, "csharp_length_test",
        &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
    ).unwrap();

    let result_nodes = execute_stage(
        &GraphStage::ComputeMetric { compute_metric: "function_length".to_string() },
        nodes, &graph, Some(&ws), None, "csharp_length_test",
        &is_test_fn, &is_generated_fn, &is_barrel_fn, &mut taint_ctx,
    ).unwrap();

    assert_eq!(result_nodes.len(), 1);
    let len = result_nodes[0].metrics.get("function_length")
        .expect("function_length metric should be present");
    // 55 var assignments + 2 braces = 57 body lines
    match len {
        MetricValue::Int(v) => assert!(*v >= 50, "expected >= 50 lines, got {}", v),
        _ => panic!("expected Int metric"),
    }
}
```

- [ ] **Step 2: Run to see it fail**

```bash
cargo test test_compute_metric_function_length_csharp_method -- --nocapture 2>&1 | head -20
```

If it fails with "no function body at line 2", the C# `method_declaration` body lookup needs investigation. Run the AST debug pattern from Task 2 Step 1 adapted for C# to see the node structure at row 1 (0-indexed).

- [ ] **Step 3: Fix C# body lookup if needed**

If `method_declaration` in C# tree-sitter grammar uses `"body"` as the field name for `block`, this should already work. If it fails:

In `src/graph/metrics.rs`, check `function_node_kinds_for_language` for CSharp:
```rust
Language::CSharp => &["method_declaration", "constructor_declaration"],
```

Add any missing kinds (e.g. `local_function_statement`) if the debug output reveals them.

- [ ] **Step 4: Skip C++ forward declarations**

In `src/languages/cpp.rs`, find `determine_cpp_kind` function. Forward declarations are `declaration` nodes — they produce symbols with no body. Return `None` for function-like `declaration` nodes so they're skipped:

Locate the match arm handling `"declaration"` and change it to return `None` for function-like declarations:

```rust
// In determine_cpp_kind, find the "declaration" handling arm.
// Return None so forward declarations are not symbolized.
// Full function_definition nodes (with body) ARE matched by the
// function_definition rules in CPP_SYMBOL_QUERY instead.
"declaration" => {
    // Skip forward declarations — they have no body and produce
    // silent compute_metric warnings.
    None
}
```

> Note: Read `determine_cpp_kind` in `src/languages/cpp.rs` first to find the exact location and confirm the `"declaration"` arm exists before making this change.

- [ ] **Step 5: Run full tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 6: Commit**

```bash
git add src/languages/cpp.rs src/pipeline/executor.rs
git commit -m "fix(cpp,csharp): skip forward declarations, add C# function_length test"
```

---

### Task 5: `lhs_is_parameter` — DSL Extension

Add `lhs_is_parameter: Option<bool>` to `WhereClause` and `when: Option<WhereClause>` to the `MatchPattern` DSL stage.

**Files:**
- Modify: `src/pipeline/dsl.rs`

- [ ] **Step 1: Add `lhs_is_parameter` to `WhereClause`**

In `src/pipeline/dsl.rs`, find the `WhereClause` struct. After the `metrics` field, add:

```rust
/// When true, the match node's LHS member-expression object must be a named
/// parameter of the enclosing function. Used to filter argument_mutation findings.
#[serde(default)]
pub lhs_is_parameter: Option<bool>,
```

Also update `is_empty()` to include the new field:

```rust
&& self.lhs_is_parameter.is_none()
```

- [ ] **Step 2: Add `when` field to `GraphStage::MatchPattern`**

Find `GraphStage::MatchPattern` in `src/pipeline/dsl.rs`:

```rust
MatchPattern {
    match_pattern: String,
},
```

Change to:

```rust
MatchPattern {
    match_pattern: String,
    /// Optional post-filter applied to each match result.
    #[serde(default)]
    when: Option<WhereClause>,
},
```

- [ ] **Step 3: Write a deserialisation test**

In the `#[cfg(test)]` block at the bottom of `src/pipeline/dsl.rs`, add:

```rust
#[test]
fn test_deserialize_match_pattern_with_when() {
    let json = r#"{"match_pattern": "(identifier) @name", "when": {"lhs_is_parameter": true}}"#;
    let stage: GraphStage = serde_json::from_str(json).unwrap();
    match stage {
        GraphStage::MatchPattern { match_pattern, when } => {
            assert_eq!(match_pattern, "(identifier) @name");
            let wc = when.expect("when should be present");
            assert_eq!(wc.lhs_is_parameter, Some(true));
        }
        _ => panic!("expected MatchPattern stage"),
    }
}
```

- [ ] **Step 4: Run the test**

```bash
cargo test test_deserialize_match_pattern_with_when -- --nocapture
```

Expected: PASS (serde handles the new fields automatically).

- [ ] **Step 5: Run full tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 6: Commit**

```bash
git add src/pipeline/dsl.rs
git commit -m "feat(dsl): add lhs_is_parameter to WhereClause and when to MatchPattern stage"
```

---

### Task 6: `lhs_is_parameter` — Executor Implementation

Wire up the `when` filter in `execute_match_pattern`. Implement `lhs_is_parameter` by walking the tree upward from a match node to its enclosing function.

**Files:**
- Modify: `src/pipeline/executor.rs`

- [ ] **Step 1: Write a failing test for `lhs_is_parameter`**

In `src/pipeline/executor.rs` tests, add:

```rust
#[test]
fn test_lhs_is_parameter_filters_local_object_mutations() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    // createFilter: locally-constructed object — should NOT fire
    // mutateParam: genuine parameter mutation — SHOULD fire
    std::fs::write(
        dir.path().join("src").join("service.js"),
        r#"function createFilter(role) {
    const filter = {};
    filter.role = role;
    return filter;
}
function mutateParam(user) {
    user.name = "overwritten";
}
"#,
    )
    .unwrap();
    let ws = crate::workspace::Workspace::load(
        dir.path(), &[Language::JavaScript], None,
    ).unwrap();

    let stages = vec![
        GraphStage::MatchPattern {
            match_pattern: "(assignment_expression left: (member_expression) @lhs) @assign".to_string(),
            when: Some(crate::pipeline::dsl::WhereClause {
                lhs_is_parameter: Some(true),
                ..Default::default()
            }),
        },
        GraphStage::Flag {
            flag: crate::pipeline::dsl::FlagConfig {
                pattern: "argument_mutation".to_string(),
                message: "mutation".to_string(),
                severity: Some("warning".to_string()),
                severity_map: None,
                pipeline_name: None,
            },
        },
    ];
    let graph = CodeGraph::new();
    let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "lhs_param_test").unwrap();
    match out {
        PipelineOutput::Findings(findings) => {
            assert_eq!(findings.len(), 1, "expected exactly 1 finding (mutateParam), got {:?}", findings.iter().map(|f| &f.message).collect::<Vec<_>>());
            assert!(findings[0].line == 7, "expected line 7 (user.name = ...), got {}", findings[0].line);
        }
        _ => panic!("expected Findings"),
    }
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
cargo test test_lhs_is_parameter_filters_local_object_mutations -- --nocapture 2>&1 | head -20
```

Expected: FAIL — the `when` filter is not implemented yet, so both `filter.role` and `user.name` findings are emitted (2 findings instead of 1).

- [ ] **Step 3: Implement `when` filtering in `execute_match_pattern`**

In `src/pipeline/executor.rs`, find `fn execute_match_pattern` (around line 765). Change its signature and add filtering after the match loop:

```rust
fn execute_match_pattern(
    query_str: &str,
    when: Option<&crate::pipeline::dsl::WhereClause>,
    workspace: &Workspace,
    pipeline_languages: Option<&[String]>,
) -> anyhow::Result<Vec<PipelineNode>> {
    use streaming_iterator::StreamingIterator;
    let mut result = Vec::new();

    for rel_path in workspace.files() {
        let Some(lang) = workspace.file_language(rel_path) else { continue };
        if let Some(langs) = pipeline_languages {
            let lang_str = lang.as_str();
            if !langs.iter().any(|l| l.eq_ignore_ascii_case(lang_str)) { continue }
        }
        let Some(source) = workspace.read_file(rel_path) else { continue };
        let ts_lang = lang.tree_sitter_language();
        let query = match tree_sitter::Query::new(&ts_lang, query_str) {
            Ok(q) => q,
            Err(_) => continue,
        };
        let mut parser = crate::parser::create_parser(lang)?;
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => { eprintln!("Warning: match_pattern: failed to parse {rel_path}"); continue }
        };
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let node = cap.node;
                // Apply when filter if present
                if let Some(wc) = when {
                    if wc.lhs_is_parameter == Some(true) {
                        if !node_lhs_is_parameter(&node, tree.root_node(), source.as_bytes(), lang) {
                            continue;
                        }
                    }
                }
                let line = node.start_position().row as u32 + 1;
                result.push(PipelineNode {
                    node_idx: petgraph::graph::NodeIndex::new(0),
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

- [ ] **Step 4: Implement `node_lhs_is_parameter`**

Add this helper function just before `execute_match_pattern` in `src/pipeline/executor.rs`:

```rust
/// For an `assignment_expression` or `augmented_assignment_expression` node,
/// check whether the LHS member-expression's object is a named parameter of
/// the nearest enclosing function. Returns false if the node is not an
/// assignment or if the object cannot be traced to a parameter.
fn node_lhs_is_parameter(
    node: &tree_sitter::Node,
    root: tree_sitter::Node,
    source: &[u8],
    lang: crate::language::Language,
) -> bool {
    // Accept assignment_expression or augmented_assignment_expression
    let kind = node.kind();
    if kind != "assignment_expression" && kind != "augmented_assignment_expression" {
        return false;
    }
    // Get the LHS child (field "left")
    let Some(lhs) = node.child_by_field_name("left") else { return false };
    if lhs.kind() != "member_expression" { return false }
    // Get the object of the member expression (field "object")
    let Some(obj) = lhs.child_by_field_name("object") else { return false };
    if obj.kind() != "identifier" { return false }
    let obj_name = match obj.utf8_text(source) {
        Ok(n) => n,
        Err(_) => return false,
    };

    // Walk up the tree from the assignment node to find the enclosing function
    let func_kinds = crate::graph::metrics::function_node_kinds_for_language(lang);
    // Build parent map by walking the whole tree once
    let mut parent_map: std::collections::HashMap<usize, tree_sitter::Node> = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            parent_map.insert(child.id(), current);
            stack.push(child);
        }
    }

    // Walk up from node to find enclosing function
    let mut current_id = node.id();
    loop {
        let Some(parent) = parent_map.get(&current_id) else { break };
        if func_kinds.contains(&parent.kind()) {
            // Found enclosing function — collect parameter names
            let params = collect_function_params(parent, source, lang);
            return params.contains(obj_name);
        }
        current_id = parent.id();
    }
    false
}

/// Collect the parameter identifier names from a function node.
fn collect_function_params<'a>(func_node: &tree_sitter::Node, source: &'a [u8], lang: crate::language::Language) -> Vec<&'a str> {
    let mut params = Vec::new();
    // For JS/TS: parameters are in a "parameters" child (formal_parameters)
    // For other languages: similar pattern
    let params_field = match lang {
        crate::language::Language::JavaScript
        | crate::language::Language::Jsx
        | crate::language::Language::TypeScript
        | crate::language::Language::Tsx => "parameters",
        _ => "parameters",
    };
    let Some(params_node) = func_node.child_by_field_name(params_field) else { return params };
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        // identifier children are parameter names; pattern children (destructuring) skipped
        if child.kind() == "identifier" {
            if let Ok(name) = child.utf8_text(source) {
                params.push(name);
            }
        }
        // Also handle: required_parameter (TS), optional_parameter (TS)
        if child.kind() == "required_parameter" || child.kind() == "optional_parameter" {
            if let Some(pattern) = child.child_by_field_name("pattern") {
                if pattern.kind() == "identifier" {
                    if let Ok(name) = pattern.utf8_text(source) {
                        params.push(name);
                    }
                }
            }
        }
    }
    params
}
```

- [ ] **Step 5: Update the call site in the executor stage dispatch**

In the `execute_stage` match arm for `GraphStage::MatchPattern`, update the call to pass `when`:

```rust
GraphStage::MatchPattern { match_pattern, when } => {
    match workspace {
        Some(ws) => execute_match_pattern(match_pattern, when.as_ref(), ws, pipeline_languages),
        None => anyhow::bail!(
            "match_pattern stage requires workspace -- call run_pipeline with Some(workspace)"
        ),
    }
}
```

- [ ] **Step 6: Run the failing test**

```bash
cargo test test_lhs_is_parameter_filters_local_object_mutations -- --nocapture
```

Expected: PASS (1 finding for `mutateParam`, not 2).

- [ ] **Step 7: Run full tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 8: Commit**

```bash
git add src/pipeline/executor.rs
git commit -m "feat(executor): implement lhs_is_parameter filter in MatchPattern stage"
```

---

### Task 7: Update `argument_mutation` + Pipeline Audit

**Files:**
- Modify: `src/audit/builtin/argument_mutation_javascript.json`
- Audit: all files in `src/audit/builtin/` matching `*_javascript.json`, `*_typescript.json`

- [ ] **Step 1: Update `argument_mutation_javascript.json`**

Replace the contents of `src/audit/builtin/argument_mutation_javascript.json` with:

```json
{
  "pipeline": "argument_mutation",
  "category": "code_style",
  "description": "Detects mutation of function parameters in JavaScript. Fires only when the LHS member expression object is a named function parameter.",
  "languages": [
    "javascript"
  ],
  "graph": [
    {
      "match_pattern": "[(assignment_expression left: (member_expression) @lhs) (augmented_assignment_expression left: (member_expression) @lhs)] @assign",
      "when": { "lhs_is_parameter": true }
    },
    {
      "flag": {
        "pattern": "argument_mutation",
        "message": "Function parameter `{{name}}` is mutated — prefer returning a new value instead",
        "severity": "warning"
      }
    }
  ]
}
```

- [ ] **Step 2: Audit all JS/TS pipelines for `lhs_is_parameter` applicability**

```bash
grep -l "assignment_expression\|member_expression" src/audit/builtin/*.json
```

For each file returned, check whether it flags member-expression assignments that could be false-positives if `lhs_is_parameter` were applied. The key question: does this pipeline intend to fire ONLY on parameter mutations, or on all assignments?

If a pipeline currently has a `"manually verify"` note AND targets `assignment_expression left: (member_expression)` — add `"when": {"lhs_is_parameter": true}` and remove the note.

If a pipeline targets broader patterns (e.g. all `member_expression` reads, not assignments) — leave it unchanged.

Document any pipelines updated.

- [ ] **Step 3: Verify audit finds zero regressions**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 4: Commit**

```bash
git add src/audit/builtin/argument_mutation_javascript.json
# Add any other pipelines updated in the audit
git commit -m "fix(pipelines): apply lhs_is_parameter to argument_mutation, audit other JS/TS pipelines"
```

---

### Task 8: JSON Quick Fixes — Go Threshold, Java Coupling, Python Pipelines

**Files:**
- Modify: `src/audit/builtin/api_surface_area_go.json`
- Modify: `src/audit/builtin/coupling_java.json`
- Modify: `src/audit/builtin/coupling_php.json` (investigation first)
- Create: `src/audit/builtin/print_in_production_python.json`

- [ ] **Step 1: Lower Go `excessive_public_api` threshold**

In `src/audit/builtin/api_surface_area_go.json`, change the `count` threshold from `20` to `10`:

```json
"threshold": {
  "and": [
    { "metrics": { "count": { "gte": 10 } } },
    { "metrics": { "ratio": { "gte": 0.8 } } }
  ]
}
```

- [ ] **Step 2: Rewrite Java `high_coupling` pipeline**

Replace the full contents of `src/audit/builtin/coupling_java.json` with:

```json
{
  "pipeline": "coupling",
  "category": "code-quality",
  "description": "Detects Java files with high efferent coupling (excessive unique import dependencies). Warning >= 8 imports, error >= 15.",
  "languages": [
    "java"
  ],
  "graph": [
    {
      "select": "file",
      "exclude": {
        "or": [
          { "is_test_file": true },
          { "is_generated": true }
        ]
      }
    },
    { "compute_metric": "efferent_coupling" },
    {
      "flag": {
        "pattern": "high_coupling",
        "message": "{{file}} imports from {{efferent_coupling}} modules — high fan-out coupling",
        "severity_map": [
          {
            "when": { "metrics": { "efferent_coupling": { "gte": 15 } } },
            "severity": "error"
          },
          {
            "when": { "metrics": { "efferent_coupling": { "gte": 8 } } },
            "severity": "warning"
          }
        ]
      }
    }
  ]
}
```

- [ ] **Step 3: Investigate PHP coupling line numbers**

Run the PHP benchmark against the current pipeline to see what line numbers are reported:

```bash
cargo run -- audit --dir ../virgil-skills/benchmarks/php/laravel-store --language php --pipeline coupling 2>&1 | head -20
```

If findings point to `use` statement lines (lines 5-15 in a typical Laravel file) — no fix needed, note it as resolved.
If findings point to the class declaration line (typically line 17+) — open `src/audit/builtin/coupling_php.json` and check whether `namespace_use_declaration` has correct line reporting. If it fires at the class line, the issue is that PHP `namespace_use_declaration` nodes don't have separate line positions from their containing file scope. In that case, update the match pattern to target the individual `use_instead_of_namespace` child:

```json
{
  "match_pattern": "(namespace_use_declaration (namespace_use_clause) @use_clause) @use_stmt"
}
```

- [ ] **Step 4: Create `print_in_production_python.json`**

Create `src/audit/builtin/print_in_production_python.json`:

```json
{
  "pipeline": "print_in_production",
  "category": "code_style",
  "description": "Detects print() calls in production Python modules. In codebases using a logging library, print() loses severity levels, routing, and structured context.",
  "languages": [
    "python"
  ],
  "graph": [
    {
      "match_pattern": "(call function: (identifier) @fn (#eq? @fn \"print\")) @call",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "print_instead_of_logging",
        "message": "print() call in production code — replace with logging.info() / logging.debug()",
        "severity": "info"
      }
    }
  ]
}
```

> Note: `exclude` on `match_pattern` is a DSL field. Check whether `GraphStage::MatchPattern` supports an `exclude` field. If it doesn't, omit the exclude and add it when the DSL is extended, or use the `is_test_file` filter at the Flag stage if supported.

- [ ] **Step 5: Build to verify JSON is valid**

```bash
cargo build 2>&1 | grep -E "error|warning" | head -10
```

Expected: no errors (JSON files are loaded at runtime, but a build confirms Rust compiles).

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/api_surface_area_go.json \
        src/audit/builtin/coupling_java.json \
        src/audit/builtin/coupling_php.json \
        src/audit/builtin/print_in_production_python.json
git commit -m "feat(pipelines): lower Go API threshold, rewrite Java coupling, add Python print detector"
```

---

### Task 9: `hardcoded_secrets` Pipelines — All Languages

Ten pipeline files with the same structure: name-pattern match on LHS identifier, string literal on RHS, `error` severity.

**Files:** All in `src/audit/builtin/`

- [ ] **Step 1: Create `hardcoded_secrets_python.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects Python variable assignments where the name suggests a secret and the value is a string literal.",
  "languages": ["python"],
  "graph": [
    {
      "match_pattern": "(assignment left: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") right: (string) @val) @assign",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable or secrets manager",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 2: Create `hardcoded_secrets_javascript.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects JavaScript variable declarations where the name suggests a secret and the value is a string literal.",
  "languages": ["javascript"],
  "graph": [
    {
      "match_pattern": "[(lexical_declaration (variable_declarator name: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (string) @val)) (variable_declaration (variable_declarator name: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (string) @val))] @decl",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 3: Create `hardcoded_secrets_typescript.json`**

Same as JavaScript but `"languages": ["typescript"]`:

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects TypeScript variable declarations where the name suggests a secret and the value is a string literal.",
  "languages": ["typescript"],
  "graph": [
    {
      "match_pattern": "[(lexical_declaration (variable_declarator name: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (string) @val)) (variable_declaration (variable_declarator name: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (string) @val))] @decl",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 4: Create `hardcoded_secrets_java.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects Java field declarations where the name suggests a secret and the value is a string literal.",
  "languages": ["java"],
  "graph": [
    {
      "match_pattern": "(field_declaration (variable_declarator name: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (string_literal) @val)) @field",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Create `hardcoded_secrets_go.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects Go constant declarations where the name suggests a secret and the value is a string literal.",
  "languages": ["go"],
  "graph": [
    {
      "match_pattern": "(const_declaration (const_spec name: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (interpreted_string_literal) @val)) @const",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 6: Create `hardcoded_secrets_rust.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects Rust const/static items where the name suggests a secret and the value is a string literal.",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "[(const_item name: (identifier) @name (#match? @name \"(?i)(SECRET|PASSWORD|API_KEY|TOKEN|CREDENTIAL|AUTH_KEY|PRIVATE_KEY)\") value: (string_literal) @val) (static_item name: (identifier) @name (#match? @name \"(?i)(SECRET|PASSWORD|API_KEY|TOKEN|CREDENTIAL|AUTH_KEY|PRIVATE_KEY)\") value: (string_literal) @val)] @item",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 7: Create `hardcoded_secrets_csharp.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects C# field declarations where the name suggests a secret and the value is a string literal.",
  "languages": ["csharp"],
  "graph": [
    {
      "match_pattern": "(field_declaration (variable_declaration (variable_declarator name: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") (equals_value_clause (string_literal) @val)))) @field",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 8: Create `hardcoded_secrets_php.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects PHP variable assignments where the name suggests a secret and the value is a string.",
  "languages": ["php"],
  "graph": [
    {
      "match_pattern": "(assignment_expression left: (variable_name (name) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\")) right: [(string) (encapsed_string)] @val) @assign",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 9: Create `hardcoded_secrets_c.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects C variable initializations where the name suggests a secret and the value is a string literal.",
  "languages": ["c"],
  "graph": [
    {
      "match_pattern": "(declaration (init_declarator declarator: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (string_literal) @val)) @decl",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 10: Create `hardcoded_secrets_cpp.json`**

```json
{
  "pipeline": "hardcoded_secrets",
  "category": "security",
  "description": "Detects C++ variable initializations where the name suggests a secret and the value is a string literal.",
  "languages": ["cpp"],
  "graph": [
    {
      "match_pattern": "(declaration (init_declarator declarator: (identifier) @name (#match? @name \"(?i)(secret|password|api_key|token|credential|auth_key|private_key)\") value: (string_literal) @val)) @decl",
      "exclude": { "is_test_file": true }
    },
    {
      "flag": {
        "pattern": "hardcoded_secrets",
        "message": "Potential hardcoded secret in `{{name}}` — move to environment variable",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 11: Build and test**

```bash
cargo build 2>&1 | grep "^error" | head -5
cargo test 2>&1 | tail -5
```

Expected: no build errors, `test result: ok.`

- [ ] **Step 12: Commit**

```bash
git add src/audit/builtin/hardcoded_secrets_*.json
git commit -m "feat(pipelines): add hardcoded_secrets detection for all 10 languages"
```

---

### Task 10: CommonJS `dead_export` — JavaScript Parser Extension

Extend the JavaScript extraction path in `src/languages/typescript.rs` to detect `module.exports.NAME = value` and `exports.NAME = value` assignments as exported symbols.

**Files:**
- Modify: `src/languages/typescript.rs`
- Modify: `src/pipeline/executor.rs` (add test)

- [ ] **Step 1: Write failing test**

In `src/pipeline/executor.rs` tests, add:

```rust
#[test]
fn test_commonjs_exports_marked_as_exported() {
    use crate::languages::typescript::extract_symbols;
    let source = r#"
function helper() { return 1; }
module.exports.helper = helper;
exports.utils = function() { return 2; };
function internal() {}
"#;
    let syms = extract_symbols("utils.js", source, crate::language::Language::JavaScript).unwrap();
    let exported: Vec<_> = syms.iter().filter(|s| s.is_exported).collect();
    assert_eq!(exported.len(), 2, "expected 2 CommonJS exports, got {:?}", exported.iter().map(|s| &s.name).collect::<Vec<_>>());
    let names: Vec<&str> = exported.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"helper"), "expected 'helper' export");
    assert!(names.contains(&"utils"), "expected 'utils' export");
}
```

- [ ] **Step 2: Run the test to see it fail**

```bash
cargo test test_commonjs_exports_marked_as_exported -- --nocapture 2>&1 | head -20
```

Expected: FAIL — `helper` and `utils` are not currently marked as exported because CommonJS assignments aren't detected.

- [ ] **Step 3: Implement CommonJS export detection**

In `src/languages/typescript.rs`, find the JS symbol extraction function (the one called when `Language::JavaScript`). After the existing symbol extraction loop, add a second pass to detect CommonJS exports:

Add a new tree-sitter query constant for CommonJS patterns:

```rust
const JS_COMMONJS_EXPORT_QUERY: &str = r#"
(assignment_expression
  left: (member_expression
    object: (member_expression
      object: (identifier) @module (#eq? @module "module")
      property: (property_identifier) @exports_prop (#eq? @exports_prop "exports"))
    property: (property_identifier) @name)
  right: (_)) @assign

(assignment_expression
  left: (member_expression
    object: (identifier) @exports (#eq? @exports "exports")
    property: (property_identifier) @name)
  right: (_)) @assign
"#;
```

In the extraction function, after the main symbol loop, add:

```rust
// Second pass: detect CommonJS exports and mark matching symbols as exported
// (or create new exported symbols for inline function assignments)
if matches!(language, Language::JavaScript | Language::Jsx) {
    if let Ok(query) = tree_sitter::Query::new(&ts_lang, JS_COMMONJS_EXPORT_QUERY) {
        let name_idx = query.capture_index_for_name("name");
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            if let Some(name_cap) = name_idx.and_then(|idx| m.captures.iter().find(|c| c.index == idx)) {
                let export_name = name_cap.node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                if export_name.is_empty() { continue; }
                // Mark existing symbol as exported if it matches the name
                if let Some(sym) = symbols.iter_mut().find(|s| s.name == export_name) {
                    sym.is_exported = true;
                } else {
                    // Inline assignment: create a new exported symbol at this line
                    let assign_line = name_cap.node.start_position().row as u32 + 1;
                    symbols.push(SymbolInfo {
                        name: export_name,
                        kind: SymbolKind::Variable,
                        file_path: file_path.to_string(),
                        start_line: assign_line,
                        start_column: name_cap.node.start_position().column as u32,
                        end_line: assign_line,
                        end_column: name_cap.node.end_position().column as u32,
                        is_exported: true,
                    });
                }
            }
        }
    }
}
```

> Read `src/languages/typescript.rs` carefully to find the exact function signature for JavaScript extraction before inserting. The function is called from `GraphBuilder` and has a signature like `pub fn extract_symbols(file_path: &str, source: &str, language: Language) -> Result<Vec<SymbolInfo>>`.

- [ ] **Step 4: Run the test**

```bash
cargo test test_commonjs_exports_marked_as_exported -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Run full tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 6: Commit**

```bash
git add src/languages/typescript.rs src/pipeline/executor.rs
git commit -m "feat(parser): detect CommonJS module.exports/exports assignments as exported symbols"
```

---

## Self-Review Checklist

After writing the plan, verify against the spec:

**Spec coverage:**
- [x] 1A: JS metric gap — Tasks 1, 2
- [x] 1A: C++ metric gap (qualified names) — Task 3
- [x] 1A: C++ forward declarations — Task 4
- [x] 1A: C# metric gap — Task 4
- [x] 1B: `lhs_is_parameter` DSL — Task 5
- [x] 1B: `lhs_is_parameter` executor — Task 6
- [x] 1B: `argument_mutation` update + pipeline audit — Task 7
- [x] 2A: Go `excessive_public_api` threshold — Task 8
- [x] 2B: `print_instead_of_logging` — Task 8
- [x] 2C: `hardcoded_secrets` all 10 languages — Task 9
- [x] 2D: Java `high_coupling` rewrite — Task 8
- [x] 2E: CommonJS `dead_export` — Task 10
- [x] 2F: PHP coupling line fix — Task 8

**Acceptance criteria coverage:**
1. `cargo test` — every task ends with `cargo test 2>&1 | tail -5`
2. JS/C++/C# metric tests — Tasks 1-4
3. `argument_mutation` precision — Task 6 test + Task 7 JSON update
4. Pipeline audit documented — Task 7 Step 2
5. Go API threshold — Task 8 Step 1
6. Python `print` detector — Task 8 Step 4
7. `hardcoded_secrets` all languages — Task 9
8. Java `high_coupling` — Task 8 Step 2
9. CommonJS exports as `is_exported: true` — Task 10
10. PHP coupling lines — Task 8 Step 3
