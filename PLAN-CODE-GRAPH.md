# Plan: Add petgraph-based CodeGraph to virgil-cli

## Context

Virgil-cli's audit system detects vulnerabilities per-file using tree-sitter pattern matching. This produces false positives because it cannot trace data flow across functions or files. For example, the SQL injection pipeline flags any non-literal argument to `db.query()`, even if the value was sanitized upstream. Adding a petgraph-backed `CodeGraph` enables taint analysis, resource lifecycle tracking, guard/sanitizer detection, cross-file dead code analysis, and complexity aggregation — improving accuracy across 100+ detection patterns.

## Design Decisions

- **petgraph** embedded `DiGraph<NodeWeight, EdgeWeight>`, single graph
- **Replace** `ProjectIndex` entirely — delete `project_index.rs`, `index_builder.rs`, `call_graph.rs`
- **Level 3 data flow** — full CFG per function with branch-dependent flow and phi nodes
- **Per-language CFG builders** for all 11 language groups (TS/JS/TSX/JSX share one)
- **Hardcoded** taint sources, sinks, sanitizers per language
- **PipelineContext** struct — new `check_with_context()` method on Pipeline trait with default delegation to `check()` (zero breakage across 304 pipeline impls)
- **Always build CodeGraph** for every query and audit (consistency over conditional logic)
- **Full graph at server startup**
- **Delete call_graph.rs** — traversal moves into `CodeGraph` methods

## Graph Model

### Node Types

| Node Type | What it represents | Created from |
|-----------|-------------------|--------------|
| `File` | Source file | Workspace file list |
| `Symbol` | Function, method, class, struct, etc. | Existing symbol extraction |
| `CallSite` | A specific function/method invocation | New: tree-sitter call_expression queries |
| `Parameter` | Function parameter | New: tree-sitter parameter queries |
| `ExternalSource` | Taint origin (req.body, argv, env, stdin) | New: configurable source patterns |

### Edge Types

| Edge Type | From → To | Purpose | Pipelines improved |
|-----------|----------|---------|-------------------|
| `DefinedIn` | Symbol → File | Containment | dead code, architecture |
| `Calls` | Symbol → Symbol | Call graph | complexity, dead code, resource lifecycle |
| `Imports` | File → File | Dependency | circular deps, coupling |
| `FlowsTo` | Parameter/ExternalSource → CallSite | Taint propagation | all 14 security pipelines |
| `SanitizedBy` | Value → Symbol | Taint break | security false positive reduction |
| `Exports` | File → Symbol | Public API | dead exports, API surface |
| `Acquires` | CallSite → Resource type | Resource tracking | all 6 leak pipelines |
| `ReleasedBy` | Resource → CallSite | Cleanup tracking | all 6 leak pipelines |
| `Contains` | File → Symbol | Parent containment | architecture |

### Graph Build Order

1. Files + Symbols + Parameters (from existing tree-sitter extraction) — parallelizable per file
2. DefinedIn + Exports + Contains edges
3. Imports edges (from existing import resolution)
4. Calls edges (improved from call_graph.rs, name-based resolution via `symbols_by_name`)
5. CFG per function (NEW: per-language CFG builders) — parallelizable per file
6. FlowsTo + SanitizedBy edges (requires steps 1-5, taint engine)
7. Acquires + ReleasedBy edges (requires steps 4-5, resource analyzer)

## Module Structure

```
src/
├── graph/
│   ├── mod.rs              # CodeGraph struct, NodeWeight, EdgeWeight enums, traversal methods
│   ├── builder.rs          # GraphBuilder: orchestrates steps 1-7
│   ├── taint.rs            # Taint propagation engine + hardcoded source/sink/sanitizer tables
│   ├── resource.rs         # Resource lifecycle analysis (acquire/release tracking)
│   ├── cfg.rs              # CFG types (BasicBlock, CfgStatement, CfgEdge, FunctionCfg)
│   └── cfg_languages/
│       ├── mod.rs           # CfgBuilder trait + dispatch
│       ├── typescript.rs    # JS/TS/TSX/JSX CFG builder
│       ├── python.rs        # Python CFG builder
│       ├── rust_lang.rs     # Rust CFG builder
│       ├── go.rs            # Go CFG builder
│       ├── java.rs          # Java CFG builder
│       ├── c_lang.rs        # C CFG builder
│       ├── cpp.rs           # C++ CFG builder
│       ├── csharp.rs        # C# CFG builder
│       └── php.rs           # PHP CFG builder
```

