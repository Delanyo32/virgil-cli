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
