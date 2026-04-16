//! Integration tests for JSON-driven architecture audit pipelines.
//!
//! Each test exercises the full AuditEngine path end-to-end:
//! workspace loading → graph building → JSON pipeline execution → finding verification.
//!
//! Per D-06: one representative language per pipeline, 4 positive + 4 negative = 8 tests.
//!
//! NOTE: Deviations from the plan's suggested fixtures:
//! - circular_dependencies: uses Python relative imports (`.module_b`) because
//!   `resolve_import` in python.rs only resolves relative imports (absolute = external).
//! - dependency_graph_depth: uses TypeScript instead of Go because Go's `resolve_import`
//!   does not handle `"./b"` style relative paths; TypeScript threshold is gte:6
//!   so the chain needs 7 files (a->b->c->d->e->f->g).

use virgil_cli::{
    audit::engine::{AuditEngine, PipelineSelector},
    graph::builder::GraphBuilder,
    language::Language,
    workspace::Workspace,
};

// ── module_size_distribution (Rust) ──

#[test]
fn module_size_distribution_rust_finds_oversized() {
    let dir = tempfile::tempdir().unwrap();
    // 31 public functions exceeds the 30-symbol threshold
    let content: String = (0..31)
        .map(|i| format!("pub fn func_{i}() {{}}\n"))
        .collect();
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "oversized_module"),
        "expected oversized_module finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn module_size_distribution_rust_clean_file() {
    let dir = tempfile::tempdir().unwrap();
    // 5 functions is well under the 30-symbol threshold
    let content: String = (0..5)
        .map(|i| format!("pub fn func_{i}() {{}}\n"))
        .collect();
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "oversized_module"),
        "expected no oversized_module finding; got findings: {:?}",
        findings.iter().map(|f| (&f.pattern, &f.pipeline)).collect::<Vec<_>>()
    );
}

// ── api_surface_area (TypeScript) ──

#[test]
fn api_surface_area_typescript_finds_excessive() {
    let dir = tempfile::tempdir().unwrap();
    // 11 exported functions in one file, all exported = 100% ratio > 80% threshold
    let content: String = (0..11)
        .map(|i| format!("export function handler_{i}() {{}}\n"))
        .collect();
    std::fs::write(dir.path().join("handlers.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "excessive_public_api"),
        "expected excessive_public_api finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn api_surface_area_typescript_clean_file() {
    let dir = tempfile::tempdir().unwrap();
    // Only 3 exported functions — under the 10-symbol minimum
    let content = "export function a() {}\nexport function b() {}\nexport function c() {}\n";
    std::fs::write(dir.path().join("utils.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "excessive_public_api"),
        "expected no excessive_public_api finding"
    );
}

// ── circular_dependencies (Python) ──
//
// NOTE: Uses relative imports (.module_b / .module_a) because python.rs
// resolve_import only resolves relative imports; absolute imports are treated
// as external and never create intra-workspace Imports graph edges.

#[test]
fn circular_dependencies_python_finds_cycle() {
    let dir = tempfile::tempdir().unwrap();
    // Two Python files that import each other via relative imports
    std::fs::write(
        dir.path().join("module_a.py"),
        "from .module_b import something\ndef func_a(): pass\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("module_b.py"),
        "from .module_a import func_a\ndef something(): pass\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "circular_dependency"),
        "expected circular_dependency finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn circular_dependencies_python_no_cycle() {
    let dir = tempfile::tempdir().unwrap();
    // Single file, no imports — no possible cycle
    std::fs::write(dir.path().join("standalone.py"), "def greet(): pass\n").unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "circular_dependency"),
        "expected no circular_dependency finding"
    );
}

// ── dependency_graph_depth (TypeScript) ──
//
// NOTE: Uses TypeScript instead of Go because go.rs resolve_import does not
// handle "./b" style relative paths (Go uses full module paths). TypeScript's
// threshold is gte:6, so the chain requires 7 files: a->b->c->d->e->f->g.
// File g has depth 6 from a (a is entry, g is 6 hops away).

#[test]
fn dependency_graph_depth_typescript_finds_deep_chain() {
    let dir = tempfile::tempdir().unwrap();
    // Create a chain: a.ts imports b.ts, b imports c, ..., f imports g
    // That gives g a depth of 6 from a (threshold gte:6)
    std::fs::write(
        dir.path().join("a.ts"),
        "import { B } from './b';\nexport function A() { return B(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b.ts"),
        "import { C } from './c';\nexport function B() { return C(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("c.ts"),
        "import { D } from './d';\nexport function C() { return D(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("d.ts"),
        "import { E } from './e';\nexport function D() { return E(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("e.ts"),
        "import { F } from './f';\nexport function E() { return F(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("f.ts"),
        "import { G } from './g';\nexport function F() { return G(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("g.ts"),
        "export function G() { return 42; }\n",
    )
    .unwrap();

    let workspace =
        Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "deep_import_chain"),
        "expected deep_import_chain finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn dependency_graph_depth_typescript_shallow_is_clean() {
    let dir = tempfile::tempdir().unwrap();
    // Single file, no imports — depth 0
    std::fs::write(
        dir.path().join("main.ts"),
        "export function main(): void {}\n",
    )
    .unwrap();

    let workspace =
        Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "deep_import_chain"),
        "expected no deep_import_chain finding"
    );
}

// ── Phase 3: Complexity Pipelines (compute_metric) ──