## Phase 0: Foundation — types and dependency

**Goal**: Add petgraph, define all graph/CFG types. No behavioral changes.

### New files

**`src/graph/mod.rs`**
```rust
pub mod builder;
pub mod cfg;
pub mod cfg_languages;
pub mod taint;
pub mod resource;

use std::collections::HashMap;
use petgraph::graph::{DiGraph, NodeIndex};
use crate::language::Language;
use crate::models::SymbolKind;

#[derive(Debug, Clone)]
pub enum SourceKind {
    UserInput, DatabaseRead, FileRead, EnvironmentVar, NetworkRead, Deserialization,
}

#[derive(Debug, Clone)]
pub enum NodeWeight {
    File { path: String, language: Language },
    Symbol { name: String, kind: SymbolKind, file_path: String, start_line: u32, end_line: u32, exported: bool },
    CallSite { name: String, file_path: String, line: u32 },
    Parameter { name: String, function_node: NodeIndex, position: usize, is_taint_source: bool },
    ExternalSource { kind: SourceKind, file_path: String, line: u32 },
}

#[derive(Debug, Clone)]
pub enum EdgeWeight {
    DefinedIn,
    Calls,
    Imports,
    FlowsTo,
    SanitizedBy { sanitizer: String },
    Exports,
    Acquires { resource_type: String },
    ReleasedBy,
    Contains,
}

pub struct CodeGraph {
    pub graph: DiGraph<NodeWeight, EdgeWeight>,
    pub file_nodes: HashMap<String, NodeIndex>,
    pub symbol_nodes: HashMap<(String, u32), NodeIndex>,   // (file_path, start_line) -> NodeIndex
    pub symbols_by_name: HashMap<String, Vec<NodeIndex>>,
    pub function_cfgs: HashMap<NodeIndex, cfg::FunctionCfg>,
}
```

**`src/graph/cfg.rs`** — BasicBlock, CfgStatement, CfgStatementKind (Assignment/Call/Return/Guard/ResourceAcquire/ResourceRelease/PhiNode), CfgEdge (Normal/TrueBranch/FalseBranch/Exception/Cleanup), FunctionCfg (petgraph DiGraph<BasicBlock, CfgEdge> + entry/exits)

**`src/graph/builder.rs`** — Empty `GraphBuilder::build()` returning `CodeGraph::new()`

**`src/graph/taint.rs`** — Empty `TaintEngine` struct, hardcoded source/sink/sanitizer tables (populated later)

**`src/graph/resource.rs`** — Empty `ResourceAnalyzer` struct

**`src/graph/cfg_languages/mod.rs`** — `CfgBuilder` trait definition + `cfg_builder_for_language()` dispatch

### Modified files
- **`Cargo.toml`** — add `petgraph = "0.7"`
- **`src/lib.rs`** — add `pub mod graph;`

### Verify
`cargo build` succeeds. `cargo test` passes unchanged.

---

## Phase 1: GraphBuilder — files, symbols, imports, calls

**Goal**: Build the same data currently in `ProjectIndex` plus `CALLS` edges into the `CodeGraph`. Reuse existing extraction from `languages/mod.rs`.

### `src/graph/builder.rs` — full implementation

**Per-file extraction** (parallel via rayon, same pattern as `index_builder.rs:48-91`):
1. Parse with `parser::create_parser(lang)`
2. `languages::extract_symbols()` → symbols
3. `languages::extract_imports()` → imports
4. Extract call sites within each symbol's line range (port logic from `call_graph::collect_calls_in_range`)
5. Extract parameters from function nodes
6. Return `FileGraphData { path, language, symbols, imports, call_sites, parameters, line_count }`

**Graph assembly** (single-threaded — DiGraph is not Sync):
1. Add `NodeWeight::File` per file → store in `file_nodes`
2. Add `NodeWeight::Symbol` per symbol → `DefinedIn` edge to file, `Exports` edge if exported → store in `symbol_nodes` and `symbols_by_name`
3. Add `NodeWeight::Parameter` per function parameter → link to function's NodeIndex
4. Resolve imports via `languages::resolve_import()` → `Imports` edge between file nodes
5. Resolve calls via `symbols_by_name` name-based lookup → `Calls` edge from containing symbol to target symbol(s)

**Compat methods on CodeGraph** (for analyzer migration in Phase 2):
- `file_dependency_edges()` → `HashMap<GraphNode, HashSet<GraphNode>>` (same shape as old `ProjectIndex.edges`)
- `reverse_file_edges()` → reverse of above
- `file_entries()` → `HashMap<String, FileEntry>` equivalent

### Verify
Unit tests: build CodeGraph from temp workspace, assert node/edge counts. Compare compat methods output against existing `build_index()` on same data.

---

## Phase 2: Replace ProjectIndex in analyzers and engine

**Goal**: Wire CodeGraph into all existing cross-file analysis. Delete old index types.

### Delete
- `src/audit/project_index.rs`
- `src/audit/index_builder.rs`

### Modify

**`src/audit/project_analyzer.rs`** — change signature:
```rust
fn analyze(&self, graph: &CodeGraph) -> Vec<AuditFinding>;
```

**`src/audit/engine.rs`** — `run()` takes `Option<&CodeGraph>` instead of `Option<&ProjectIndex>`. Pass to project analyzers. Engine invocation loop unchanged.

**`src/audit/analyzers/circular_deps.rs`** — Use `petgraph::algo::tarjan_scc()` on subgraph of `Imports` edges. Delete hand-rolled Tarjan (~80 lines).

**`src/audit/analyzers/dependency_depth.rs`** — BFS on `Imports` edges via `graph.graph.neighbors_directed()`.

**`src/audit/analyzers/coupling.rs`** — Count `Imports` edges per file node using `graph.graph.edges_directed(node, Outgoing/Incoming)`.

**`src/audit/analyzers/dead_exports.rs`** — Find `Symbol` nodes where `exported=true`, check for any incoming `Calls` or `Imports` edges. More accurate than current name-matching approach (can trace transitive usage).

**`src/audit/analyzers/duplicate_symbols.rs`** — Use `graph.symbols_by_name` to find multi-definition names.

**`src/audit/mod.rs`** — Remove `pub mod index_builder;` and `pub mod project_index;`

**`src/main.rs`** — Replace all `audit::index_builder::build_index()` calls (4 sites) with `graph::builder::GraphBuilder::new(&workspace, &languages).build()?`

**`src/server.rs`** — `AppState.code_graph: CodeGraph` replaces `project_index: ProjectIndex`. Build once at startup.

**`src/languages/mod.rs`** — Move `GraphNode` enum into `src/graph/mod.rs`. Update `resolve_import()` return type import path.

### Verify
`cargo test` — all existing analyzer tests pass. Audit CLI produces same results. Server starts and handles requests.

---

## Phase 3: Replace call_graph.rs in query engine

**Goal**: Delete `call_graph.rs`. Query engine uses CodeGraph for `--calls` traversal.

### Delete
- `src/call_graph.rs`

### Modify

**`src/graph/mod.rs`** — Add methods:
```rust
impl CodeGraph {
    pub fn traverse_callees(&self, seeds: &[NodeIndex], max_depth: usize) -> Vec<NodeIndex>
    pub fn traverse_callers(&self, seeds: &[NodeIndex], max_depth: usize) -> Vec<NodeIndex>
    pub fn find_symbol(&self, file_path: &str, start_line: u32) -> Option<NodeIndex>
    pub fn find_symbols_by_name(&self, name: &str) -> &[NodeIndex]
}
```
BFS implementation using `graph.graph.neighbors_directed()`.

**`src/query_engine.rs`** — Change `execute()` to accept `graph: &CodeGraph`. Replace `call_graph::traverse_call_graph()` with `graph.traverse_callees()`/`traverse_callers()`. Convert `NodeIndex` results back to `QueryResult` structs.