#[test]
fn cyclomatic_complexity_ts_finds_complex_function() {
    let dir = tempfile::tempdir().unwrap();
    // 12 if-statements = CC of 13 (1 base + 12 decision points), exceeds threshold of 10
    let mut content = String::from("export function complex(x: number) {\n");
    for i in 0..12 {
        content.push_str(&format!("  if (x > {i}) {{ console.log({i}); }}\n"));
    }
    content.push_str("}\n");
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cyclomatic_complexity" && f.pattern == "high_cyclomatic_complexity"),
        "expected high_cyclomatic_complexity finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cyclomatic_complexity_ts_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = "export function simple(x: number) {\n  if (x > 0) { return x; }\n  return 0;\n}\n";
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cyclomatic_complexity" && f.pattern == "high_cyclomatic_complexity"),
        "expected no high_cyclomatic_complexity finding for simple function; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn function_length_ts_finds_long_function() {
    let dir = tempfile::tempdir().unwrap();
    let mut content = String::from("export function longFunc() {\n");
    for i in 0..55 {
        content.push_str(&format!("  const x{i} = {i};\n"));
    }
    content.push_str("}\n");
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "function_length" && f.pattern == "function_too_long"),
        "expected function_too_long finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn function_length_ts_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = "export function short() {\n  return 1;\n}\n";
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "function_length" && f.pattern == "function_too_long"),
        "expected no function_too_long finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cognitive_complexity_ts_finds_complex_function() {
    let dir = tempfile::tempdir().unwrap();
    // Deeply nested: if > for > while > if > if > if produces cognitive complexity well above 15
    let content = r#"export function deepNest(x: number) {
  if (x > 0) {
    for (let i = 0; i < x; i++) {
      while (i > 0) {
        if (i % 2 === 0) {
          if (i % 3 === 0) {
            if (i % 5 === 0) {
              console.log(i);
            }
          }
        }
      }
    }
  }
  if (x > 1) {
    if (x > 2) {
      if (x > 3) {
        console.log(x);
      }
    }
  }
}
"#;
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cognitive_complexity" && f.pattern == "high_cognitive_complexity"),
        "expected high_cognitive_complexity finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cognitive_complexity_ts_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = "export function simple(x: number) {\n  if (x > 0) { return x; }\n  return 0;\n}\n";
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cognitive_complexity" && f.pattern == "high_cognitive_complexity"),
        "expected no high_cognitive_complexity finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn comment_to_code_ratio_ts_finds_under_documented() {
    let dir = tempfile::tempdir().unwrap();
    // 30 lines of code, zero comments = 0% ratio, below 5% threshold
    let mut content = String::new();
    for i in 0..30 {
        content.push_str(&format!("const x{i} = {i};\n"));
    }
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "comment_to_code_ratio" && f.pattern == "comment_ratio_violation"),
        "expected comment_ratio_violation finding for under-documented file; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn comment_to_code_ratio_ts_clean_file() {
    let dir = tempfile::tempdir().unwrap();
    // 5 comment lines + 5 code lines = 50% ratio, within acceptable range (5%-60%)
    let content = "// comment 1\n// comment 2\n// comment 3\n// comment 4\n// comment 5\nconst a = 1;\nconst b = 2;\nconst c = 3;\nconst d = 4;\nconst e = 5;\n";
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "comment_to_code_ratio" && f.pattern == "comment_ratio_violation"),
        "expected no comment_ratio_violation finding for balanced file; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── Phase 3: Scalability Pipelines (match_pattern) ──