**`src/main.rs`** — In `ProjectCommand::Query`, build `CodeGraph` and pass to `execute()`.

**`src/server.rs`** — Pass `&state.code_graph` to `execute()`.

**`src/lib.rs`** — Remove `pub mod call_graph;`

### Verify
`cargo test` — query tests pass. Manual: `--calls down` produces same results.

---

## Phase 4: PipelineContext — graph-aware pipeline trait

**Goal**: Make the Pipeline trait accept CodeGraph without modifying 304 pipeline files.

### Modify

**`src/audit/pipeline.rs`** — Add:
```rust
pub struct PipelineContext<'a> {
    pub tree: &'a Tree,
    pub source: &'a [u8],
    pub file_path: &'a str,
    pub id_counts: &'a HashMap<String, usize>,
    pub graph: Option<&'a CodeGraph>,
}
```
Add to Pipeline trait:
```rust
fn check_with_context(&self, ctx: &PipelineContext) -> Vec<AuditFinding> {
    self.check_with_ids(ctx.tree, ctx.source, ctx.file_path, ctx.id_counts)
}
```
Default delegates to existing `check_with_ids` → `check`. Zero pipeline changes needed.

**`src/audit/engine.rs`** — In rayon loop, construct `PipelineContext` and call `check_with_context()` instead of `check_with_ids()`. Pass `graph` from `run()` parameter. Change `run()` to always receive `Option<&CodeGraph>` and pass to all pipeline categories (not just Architecture/CodeStyle).

### Verify
`cargo build && cargo test` — all 304 pipelines compile unchanged via default delegation. Engine passes graph through.

---

## Phase 5: Per-language CFG builders

**Goal**: Build intra-procedural CFGs for every function, stored in `CodeGraph::function_cfgs`.

### New files (9 CFG builders)
- `src/graph/cfg_languages/typescript.rs` — JS/TS/TSX/JSX (if/else, for/while, switch, try/catch, `.then()` simplified as direct call, `await` as sync)
- `src/graph/cfg_languages/python.rs` — if/elif/else, for/while, try/except/finally, with, comprehensions as loops
- `src/graph/cfg_languages/rust_lang.rs` — if/else, match arms with guards, loop/while/for, `?` operator as error branch
- `src/graph/cfg_languages/go.rs` — if/else, for, switch/select, defer as cleanup edges, goroutine spawn
- `src/graph/cfg_languages/java.rs` — if/else, for/while, switch, try-catch-finally, try-with-resources auto-close
- `src/graph/cfg_languages/c_lang.rs` — if/else, for/while/do, switch with fallthrough, goto (best-effort)
- `src/graph/cfg_languages/cpp.rs` — extends C with RAII destructors, exceptions, range-for
- `src/graph/cfg_languages/csharp.rs` — if/else, for/foreach, switch, try/catch/finally, using (dispose), pattern matching
- `src/graph/cfg_languages/php.rs` — if/else, for/foreach/while, switch, try/catch/finally

### Each builder
1. Find function body node in tree-sitter AST
2. Walk AST top-down creating BasicBlocks for sequential statements
3. Create CfgStatementKind entries: Assignment (track target + source_vars), Call (name + args), Return, Guard
4. Branch edges: if→TrueBranch/FalseBranch, loop→back edge, switch→multi-branch, try→Exception
5. Language-specific: Go defer→Cleanup edges, Rust ?→Exception, Python with→acquire/release

### Integration into `src/graph/builder.rs`
After step 4 (calls), add step 5: iterate Symbol nodes of kind Function/Method/ArrowFunction, get `cfg_builder_for_language()`, call `build_cfg()` on function's tree-sitter node, store result in `graph.function_cfgs[node_index]`. Parallelized per-file (parse once, build all function CFGs).

### Verify
Unit tests per language: parse known function → build CFG → assert block count, edge types, statement extraction.

---

## Phase 6: Taint analysis engine

**Goal**: Use CFGs to compute FlowsTo/SanitizedBy edges. Detect unsanitized data paths from sources to sinks.

### `src/graph/taint.rs` — full implementation