#[test]
fn n_plus_one_queries_ts_finds_call_in_loop() {
    let dir = tempfile::tempdir().unwrap();
    let content = r#"
const db = { findOne: (id: number) => ({}) };
const ids = [1, 2, 3];
for (let i = 0; i < ids.length; i++) {
  db.findOne(ids[i]);
}
"#;
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "n_plus_one_queries" && f.pattern == "query_in_loop"),
        "expected query_in_loop finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn n_plus_one_queries_ts_clean_code() {
    let dir = tempfile::tempdir().unwrap();
    let content = "const db = { findOne: (id: number) => ({}) };\nconst user = db.findOne(1);\n";
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "n_plus_one_queries" && f.pattern == "query_in_loop"),
        "expected no query_in_loop finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn sync_blocking_in_async_ts_finds_sync_call() {
    let dir = tempfile::tempdir().unwrap();
    let content = "import * as fs from 'fs';\nfs.readFileSync('test.txt');\n";
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "sync_blocking_in_async" && f.pattern == "sync_call_in_async"),
        "expected sync_call_in_async finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn sync_blocking_in_async_ts_clean_code() {
    let dir = tempfile::tempdir().unwrap();
    // Only async/promise calls, no Sync suffix methods
    let content = "async function load() {\n  const data = await fetch('https://example.com');\n  return data;\n}\n";
    std::fs::write(dir.path().join("test.ts"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "sync_blocking_in_async" && f.pattern == "sync_call_in_async"),
        "expected no sync_call_in_async finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── Phase 4: Rust Security + Scalability Pipelines ──

// ── race_conditions (Rust) ──

#[test]
fn race_conditions_rust_finds_static_mut() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "static mut COUNTER: u32 = 0;\nfn main() {}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "race_conditions" && f.pattern == "static_mut"),
        "expected race_conditions/static_mut finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn race_conditions_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // Immutable static -- no mutable_specifier
    std::fs::write(
        dir.path().join("test.rs"),
        "static COUNTER: u32 = 0;\nfn main() {}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "race_conditions"),
        "expected no race_conditions finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── type_confusion (Rust) ──

#[test]
fn type_confusion_rust_finds_union() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "union MyUnion { i: i32, f: f32 }\nfn main() {}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "type_confusion" && f.pattern == "union_type_confusion"),
        "expected type_confusion/union_type_confusion finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn type_confusion_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No union definitions -- only struct
    std::fs::write(
        dir.path().join("test.rs"),
        "struct Point { x: f32, y: f32 }\nfn main() {}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "type_confusion"),
        "expected no type_confusion finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── unsafe_memory (Rust) ──

#[test]
fn unsafe_memory_rust_finds_unsafe_block() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "fn main() {\n    unsafe { let x = 1; }\n}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "unsafe_memory" && f.pattern == "unsafe_memory_operation"),
        "expected unsafe_memory/unsafe_memory_operation finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn unsafe_memory_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No unsafe blocks at all
    std::fs::write(
        dir.path().join("test.rs"),
        "fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "unsafe_memory"),
        "expected no unsafe_memory finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── integer_overflow (Rust) ──

#[test]
fn integer_overflow_rust_finds_arithmetic() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "fn calc(n: u32) -> u32 { n * 2 }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "integer_overflow" && f.pattern == "unchecked_arithmetic"),
        "expected integer_overflow/unchecked_arithmetic finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn integer_overflow_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No binary expressions -- only a struct with a constant field
    std::fs::write(
        dir.path().join("test.rs"),
        "struct Foo;\nconst MAX: u32 = 100;\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "integer_overflow"),
        "expected no integer_overflow finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── path_traversal (Rust) ──

#[test]
fn path_traversal_rust_finds_method_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "fn f(p: std::path::PathBuf, s: &str) -> std::path::PathBuf { p.join(s) }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "path_traversal" && f.pattern == "unvalidated_path_operation"),
        "expected path_traversal/unvalidated_path_operation finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn path_traversal_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method calls -- only a struct definition and a static
    std::fs::write(
        dir.path().join("test.rs"),
        "struct Config;\nstatic NAME: &str = \"app\";\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "path_traversal"),
        "expected no path_traversal finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── resource_exhaustion (Rust) ──

#[test]
fn resource_exhaustion_rust_finds_scoped_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "fn f(n: usize) -> Vec<u8> { Vec::<u8>::with_capacity(n) }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "resource_exhaustion" && f.pattern == "unbounded_allocation"),
        "expected resource_exhaustion/unbounded_allocation finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn resource_exhaustion_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No scoped identifier calls -- only a simple function with local arithmetic
    std::fs::write(
        dir.path().join("test.rs"),
        "fn add(a: u32, b: u32) -> u32 { a + b }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "resource_exhaustion"),
        "expected no resource_exhaustion finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── panic_dos (Rust) ──

#[test]
fn panic_dos_rust_finds_method_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "fn f(x: Option<u32>) -> u32 { x.unwrap() }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "panic_dos" && f.pattern == "unwrap_untrusted"),
        "expected panic_dos/unwrap_untrusted finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn panic_dos_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method calls at all -- only struct and constant definitions
    std::fs::write(
        dir.path().join("test.rs"),
        "struct Wrapper(u32);\nconst LIMIT: u32 = 1000;\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "panic_dos"),
        "expected no panic_dos finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── toctou (Rust) ──

#[test]
fn toctou_rust_finds_method_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "fn check(p: &std::path::Path) -> bool { p.exists() }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "toctou" && f.pattern == "path_check_use_race"),
        "expected toctou/path_check_use_race finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn toctou_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method calls -- only type definitions
    std::fs::write(
        dir.path().join("test.rs"),
        "type PathStr = String;\nstruct FileInfo { name: String }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "toctou"),
        "expected no toctou finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (Rust, Scalability) ──

#[test]
fn memory_leak_indicators_rust_finds_scoped_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.rs"),
        "fn leak() {\n    let s = String::from(\"hello\");\n    let _: &'static str = Box::leak(s.into_boxed_str());\n}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No scoped calls -- only local variable declarations with literal values
    std::fs::write(
        dir.path().join("test.rs"),
        "fn safe() {\n    let x: u32 = 42;\n    let y: bool = true;\n}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators"),
        "expected no memory_leak_indicators finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── Phase 3: Cross-Language Verification (Rust + Python) ──

#[test]
fn cyclomatic_complexity_rust_finds_complex_function() {
    let dir = tempfile::tempdir().unwrap();
    // 12 if-statements in a Rust function = CC of 13, exceeds threshold of 10
    let mut content = String::from("pub fn complex(x: i32) {\n");
    for i in 0..12 {
        content.push_str(&format!("    if x > {i} {{ println!(\"{i}\"); }}\n"));
    }
    content.push_str("}\n");
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cyclomatic_complexity" && f.pattern == "high_cyclomatic_complexity"),
        "expected high_cyclomatic_complexity finding for Rust; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cyclomatic_complexity_rust_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = "pub fn simple(x: i32) -> i32 {\n    if x > 0 { x } else { 0 }\n}\n";
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cyclomatic_complexity" && f.pattern == "high_cyclomatic_complexity"),
        "expected no high_cyclomatic_complexity finding for simple Rust function; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cyclomatic_complexity_python_finds_complex_function() {
    let dir = tempfile::tempdir().unwrap();
    // 12 if-statements in a Python function = CC of 13, exceeds threshold of 10
    let mut content = String::from("def complex(x):\n");
    for i in 0..12 {
        content.push_str(&format!("    if x > {i}:\n        print({i})\n"));
    }
    std::fs::write(dir.path().join("test.py"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cyclomatic_complexity" && f.pattern == "high_cyclomatic_complexity"),
        "expected high_cyclomatic_complexity finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cyclomatic_complexity_python_clean_function() {
    let dir = tempfile::tempdir().unwrap();
    let content = "def simple(x):\n    if x > 0:\n        return x\n    return 0\n";
    std::fs::write(dir.path().join("test.py"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Complexity)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cyclomatic_complexity" && f.pattern == "high_cyclomatic_complexity"),
        "expected no high_cyclomatic_complexity finding for simple Python function; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── Phase 4: JavaScript/TypeScript Security + Scalability Pipelines ──

// ── command_injection (JavaScript) ──

#[test]
fn command_injection_javascript_finds_exec_call() {
    let dir = tempfile::tempdir().unwrap();
    // exec() method call on child_process object -- triggers exec_command_injection pattern
    std::fs::write(
        dir.path().join("test.js"),
        "const cp = require('child_process');\ncp.exec(userInput, (err, out) => { console.log(out); });\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "exec_command_injection"),
        "expected command_injection/exec_command_injection finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn command_injection_javascript_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls at all -- only variable declarations with literals
    std::fs::write(
        dir.path().join("test.js"),
        "const name = 'world';\nconst greeting = 'hello';\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection"),
        "expected no command_injection finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── code_injection (JavaScript) ──

#[test]
fn code_injection_javascript_finds_direct_call() {
    let dir = tempfile::tempdir().unwrap();
    // Direct function call (identifier-style) -- triggers code_injection_call pattern
    std::fs::write(
        dir.path().join("test.js"),
        "eval(userInput);\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "code_injection" && f.pattern == "code_injection_call"),
        "expected code_injection/code_injection_call finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn code_injection_javascript_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only object/constant declarations
    std::fs::write(
        dir.path().join("test.js"),
        "const obj = { name: 'test', value: 42 };\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "code_injection"),
        "expected no code_injection finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── prototype_pollution (JavaScript) ──

#[test]
fn prototype_pollution_javascript_finds_for_in() {
    let dir = tempfile::tempdir().unwrap();
    // for...in loop -- triggers prototype_pollution_risk pattern
    std::fs::write(
        dir.path().join("test.js"),
        "function merge(target, source) {\n  for (let key in source) {\n    target[key] = source[key];\n  }\n}\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "prototype_pollution" && f.pattern == "prototype_pollution_risk"),
        "expected prototype_pollution/prototype_pollution_risk finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn prototype_pollution_javascript_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No for-in loops -- only a regular for loop and object literal
    std::fs::write(
        dir.path().join("test.js"),
        "const arr = [1, 2, 3];\nfor (let i = 0; i < arr.length; i++) { console.log(arr[i]); }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "prototype_pollution"),
        "expected no prototype_pollution finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── type_system_bypass (TypeScript) ──

#[test]
fn type_system_bypass_typescript_finds_as_expression() {
    let dir = tempfile::tempdir().unwrap();
    // TypeScript 'as' cast -- triggers type_system_bypass pattern
    std::fs::write(
        dir.path().join("test.ts"),
        "const data: unknown = JSON.parse(input);\nconst user = data as User;\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "type_system_bypass" && f.pattern == "type_system_bypass"),
        "expected type_system_bypass/type_system_bypass finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn type_system_bypass_typescript_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No type assertions -- only plain TypeScript with type annotations
    std::fs::write(
        dir.path().join("test.ts"),
        "function greet(name: string): string {\n  return 'hello ' + name;\n}\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::TypeScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::TypeScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::TypeScript])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "type_system_bypass"),
        "expected no type_system_bypass finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (JavaScript, Scalability) ──

#[test]
fn memory_leak_indicators_javascript_finds_direct_call() {
    let dir = tempfile::tempdir().unwrap();
    // Direct function call (setInterval) -- triggers potential_memory_leak pattern
    std::fs::write(
        dir.path().join("test.js"),
        "setInterval(() => { doWork(); }, 1000);\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_javascript_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only variable declarations with literals
    std::fs::write(
        dir.path().join("test.js"),
        "const x = 42;\nconst name = 'hello';\nconst flag = true;\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators"),
        "expected no memory_leak_indicators finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── Phase 4: Go Security + Scalability Pipelines ──

// ── command_injection (Go) ──

#[test]
fn command_injection_go_finds_selector_call() {
    let dir = tempfile::tempdir().unwrap();
    // exec.Command selector call -- triggers exec_command_injection pattern
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nimport \"os/exec\"\nfunc f(cmd string) { exec.Command(cmd) }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "exec_command_injection"),
        "expected command_injection/exec_command_injection finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn command_injection_go_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only type/const declarations
    std::fs::write(
        dir.path().join("test.go"),
        "package main\ntype Config struct{ Name string }\nconst Limit = 100\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".go")),
        "expected no command_injection finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── go_path_traversal (Go) ──

#[test]
fn go_path_traversal_go_finds_selector_call() {
    let dir = tempfile::tempdir().unwrap();
    // filepath.Join selector call -- triggers unvalidated_path_join pattern
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nimport \"path/filepath\"\nfunc serve(p string) { filepath.Join(\"/base\", p) }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "go_path_traversal" && f.pattern == "unvalidated_path_join"),
        "expected go_path_traversal/unvalidated_path_join finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn go_path_traversal_go_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only type and constant declarations
    std::fs::write(
        dir.path().join("test.go"),
        "package main\ntype Handler struct{}\nconst BasePath = \"/srv\"\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "go_path_traversal"),
        "expected no go_path_traversal finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── race_conditions (Go) ──

#[test]
fn race_conditions_go_finds_goroutine_in_loop() {
    let dir = tempfile::tempdir().unwrap();
    // go_statement inside for_statement -- triggers loop_var_capture pattern
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc f() { for i := 0; i < 10; i++ { go func(){} () } }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "race_conditions" && f.pattern == "loop_var_capture"),
        "expected race_conditions/loop_var_capture finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn race_conditions_go_clean() {
    let dir = tempfile::tempdir().unwrap();
    // goroutine outside loop -- no loop_var_capture
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc f() { go func(){} () }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "race_conditions" && f.pattern == "loop_var_capture"),
        "expected no race_conditions/loop_var_capture finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── resource_exhaustion (Go) ──

#[test]
fn resource_exhaustion_go_finds_goroutine_in_loop() {
    let dir = tempfile::tempdir().unwrap();
    // go_statement inside for_statement -- triggers unbounded_goroutine_spawn pattern
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc main() { for i := 0; i < 10; i++ { go func(){} () } }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "resource_exhaustion" && f.pattern == "unbounded_goroutine_spawn"),
        "expected resource_exhaustion/unbounded_goroutine_spawn finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn resource_exhaustion_go_clean() {
    let dir = tempfile::tempdir().unwrap();
    // goroutine outside loop -- no unbounded_goroutine_spawn
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc main() { go func(){} () }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "resource_exhaustion" && f.pattern == "unbounded_goroutine_spawn"),
        "expected no resource_exhaustion/unbounded_goroutine_spawn finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── go_integer_overflow (Go) ──

#[test]
fn go_integer_overflow_go_finds_type_conversion() {
    let dir = tempfile::tempdir().unwrap();
    // int32() type conversion -- triggers narrowing_conversion pattern
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc f() { var x int64; _ = int32(x) }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "go_integer_overflow" && f.pattern == "narrowing_conversion"),
        "expected go_integer_overflow/narrowing_conversion finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn go_integer_overflow_go_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No call expressions at all -- only type and variable declarations
    std::fs::write(
        dir.path().join("test.go"),
        "package main\ntype Point struct{ X, Y int }\nvar origin Point\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "go_integer_overflow"),
        "expected no go_integer_overflow finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── go_type_confusion (Go) ──

#[test]
fn go_type_confusion_go_finds_type_assertion() {
    let dir = tempfile::tempdir().unwrap();
    // Unguarded type assertion x.(string) -- triggers unsafe_pointer_cast pattern
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc f(x interface{}) { v := x.(string); _ = v }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "go_type_confusion" && f.pattern == "unsafe_pointer_cast"),
        "expected go_type_confusion/unsafe_pointer_cast finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn go_type_confusion_go_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No type assertions -- only struct and interface definitions
    std::fs::write(
        dir.path().join("test.go"),
        "package main\ntype Stringer interface{ String() string }\ntype MyStr struct{ val string }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "go_type_confusion"),
        "expected no go_type_confusion finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── Phase 4: Python Security + Scalability Pipelines ──

// ── command_injection (Python) ──

#[test]
fn command_injection_python_finds_attribute_call() {
    let dir = tempfile::tempdir().unwrap();
    // Attribute call on os object -- triggers command_injection_call pattern
    std::fs::write(
        dir.path().join("test.py"),
        "import os\nos.system(user_input)\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn command_injection_python_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only import and constant declarations
    std::fs::write(
        dir.path().join("test.py"),
        "TIMEOUT = 30\nMAX_RETRIES = 3\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".py")),
        "expected no command_injection finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── code_injection (Python) ──

#[test]
fn code_injection_python_finds_direct_call() {
    let dir = tempfile::tempdir().unwrap();
    // Direct identifier call -- triggers code_injection_call pattern
    std::fs::write(
        dir.path().join("test.py"),
        "eval(user_input)\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "code_injection" && f.pattern == "code_injection_call"),
        "expected code_injection/code_injection_call finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn code_injection_python_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only class and attribute definitions
    std::fs::write(
        dir.path().join("test.py"),
        "class Config:\n    debug = False\n    timeout = 30\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "code_injection" && f.file_path.ends_with(".py")),
        "expected no code_injection finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── path_traversal (Python) ──

#[test]
fn path_traversal_python_finds_attribute_call() {
    let dir = tempfile::tempdir().unwrap();
    // Attribute call on path object -- triggers unvalidated_path_join pattern
    // Note: os.path.join uses chained attributes; use single-level path.join for pattern match
    std::fs::write(
        dir.path().join("test.py"),
        "import posixpath as path\npath.join(base_dir, user_path)\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "path_traversal" && f.pattern == "unvalidated_path_join"),
        "expected path_traversal/unvalidated_path_join finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn path_traversal_python_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only constant and type definitions
    std::fs::write(
        dir.path().join("test.py"),
        "BASE_DIR = '/srv/app'\nMAX_SIZE = 1024\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "path_traversal" && f.file_path.ends_with(".py")),
        "expected no path_traversal finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── insecure_deserialization (Python) ──

#[test]
fn insecure_deserialization_python_finds_attribute_call() {
    let dir = tempfile::tempdir().unwrap();
    // pickle.loads attribute call -- triggers insecure_deserialization pattern
    std::fs::write(
        dir.path().join("test.py"),
        "import pickle\npickle.loads(data)\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.pattern == "insecure_deserialization"),
        "expected insecure_deserialization finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn insecure_deserialization_python_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only variable and constant declarations
    std::fs::write(
        dir.path().join("test.py"),
        "FORMAT = 'json'\nVERSION = '1.0'\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.file_path.ends_with(".py")),
        "expected no insecure_deserialization finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── xxe_format_string (Python) ──

#[test]
fn xxe_format_string_python_finds_attribute_call() {
    let dir = tempfile::tempdir().unwrap();
    // ET.fromstring attribute call -- triggers xxe_format_string pattern
    std::fs::write(
        dir.path().join("test.py"),
        "import xml.etree.ElementTree as ET\nET.fromstring(user_data)\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "xxe_format_string" && f.pattern == "xxe_format_string"),
        "expected xxe_format_string finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn xxe_format_string_python_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only constant declarations
    std::fs::write(
        dir.path().join("test.py"),
        "XML_VERSION = '1.0'\nENCODING = 'utf-8'\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "xxe_format_string" && f.file_path.ends_with(".py")),
        "expected no xxe_format_string finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── resource_exhaustion (Python, ReDoS) ──

#[test]
fn resource_exhaustion_python_finds_re_call() {
    let dir = tempfile::tempdir().unwrap();
    // re.compile attribute call -- triggers redos_pattern
    std::fs::write(
        dir.path().join("test.py"),
        "import re\nre.compile(pattern)\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "resource_exhaustion" && f.pattern == "redos_pattern"),
        "expected resource_exhaustion/redos_pattern finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn resource_exhaustion_python_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only constant declarations
    std::fs::write(
        dir.path().join("test.py"),
        "TIMEOUT = 30\nMAX_ATTEMPTS = 3\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "resource_exhaustion" && f.file_path.ends_with(".py")),
        "expected no resource_exhaustion finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (Python, Scalability) ──

#[test]
fn memory_leak_indicators_python_finds_open_call() {
    let dir = tempfile::tempdir().unwrap();
    // open() direct call -- triggers potential_memory_leak pattern
    std::fs::write(
        dir.path().join("test.py"),
        "f = open('file.txt')\ndata = f.read()\nf.close()\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_python_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only constant and class attribute declarations
    std::fs::write(
        dir.path().join("test.py"),
        "BUFFER_SIZE = 4096\nENCODING = 'utf-8'\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.file_path.ends_with(".py")),
        "expected no memory_leak_indicators finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (Go, Scalability) ──

#[test]
fn memory_leak_indicators_go_finds_goroutine_in_loop() {
    let dir = tempfile::tempdir().unwrap();
    // go_statement inside for_statement -- triggers potential_memory_leak pattern
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc f() { for i := 0; i < 10; i++ { go func(){} () } }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_go_clean() {
    let dir = tempfile::tempdir().unwrap();
    // goroutine outside loop -- no potential_memory_leak
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nfunc f() { go func(){} () }\n",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected no memory_leak_indicators/potential_memory_leak finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── Phase 4: Java Security + Scalability Pipelines ──

// ── command_injection (Java) ──

#[test]
fn command_injection_java_finds_method_invocation() {
    let dir = tempfile::tempdir().unwrap();
    // method_invocation node -- triggers command_injection_call pattern
    std::fs::write(
        dir.path().join("test.java"),
        "class A { void f(String cmd) { Runtime.getRuntime().exec(cmd); } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn command_injection_java_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method invocations -- only field and constant declarations
    std::fs::write(
        dir.path().join("test.java"),
        "class Config { static final int MAX = 100; String name = \"app\"; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".java")),
        "expected no command_injection finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── weak_cryptography (Java) ──

#[test]
fn weak_cryptography_java_finds_method_invocation() {
    let dir = tempfile::tempdir().unwrap();
    // method_invocation with getInstance -- triggers weak_crypto_usage pattern
    std::fs::write(
        dir.path().join("test.java"),
        "import java.security.*;\nclass A { void f() { MessageDigest.getInstance(\"MD5\"); } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "weak_cryptography" && f.pattern == "weak_crypto_usage"),
        "expected weak_cryptography/weak_crypto_usage finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn weak_cryptography_java_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method invocations -- only interface definition
    std::fs::write(
        dir.path().join("test.java"),
        "interface Hasher { String ALGO = \"SHA-256\"; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "weak_cryptography" && f.file_path.ends_with(".java")),
        "expected no weak_cryptography finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── insecure_deserialization (Java) ──

#[test]
fn insecure_deserialization_java_finds_method_invocation() {
    let dir = tempfile::tempdir().unwrap();
    // method_invocation readObject -- triggers insecure_deserialization pattern
    std::fs::write(
        dir.path().join("test.java"),
        "import java.io.*;\nclass A { void f(InputStream in) throws Exception { ObjectInputStream ois = new ObjectInputStream(in); Object obj = ois.readObject(); } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.pattern == "insecure_deserialization"),
        "expected insecure_deserialization finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn insecure_deserialization_java_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method invocations -- only class with constant fields
    std::fs::write(
        dir.path().join("test.java"),
        "class Config { static final String FORMAT = \"json\"; static final int VERSION = 1; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.file_path.ends_with(".java")),
        "expected no insecure_deserialization finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── java_path_traversal (Java) ──

#[test]
fn java_path_traversal_java_finds_object_creation() {
    let dir = tempfile::tempdir().unwrap();
    // object_creation_expression for File -- triggers unvalidated_path_operation pattern
    std::fs::write(
        dir.path().join("test.java"),
        "import java.io.*;\nclass A { void f(String name) { File f = new File(\"/uploads/\" + name); } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "java_path_traversal" && f.pattern == "unvalidated_path_operation"),
        "expected java_path_traversal/unvalidated_path_operation finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn java_path_traversal_java_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No object creation expressions -- only interface and enum definitions
    std::fs::write(
        dir.path().join("test.java"),
        "interface PathUtil { String BASE = \"/srv\"; }\nenum Mode { READ, WRITE }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "java_path_traversal" && f.file_path.ends_with(".java")),
        "expected no java_path_traversal finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── reflection_injection (Java) ──

#[test]
fn reflection_injection_java_finds_method_invocation() {
    let dir = tempfile::tempdir().unwrap();
    // method_invocation forName -- triggers reflection_injection pattern
    std::fs::write(
        dir.path().join("test.java"),
        "class A { void f(String className) throws Exception { Class.forName(className); } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "reflection_injection" && f.pattern == "reflection_injection"),
        "expected reflection_injection finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn reflection_injection_java_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method invocations -- only constant and type declarations
    std::fs::write(
        dir.path().join("test.java"),
        "class Registry { static final String TYPE = \"service\"; int id = 0; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "reflection_injection" && f.file_path.ends_with(".java")),
        "expected no reflection_injection finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── java_race_conditions (Java) ──

#[test]
fn java_race_conditions_java_finds_generic_field() {
    let dir = tempfile::tempdir().unwrap();
    // generic field_declaration (HashMap<String, String>) -- triggers thread_unsafe_collection pattern
    std::fs::write(
        dir.path().join("test.java"),
        "import java.util.*;\nclass Server implements Runnable { private HashMap<String, String> cache; public void run() {} }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "java_race_conditions" && f.pattern == "thread_unsafe_collection"),
        "expected java_race_conditions/thread_unsafe_collection finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn java_race_conditions_java_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No field declarations at all -- only an interface with a method signature
    std::fs::write(
        dir.path().join("test.java"),
        "interface Processor { void process(String input); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "java_race_conditions" && f.file_path.ends_with(".java")),
        "expected no java_race_conditions finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (Java, Scalability) ──

#[test]
fn memory_leak_indicators_java_finds_resource_creation() {
    let dir = tempfile::tempdir().unwrap();
    // object_creation_expression for ObjectInputStream -- triggers potential_memory_leak pattern
    std::fs::write(
        dir.path().join("test.java"),
        "import java.io.*;\nclass A { void f(InputStream in) throws Exception { ObjectInputStream ois = new ObjectInputStream(in); } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_java_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No local variable declarations with object creation -- only class with constants
    std::fs::write(
        dir.path().join("test.java"),
        "class Config { static final int MAX = 100; static final String TAG = \"app\"; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.file_path.ends_with(".java")),
        "expected no memory_leak_indicators finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── C Security Pipelines ──

// ── format_string (C, Security) ──

#[test]
fn format_string_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdio.h>\nvoid f(char *s) { printf(s); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "format_string" && f.pattern == "format_string_vulnerability"),
        "expected format_string/format_string_vulnerability finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn format_string_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only a struct definition
    std::fs::write(
        dir.path().join("test.c"),
        "struct Point { int x; int y; };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "format_string" && f.file_path.ends_with(".c")),
        "expected no format_string finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_command_injection (C, Security) ──

#[test]
fn c_command_injection_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdlib.h>\nvoid f(char *cmd) { system(cmd); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_command_injection" && f.pattern == "command_injection_call"),
        "expected c_command_injection/command_injection_call finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_command_injection_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only a typedef
    std::fs::write(
        dir.path().join("test.c"),
        "typedef unsigned int uint32_t;",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_command_injection" && f.file_path.ends_with(".c")),
        "expected no c_command_injection finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_buffer_overflow_security (C, Security) ──

#[test]
fn c_buffer_overflow_security_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <string.h>\nvoid f(char *s) { char buf[10]; strcpy(buf, s); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_buffer_overflow_security" && f.pattern == "buffer_overflow_risk"),
        "expected c_buffer_overflow_security/buffer_overflow_risk finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_buffer_overflow_security_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only an enum definition
    std::fs::write(
        dir.path().join("test.c"),
        "enum Color { RED, GREEN, BLUE };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_buffer_overflow_security" && f.file_path.ends_with(".c")),
        "expected no c_buffer_overflow_security finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_integer_overflow (C, Security) ──

#[test]
fn c_integer_overflow_c_finds_binary_expr() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdlib.h>\nvoid f(int n, int m) { char *p = malloc(n * m); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_integer_overflow" && f.pattern == "unchecked_arithmetic"),
        "expected c_integer_overflow/unchecked_arithmetic finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_integer_overflow_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No binary expressions -- only a constant declaration
    std::fs::write(
        dir.path().join("test.c"),
        "#define MAX_SIZE 100\ntypedef int MyInt;",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_integer_overflow" && f.file_path.ends_with(".c")),
        "expected no c_integer_overflow finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_toctou (C, Security) ──

#[test]
fn c_toctou_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <unistd.h>\n#include <stdio.h>\nvoid f(const char *path) { if (access(path, R_OK) == 0) { FILE *fp = fopen(path, \"r\"); } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_toctou" && f.pattern == "toctou_check"),
        "expected c_toctou/toctou_check finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_toctou_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only struct and global constant
    std::fs::write(
        dir.path().join("test.c"),
        "struct Config { int version; int max_retries; };\nstatic const int DEFAULT_VERSION = 1;",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_toctou" && f.file_path.ends_with(".c")),
        "expected no c_toctou finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (C, Scalability) ──

#[test]
fn memory_leak_indicators_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdlib.h>\nvoid f() { int *p = malloc(sizeof(int)); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only a struct and an enum
    std::fs::write(
        dir.path().join("test.c"),
        "struct Node { int val; struct Node *next; };\nenum Status { OK, ERROR, PENDING };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.file_path.ends_with(".c")),
        "expected no memory_leak_indicators finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_weak_randomness (C, Security) ──

#[test]
fn c_weak_randomness_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdlib.h>\nvoid generate_auth_token() { int x = rand(); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_weak_randomness" && f.pattern == "weak_randomness"),
        "expected c_weak_randomness/weak_randomness finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_weak_randomness_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only a macro definition
    std::fs::write(
        dir.path().join("test.c"),
        "#define BUFFER_SIZE 256\n#define MAX_CONNECTIONS 100",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_weak_randomness" && f.file_path.ends_with(".c")),
        "expected no c_weak_randomness finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_memory_mismanagement (C, Security) ──

#[test]
fn c_memory_mismanagement_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdlib.h>\nvoid f() { int *p = malloc(sizeof(int)); free(p); free(p); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_memory_mismanagement" && f.pattern == "memory_mismanagement"),
        "expected c_memory_mismanagement/memory_mismanagement finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_memory_mismanagement_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only a struct definition with initializer
    std::fs::write(
        dir.path().join("test.c"),
        "typedef struct { int x; int y; } Point;",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_memory_mismanagement" && f.file_path.ends_with(".c")),
        "expected no c_memory_mismanagement finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_path_traversal (C, Security) ──

#[test]
fn c_path_traversal_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdio.h>\nvoid read_file(const char *path) { FILE *fp = fopen(path, \"r\"); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_path_traversal" && f.pattern == "path_traversal_risk"),
        "expected c_path_traversal/path_traversal_risk finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_path_traversal_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only global variable declarations
    std::fs::write(
        dir.path().join("test.c"),
        "static int global_counter = 0;\nstatic const char *app_name = \"myapp\";",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_path_traversal" && f.file_path.ends_with(".c")),
        "expected no c_path_traversal finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── c_uninitialized_memory (C, Security) ──

#[test]
fn c_uninitialized_memory_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdlib.h>\nvoid f(int size) { char *buf = malloc(size); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "c_uninitialized_memory" && f.pattern == "uninitialized_memory"),
        "expected c_uninitialized_memory/uninitialized_memory finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn c_uninitialized_memory_c_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only a union definition
    std::fs::write(
        dir.path().join("test.c"),
        "union Data { int i; float f; char c; };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "c_uninitialized_memory" && f.file_path.ends_with(".c")),
        "expected no c_uninitialized_memory finding for clean C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_injection (C++, Security) ──

#[test]
fn cpp_injection_cpp_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "#include <cstdlib>\nvoid f(const char *cmd) { system(cmd); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_injection" && f.pattern == "command_injection_call"),
        "expected cpp_injection/command_injection_call finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_injection_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only a class definition
    std::fs::write(
        dir.path().join("test.cpp"),
        "class Foo { int x; int y; };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_injection" && f.file_path.ends_with(".cpp")),
        "expected no cpp_injection finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_buffer_overflow (C++, Security) ──

#[test]
fn cpp_buffer_overflow_cpp_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "#include <cstring>\nvoid f(char *s) { char buf[10]; strcpy(buf, s); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_buffer_overflow" && f.pattern == "buffer_overflow_risk"),
        "expected cpp_buffer_overflow/buffer_overflow_risk finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_buffer_overflow_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only an enum definition
    std::fs::write(
        dir.path().join("test.cpp"),
        "enum class Color { Red, Green, Blue };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_buffer_overflow" && f.file_path.ends_with(".cpp")),
        "expected no cpp_buffer_overflow finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_exception_safety (C++, Security) ──

#[test]
fn cpp_exception_safety_cpp_finds_new() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "void f() { int *p = new int(42); delete p; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_exception_safety" && f.pattern == "unguarded_allocation"),
        "expected cpp_exception_safety/unguarded_allocation finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_exception_safety_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No new expressions -- only a namespace and constant
    std::fs::write(
        dir.path().join("test.cpp"),
        "namespace config { constexpr int MAX_RETRIES = 3; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_exception_safety" && f.file_path.ends_with(".cpp")),
        "expected no cpp_exception_safety finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_memory_mismanagement (C++, Security) ──

#[test]
fn cpp_memory_mismanagement_cpp_finds_delete() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "void f() { int *p = new int(42); delete p; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_memory_mismanagement" && f.pattern == "memory_mismanagement"),
        "expected cpp_memory_mismanagement/memory_mismanagement finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_memory_mismanagement_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No delete expressions -- only a struct definition
    std::fs::write(
        dir.path().join("test.cpp"),
        "struct Point { int x; int y; };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_memory_mismanagement" && f.file_path.ends_with(".cpp")),
        "expected no cpp_memory_mismanagement finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_race_conditions (C++, Security) ──

#[test]
fn cpp_race_conditions_cpp_finds_field() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "class Counter { int count; };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_race_conditions" && f.pattern == "thread_unsafe_field"),
        "expected cpp_race_conditions/thread_unsafe_field finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_race_conditions_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No class definitions -- only a function and typedef
    std::fs::write(
        dir.path().join("test.cpp"),
        "typedef unsigned int uint32_t;\nvoid noop() {}",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_race_conditions" && f.file_path.ends_with(".cpp")),
        "expected no cpp_race_conditions finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_path_traversal (C++, Security) ──

#[test]
fn cpp_path_traversal_cpp_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "#include <cstdio>\nvoid read_file(const char *path) { FILE *fp = fopen(path, \"r\"); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_path_traversal" && f.pattern == "path_traversal_risk"),
        "expected cpp_path_traversal/path_traversal_risk finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_path_traversal_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only global variable declarations
    std::fs::write(
        dir.path().join("test.cpp"),
        "static int global_counter = 0;\nconst int MAX_SIZE = 100;",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_path_traversal" && f.file_path.ends_with(".cpp")),
        "expected no cpp_path_traversal finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (C++, Scalability) ──

#[test]
fn memory_leak_indicators_cpp_finds_new() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "void f() { for (int i=0; i<10; i++) { int *p = new int; } }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No new expressions -- only a struct and an enum
    std::fs::write(
        dir.path().join("test.cpp"),
        "struct Node { int val; };\nenum Status { OK, ERROR };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.file_path.ends_with(".cpp")),
        "expected no memory_leak_indicators finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_weak_randomness (C++, Security) ──

#[test]
fn cpp_weak_randomness_cpp_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "#include <cstdlib>\nvoid generate_token() { int x = rand(); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_weak_randomness" && f.pattern == "weak_randomness"),
        "expected cpp_weak_randomness/weak_randomness finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_weak_randomness_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only macro definitions
    std::fs::write(
        dir.path().join("test.cpp"),
        "#define BUFFER_SIZE 256\n#define MAX_CONNECTIONS 100",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_weak_randomness" && f.file_path.ends_with(".cpp")),
        "expected no cpp_weak_randomness finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_type_confusion (C++, Security) ──

#[test]
fn cpp_type_confusion_cpp_finds_new() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "void f() { int x = 42; float *fp = reinterpret_cast<float*>(new int(x)); }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_type_confusion" && f.pattern == "type_confusion_cast"),
        "expected cpp_type_confusion/type_confusion_cast finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_type_confusion_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No new expressions -- only a union definition
    std::fs::write(
        dir.path().join("test.cpp"),
        "union Data { int i; float f; };",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_type_confusion" && f.file_path.ends_with(".cpp")),
        "expected no cpp_type_confusion finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── cpp_integer_overflow (C++, Security) ──

#[test]
fn cpp_integer_overflow_cpp_finds_new() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cpp"),
        "void f(int w, int h) { auto p = new int[w * h]; delete[] p; }",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_integer_overflow" && f.pattern == "unchecked_arithmetic"),
        "expected cpp_integer_overflow/unchecked_arithmetic finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn cpp_integer_overflow_cpp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No new expressions -- only constant definitions
    std::fs::write(
        dir.path().join("test.cpp"),
        "#define MAX_SIZE 100\ntypedef int MyInt;",
    )
    .unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_integer_overflow" && f.file_path.ends_with(".cpp")),
        "expected no cpp_integer_overflow finding for clean C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}

// ── Phase 4 Plan 08: C# Security + Scalability Pipelines ──

// ── command_injection (C#) ──

#[test]
fn command_injection_csharp_finds_invocation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cs"),
        "using System.Diagnostics;\nclass A { void F(string cmd) { Process.Start(\"cmd.exe\", cmd); } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn command_injection_csharp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method calls at all -- only struct/const/type definitions
    std::fs::write(
        dir.path().join("test.cs"),
        "class Config { const int MAX = 100; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".cs")),
        "expected no command_injection finding for clean C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── weak_cryptography (C#) ──

#[test]
fn weak_cryptography_csharp_finds_invocation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cs"),
        "using System.Security.Cryptography;\nclass A { void F() { MD5.Create(); } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "weak_cryptography" && f.pattern == "weak_crypto_usage"),
        "expected weak_cryptography/weak_crypto_usage finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn weak_cryptography_csharp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method calls -- only class with a constant
    std::fs::write(
        dir.path().join("test.cs"),
        "class Crypto { const string ALGO = \"AES256\"; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "weak_cryptography" && f.file_path.ends_with(".cs")),
        "expected no weak_cryptography finding for clean C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── insecure_deserialization (C#) ──

#[test]
fn insecure_deserialization_csharp_finds_object_creation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cs"),
        "class A { void F(Stream s) { var bf = new BinaryFormatter(); } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.pattern == "insecure_deserialization"),
        "expected insecure_deserialization finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn insecure_deserialization_csharp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No object creation expressions -- only interface/type definitions
    std::fs::write(
        dir.path().join("test.cs"),
        "interface ISerializer { void Serialize(object obj); }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.file_path.ends_with(".cs")),
        "expected no insecure_deserialization finding for clean C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── csharp_path_traversal (C#) ──

#[test]
fn csharp_path_traversal_csharp_finds_invocation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cs"),
        "class A { void F(string name) { File.ReadAllText(\"/uploads/\" + name); } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "csharp_path_traversal" && f.pattern == "path_traversal_risk"),
        "expected csharp_path_traversal/path_traversal_risk finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn csharp_path_traversal_csharp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method calls -- only type/interface definitions
    std::fs::write(
        dir.path().join("test.cs"),
        "interface IFileService { string BasePath { get; } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "csharp_path_traversal" && f.file_path.ends_with(".cs")),
        "expected no csharp_path_traversal finding for clean C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── csharp_race_conditions (C#) ──

#[test]
fn csharp_race_conditions_csharp_finds_field_decl() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cs"),
        "class Server { private Dictionary<string, string> _cache; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "csharp_race_conditions" && f.pattern == "thread_unsafe_field"),
        "expected csharp_race_conditions/thread_unsafe_field finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn csharp_race_conditions_csharp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No field declarations -- only method signatures
    std::fs::write(
        dir.path().join("test.cs"),
        "interface ICache { void Clear(); }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "csharp_race_conditions" && f.file_path.ends_with(".cs")),
        "expected no csharp_race_conditions finding for clean C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── reflection_unsafe (C#) ──

#[test]
fn reflection_unsafe_csharp_finds_invocation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cs"),
        "class A { void F(string typeName) { Type.GetType(typeName); } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "reflection_unsafe" && f.pattern == "reflection_injection"),
        "expected reflection_unsafe/reflection_injection finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn reflection_unsafe_csharp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No method calls -- only type/enum definitions
    std::fs::write(
        dir.path().join("test.cs"),
        "enum LoadMode { Static, Dynamic }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "reflection_unsafe" && f.file_path.ends_with(".cs")),
        "expected no reflection_unsafe finding for clean C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (C#, Scalability) ──

#[test]
fn memory_leak_indicators_csharp_finds_object_creation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.cs"),
        "using System.Data.SqlClient;\nclass A { void F() { var c = new SqlConnection(\"connstr\"); } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_csharp_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No object creation expressions -- only interface/type definitions
    std::fs::write(
        dir.path().join("test.cs"),
        "interface IConnection { void Open(); void Close(); }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.file_path.ends_with(".cs")),
        "expected no memory_leak_indicators finding for clean C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── command_injection (PHP, Security) ──

#[test]
fn command_injection_php_finds_function_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction f($cmd) { system($cmd); }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn command_injection_php_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only class definitions with no method body
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nclass SafeClass {}\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".php")),
        "expected no command_injection finding for clean PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── unsafe_include (PHP, Security) ──

#[test]
fn unsafe_include_php_finds_include_expression() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction f($path) { include($path); }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "unsafe_include" && f.pattern == "unsafe_include"),
        "expected unsafe_include finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn unsafe_include_php_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No include/require -- only class definition
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nclass Config { public $value = 'test'; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "unsafe_include" && f.file_path.ends_with(".php")),
        "expected no unsafe_include finding for clean PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── type_juggling (PHP, Security) ──

#[test]
fn type_juggling_php_finds_binary_expression() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction f($a, $b) { if ($a == $b) { return true; } }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "type_juggling" && f.pattern == "loose_comparison"),
        "expected type_juggling/loose_comparison finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn type_juggling_php_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No binary expressions -- only interface definition
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\ninterface Comparable { public function compare(): int; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "type_juggling" && f.file_path.ends_with(".php")),
        "expected no type_juggling finding for clean PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── unescaped_output (PHP, Security) ──

#[test]
fn unescaped_output_php_finds_echo_statement() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction f($name) { echo $name; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "unescaped_output" && f.pattern == "unescaped_output"),
        "expected unescaped_output finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn unescaped_output_php_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No echo statements -- only class with constant
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nclass View { const TEMPLATE = 'base'; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "unescaped_output" && f.file_path.ends_with(".php")),
        "expected no unescaped_output finding for clean PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── session_auth (PHP, Security) ──

#[test]
fn session_auth_php_finds_function_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction login($pw) { $hash = md5($pw); return $hash; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "session_auth" && f.pattern == "session_management"),
        "expected session_auth/session_management finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn session_auth_php_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only trait definition
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\ntrait Authenticatable { public $remember_token; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "session_auth" && f.file_path.ends_with(".php")),
        "expected no session_auth finding for clean PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── insecure_deserialization (PHP, Security) ──

#[test]
fn insecure_deserialization_php_finds_function_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction load($data) { return unserialize($data); }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.pattern == "insecure_deserialization"),
        "expected insecure_deserialization finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn insecure_deserialization_php_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only enum definition
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nenum Status { case Active; case Inactive; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Security)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "insecure_deserialization" && f.file_path.ends_with(".php")),
        "expected no insecure_deserialization finding for clean PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

// ── memory_leak_indicators (PHP, Scalability) ──

#[test]
fn memory_leak_indicators_php_finds_function_call() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction f() { $fh = fopen('file.txt', 'r'); }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.pattern == "potential_memory_leak"),
        "expected memory_leak_indicators/potential_memory_leak finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}

#[test]
fn memory_leak_indicators_php_clean() {
    let dir = tempfile::tempdir().unwrap();
    // No function calls -- only interface definition
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\ninterface Resource { public function getId(): int; }\n",
    )
    .unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .pipeline_selector(PipelineSelector::Scalability)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "memory_leak_indicators" && f.file_path.ends_with(".php")),
        "expected no memory_leak_indicators finding for clean PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