**Hardcoded tables** per language (const arrays):
- **Sources**: `request.body`, `request.args`, `argv`, `stdin`, `os.environ`, `env::var`, `$_GET`, `$_POST`, etc.
- **Sinks**: `execute()`, `query()`, `eval()`, `innerHTML`, `system()`, `exec.Command()`, `subprocess.call()`, etc.
- **Sanitizers**: `escape()`, `sanitize()`, `parseInt()`, `quote()`, `filepath.Clean()`, `html.EscapeString()`, etc.

**TaintEngine::analyze_function()**:
1. Mark parameters as taint sources (function parameters in HTTP handlers, etc.)
2. Forward propagation through CFG basic blocks: if `CfgStatementKind::Assignment { target, source_vars }` and any source_var is tainted → target becomes tainted
3. At branch points (TrueBranch/FalseBranch edges): split taint state, merge at phi nodes with union semantics (MAYBE tainted = tainted)
4. On `CfgStatementKind::Call`: if call is sanitizer → add `SanitizedBy` edge, remove taint. If call is sink and arg is tainted → add `FlowsTo` edge, record finding.
5. Cross-function: if callee returns tainted value, propagate to caller's return_var

**TaintEngine::analyze_all()**: iterate all functions with CFGs, run analyze_function, add resulting edges to graph.

### `src/graph/resource.rs` — full implementation

**ResourceAnalyzer::analyze_all()**:
- Walk each function's CFG for `ResourceAcquire`/`ResourceRelease` statements
- Track resource variables through CFG paths
- If any path from acquire to function exit lacks release → add `Acquires` edge without matching `ReleasedBy`
- Per-language resource patterns: C (malloc/free, fopen/fclose), Python (open without with), Go (goroutine without context cancel), Java (new Stream without close)

### Integration into `src/graph/builder.rs`
After step 5 (CFGs), add steps 6-7:
```rust
taint::TaintEngine::analyze_all(&mut graph);
resource::ResourceAnalyzer::analyze_all(&mut graph);
```

### Verify
Integration tests: multi-file workspace with known SQL injection (Python function reads request.args, passes to cursor.execute without sanitization). Assert FlowsTo edge exists, no SanitizedBy edge on path. Test sanitized version too — assert SanitizedBy edge breaks taint.

---

## Phase 7: Upgrade security pipelines to use graph

**Goal**: Incrementally migrate existing security pipelines to override `check_with_context()` and query the CodeGraph for taint paths.

### Strategy per pipeline
1. Override `check_with_context()` — query graph for unsanitized FlowsTo paths to relevant sinks for this file
2. Keep `check()` as fallback for when graph is `None`
3. When graph is available: findings from graph-based analysis (higher confidence) replace tree-sitter pattern matching (lower confidence)

### Priority order (highest impact)
1. SQL injection (6 languages: JS/TS, Python, Go, Java, PHP, C#)
2. Command injection (6 languages: JS/TS, Python, Go, Java, PHP, C)
3. XSS DOM injection (JS/TS)
4. Path traversal (6 languages)
5. SSRF (4 languages)
6. Code injection (JS/TS, Python)
7. Resource leak pipelines (C malloc, Python open, Go goroutine, Java stream)
8. Race condition pipelines (Rust, Go, C++, Java)

### New graph-powered pipelines
- **Cross-file dead code** — functions with no incoming Calls edges
- **Complexity aggregation** — sum cyclomatic complexity along call chains, flag deceptively simple entry points
- **Guard coverage** — paths from taint source to sink that bypass all sanitizers

### Verify
Before/after comparison on test workspaces. False positive rate should decrease for each migrated pipeline.

---

## Phase 8: Cleanup and CLAUDE.md update

- Remove any remaining references to `ProjectIndex`, `IndexBuilder`, `call_graph`
- Update CLAUDE.md: document `src/graph/` module structure, CodeGraph architecture, CFG design, taint analysis
- Add `petgraph` to architecture section of CLAUDE.md

---

## Files Summary

### New files (15)
- `src/graph/mod.rs`
- `src/graph/builder.rs`
- `src/graph/cfg.rs`
- `src/graph/taint.rs`
- `src/graph/resource.rs`
- `src/graph/cfg_languages/mod.rs`
- `src/graph/cfg_languages/typescript.rs`
- `src/graph/cfg_languages/python.rs`
- `src/graph/cfg_languages/rust_lang.rs`
- `src/graph/cfg_languages/go.rs`
- `src/graph/cfg_languages/java.rs`
- `src/graph/cfg_languages/c_lang.rs`
- `src/graph/cfg_languages/cpp.rs`
- `src/graph/cfg_languages/csharp.rs`
- `src/graph/cfg_languages/php.rs`

### Deleted files (3)
- `src/audit/project_index.rs`
- `src/audit/index_builder.rs`
- `src/call_graph.rs`

### Modified files (key)
- `Cargo.toml` — petgraph dep
- `src/lib.rs` — add graph module, remove call_graph
- `src/audit/mod.rs` — remove deleted module refs
- `src/audit/pipeline.rs` — PipelineContext + check_with_context()
- `src/audit/engine.rs` — use CodeGraph, call check_with_context()
- `src/audit/project_analyzer.rs` — &CodeGraph instead of &ProjectIndex
- `src/audit/analyzers/*.rs` — all 5 analyzers rewritten for CodeGraph
- `src/query_engine.rs` — accept &CodeGraph, use for call traversal
- `src/server.rs` — AppState.code_graph
- `src/main.rs` — build CodeGraph at all audit/query call sites
- `src/languages/mod.rs` — GraphNode import path update
- Security pipelines (incremental, Phase 7) — override check_with_context()

### Untouched
- All 304 existing pipeline `check()` implementations (default delegation via check_with_context)
- `src/models.rs`, `src/format.rs`, `src/signature.rs`, `src/discovery.rs`, `src/workspace.rs`, `src/file_source.rs`, `src/parser.rs`, `src/s3.rs`, `src/registry.rs`, `src/cli.rs`, `src/query_lang.rs`

## Phase Dependencies

```
Phase 0 (types)
    │
Phase 1 (builder steps 1-4)
    │
    ├── Phase 2 (replace ProjectIndex in analyzers)
    │       │
    │   Phase 3 (replace call_graph.rs in query_engine)
    │
Phase 4 (PipelineContext) ── can run in parallel with Phase 2/3
    │
Phase 5 (CFG builders) ── requires Phase 1
    │
Phase 6 (taint + resource) ── requires Phase 5
    │
Phase 7 (new pipelines) ── requires Phase 4 + Phase 6
    │
Phase 8 (migrate existing) ── requires Phase 4 + Phase 6
```

## Risks and Mitigations

1. **304 pipeline files**: The `PipelineContext` wrapper approach (Phase 4) means zero changes to existing pipelines. The `check()` method signature is preserved. Only pipelines that want graph access override `check_with_context`.

2. **petgraph Send/Sync**: `petgraph::DiGraph` is `Send` but not `Sync`. After building the graph, wrap it in `Arc<CodeGraph>` for sharing across rayon threads (read-only access is safe).

3. **Graph build time**: CFG step (Phase 5) is the most expensive. Mitigation: CFG building is per-function and parallelizable via the same rayon threadpool pattern.

4. **Memory usage**: ~100 bytes per node + ~32 bytes per edge. For a 10K file project with 50K symbols and 200K call sites: ~25MB for nodes + ~6MB for edges. Well within budget given workspace already holds all files in memory.

5. **CFG correctness**: Start with simplified model: basic blocks at statement level, branch edges at if/match/switch. Skip complex patterns (goto, coroutines) in v1. Each language CFG builder has independent unit tests.

6. **Taint false positives**: Name-based resolution means `execute()` in one file might match an unrelated `execute()` in another. Mitigation: scope taint analysis to within-function CFGs first (intra-procedural), cross-function only via direct callee resolution.

## Verification

After each phase: `cargo build && cargo test`. After Phase 2: run `cargo run -- audit src/ --language rs` and compare output to pre-change baseline. After Phase 3: run `cargo run -- projects query myapp --q '{"find": "function", "calls": "down"}'` and compare. After Phase 7: run security audit on known-vulnerable test fixtures and measure false positive reduction.
