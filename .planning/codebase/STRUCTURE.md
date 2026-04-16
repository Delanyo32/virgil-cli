# Codebase Structure

**Analysis Date:** 2026-04-16

## Directory Layout

```
virgil-cli/
├── src/
│   ├── main.rs              # CLI entry point, command dispatch
│   ├── lib.rs               # Public library re-exports
│   ├── cli.rs               # Clap command definitions (projects, audit, serve)
│   ├── registry.rs          # Project CRUD (~/.virgil-cli/projects.json)
│   ├── workspace.rs         # File discovery + in-memory loading
│   ├── file_source.rs       # FileSource trait + MemoryFileSource
│   ├── query_lang.rs        # JSON query DSL (TsQuery, filter enums)
│   ├── query_engine.rs      # Query execution (parse, filter, results)
│   ├── parser.rs            # tree-sitter parser setup + metadata
│   ├── language.rs          # Language enum + tree-sitter language mapping
│   ├── model.rs             # Shared data structures (SymbolInfo, ImportInfo, etc.)
│   ├── signature.rs         # One-line signature extraction
│   ├── discovery.rs         # File walking with .gitignore support
│   ├── format.rs            # Query output formatting (outline/snippet/full/tree/summary)
│   ├── s3.rs                # S3/R2 client + URI parsing + concurrent download
│   ├── server.rs            # Axum HTTP server (serve command)
│   │
│   ├── languages/           # Language-specific tree-sitter queries + extraction
│   │   ├── mod.rs           # Language dispatch (compile_symbol_query, extract_symbols, etc.)
│   │   ├── typescript.rs     # TS/JS/TSX/JSX queries
│   │   ├── rust_lang.rs      # Rust queries
│   │   ├── c_lang.rs         # C queries
│   │   ├── cpp.rs            # C++ queries
│   │   ├── csharp.rs         # C# queries
│   │   ├── python.rs         # Python queries
│   │   ├── go.rs             # Go queries
│   │   ├── java.rs           # Java queries
│   │   └── php.rs            # PHP queries
│   │
│   ├── graph/               # Cross-file analysis (CodeGraph, CFG, taint, resource)
│   │   ├── mod.rs           # CodeGraph struct + node/edge types + traversal
│   │   ├── builder.rs        # GraphBuilder (parallel extraction + assembly)
│   │   ├── cfg.rs            # Control flow graph types (FunctionCfg, BasicBlock, CfgStatement)
│   │   ├── cfg_languages/    # Language-specific CFG builders
│   │   │   ├── mod.rs        # CfgBuilder trait + dispatch
│   │   │   ├── typescript.rs # TS/JS CFG builder
│   │   │   ├── rust_lang.rs  # Rust CFG builder
│   │   │   ├── python.rs     # Python CFG builder
│   │   │   ├── go.rs         # Go CFG builder
│   │   │   ├── java.rs       # Java CFG builder
│   │   │   ├── c_lang.rs     # C CFG builder
│   │   │   ├── cpp.rs        # C++ CFG builder
│   │   │   ├── csharp.rs     # C# CFG builder
│   │   │   └── php.rs        # PHP CFG builder
│   │   ├── taint.rs          # Taint propagation engine + source/sink tables
│   │   ├── resource.rs       # Resource lifecycle analyzer
│   │   ├── pipeline.rs       # Graph pipeline stages + PipelineContext
│   │   └── executor.rs       # Graph pipeline executor
│   │
│   └── audit/               # Static analysis engine + pipelines
│       ├── mod.rs           # Module exports
│       ├── engine.rs        # AuditEngine (pipeline selection + parallel execution)
│       ├── pipeline.rs      # Pipeline trait definition + context wrappers
│       ├── models.rs        # AuditFinding, AuditSummary structs
│       ├── format.rs        # Finding formatting (table/json/csv)
│       ├── json_audit.rs    # JSON audit discovery + loading
│       ├── project_index.rs # ProjectIndex + ExportedSymbol for audit helpers
│       ├── project_analyzer.rs # Cross-file helpers (coupling, export analysis)
│       ├── analyzers/       # Helper analyzers (not pipelines)
│       │   ├── mod.rs       # Analyzer exports
│       │   ├── coupling.rs  # Module coupling detection
│       │   ├── dead_exports.rs # Unused export detection
│       │   └── duplicate_symbols.rs # Duplicate symbol detection
│       ├── primitives/      # Low-level analysis helpers
│       │   └── (various)    # Pattern-matching primitives, CFG helpers
│       ├── builtin/         # Built-in JSON audit files (checked into repo)
│       │   └── (JSON files)
│       └── pipelines/       # Per-language pipeline implementations
│           ├── mod.rs       # Pipeline registry + selector logic
│           ├── helpers.rs   # Shared audit helpers
│           ├── typescript/  # TS/JS pipelines
│           ├── rust/        # Rust pipelines
│           ├── python/      # Python pipelines
│           ├── go/          # Go pipelines
│           ├── java/        # Java pipelines
│           ├── javascript/  # JS-specific pipelines
│           ├── c/           # C pipelines
│           ├── cpp/         # C++ pipelines
│           ├── csharp/      # C# pipelines
│           └── php/         # PHP pipelines
│
├── tests/
│   └── integration_test.rs  # Integration tests
│
├── Cargo.toml              # Rust manifest (dependencies, bin target)
└── Cargo.lock              # Lockfile
```

## Directory Purposes

**`src/`:**
- Purpose: All Rust source code
- Contains: Main entry, CLI, core modules, language-specific logic, analysis engine
- Key files: `main.rs` (entry), `lib.rs` (public API), `cli.rs` (command definitions)

**`src/languages/`:**
- Purpose: Language-specific tree-sitter query compilation and symbol extraction
- Contains: One file per language family; each exports `compile_symbol_query()`, `compile_import_query()`, `compile_comment_query()`, and extract functions
- Key files: All nine files (typescript.rs, rust_lang.rs, c_lang.rs, etc.) — collectively handle 13 file extensions across 12 languages

**`src/graph/`:**
- Purpose: Cross-file analysis via directed graph representation and traversal
- Contains: CodeGraph definition, GraphBuilder, control flow graphs, taint analysis, resource tracking, graph pipeline execution
- Key files: `mod.rs` (CodeGraph), `builder.rs` (parallel construction), `cfg.rs` (CFG types), `taint.rs` (data flow), `pipeline.rs` (graph query stages)

**`src/graph/cfg_languages/`:**
- Purpose: Language-specific control flow graph construction
- Contains: Per-language CFG builder implementations; one file per language (9 total)
- Key files: Language-specific CFG builders used by graph builder during parallel phase

**`src/audit/`:**
- Purpose: Static analysis via pluggable pipeline architecture
- Contains: AuditEngine, Pipeline trait, per-language pipeline implementations, analyzers, JSON audit support
- Key files: `engine.rs` (orchestrator), `pipeline.rs` (trait), `pipelines/` (implementations), `json_audit.rs` (user overrides)

**`src/audit/pipelines/`:**
- Purpose: Per-language analysis pipelines (tech debt, complexity, security, scalability, architecture)
- Contains: One subdirectory per language; each contains pipelines grouped by category (e.g., `typescript/cyclomatic.rs`, `rust/panic_detection.rs`)
- Key files: Pipeline implementations for each language × category combination

**`tests/`:**
- Purpose: Integration tests
- Contains: End-to-end test of query/audit flows
- Key files: `integration_test.rs`

## Key File Locations

**Entry Points:**
- `src/main.rs`: CLI dispatch; parses args, routes to subcommands
- `src/lib.rs`: Public API exports for library use

**Configuration & Registry:**
- `src/registry.rs`: Project metadata CRUD; reads/writes `~/.virgil-cli/projects.json` atomically

**Core Query/Audit Flow:**
- `src/workspace.rs`: File discovery and in-memory loading
- `src/query_engine.rs`: Query execution; main hot loop
- `src/audit/engine.rs`: Audit orchestration; pipeline selection and execution

**Language Handling:**
- `src/language.rs`: Language enum, extension mapping, tree-sitter language setup
- `src/languages/mod.rs`: Dispatch to per-language query compilation and extraction
- `src/languages/{typescript,rust_lang,c_lang,...}.rs`: Per-language tree-sitter queries (9 files)

**Analysis Infrastructure:**
- `src/graph/mod.rs`: CodeGraph definition and call graph traversal
- `src/graph/builder.rs`: Parallel graph construction
- `src/audit/pipeline.rs`: Pipeline trait and context wrappers
- `src/audit/pipelines/mod.rs`: Pipeline registry and selector

**Output:**
- `src/format.rs`: Query result formatting (outline/snippet/full/tree/locations/summary)
- `src/audit/format.rs`: Finding formatting (table/json/csv)

**Integration:**
- `src/s3.rs`: S3/R2 file download and URI parsing
- `src/server.rs`: Axum HTTP server and request handlers

## Naming Conventions

**Files:**
- Language modules: `{language}.rs` or `{language}_lang.rs` (underscore suffix for keywords like `rust_lang.rs`, `c_lang.rs`)
- Pipeline files: Named by pattern (e.g., `cyclomatic.rs` for cyclomatic complexity) or analysis type (e.g., `dead_code.rs`)
- Trait implementations: Grouped by language in subdirectories (e.g., `src/graph/cfg_languages/`)
- Helpers: `helpers.rs` or `primitives/` subdirectory

**Directories:**
- Language-specific: Direct language name (typescript, rust, python, go, java) or language name + category (e.g., `audit/pipelines/javascript/`)
- Analysis: By category (graph, audit, languages)
- Abstractions: Trait + implementations grouped together (e.g., cfg_languages/mod.rs + one file per language)

**Functions:**
- Query/extract: `compile_{query_type}()`, `extract_{thing}()` (e.g., `compile_symbol_query`, `extract_symbols`)
- Tree-sitter helpers: Start with underscore (private helpers within language modules)
- Builder pattern: `{Type}::new()`, `.{option}()`, `.build()`

**Enums & Structs:**
- Public types: PascalCase (e.g., `SymbolKind`, `Language`, `CodeGraph`)
- Trait objects: `Any{Trait}` prefix (e.g., `AnyPipeline`)
- Contexts: `{Name}Context` (e.g., `PipelineContext`, `GraphPipelineContext`)

## Where to Add New Code

**New Audit Pipeline:**
1. Create file in `src/audit/pipelines/{language}/` (e.g., `src/audit/pipelines/typescript/new_pattern.rs`)
2. Implement `Pipeline` trait with `name()`, `check()` or `check_with_context()`
3. Register in `src/audit/pipelines/{language}/mod.rs` by adding struct to appropriate category function (e.g., `security_pipelines_for_typescript()`)
4. Update `src/audit/pipelines/mod.rs` to include in `pipelines_for_language()` or selector-specific function

**New Language Support:**
1. Add variant to `Language` enum in `src/language.rs`
2. Add tree-sitter language getter in `Language::tree_sitter_language()` and `as_str()`
3. Create `src/languages/{language}.rs` with `compile_symbol_query()`, `compile_import_query()`, `compile_comment_query()`, and `extract_*()` functions
4. Add dispatch entries in `src/languages/mod.rs` for all compile and extract functions
5. Create `src/graph/cfg_languages/{language}.rs` with `CfgBuilder` implementation
6. Add CFG dispatch in `src/graph/cfg_languages/mod.rs`
7. Create audit pipeline templates in `src/audit/pipelines/{language}/mod.rs`
8. Update extension mapping in `Language::from_extension()`

**New Command/Subcommand:**
1. Add variant to `Command` or `ProjectCommand` enum in `src/cli.rs`
2. Implement handler in `src/main.rs` under appropriate match arm
3. If async, add to `server.rs` route table instead

**New Query Filter:**
1. Add field to `TsQuery` struct in `src/query_lang.rs`
2. Implement filter enum (e.g., `MyFilter`) using `#[serde(untagged)]` pattern
3. Add filter logic in `query_engine.rs` in the per-file filtering loop
4. Add test in `src/query_lang.rs` tests module

**Shared Analysis Helper:**
1. Place in `src/audit/project_analyzer.rs` if cross-file, or `src/audit/primitives/` if primitive
2. If primitive (low-level CFG/taint helper), create `src/audit/primitives/{name}.rs`
3. Export in `src/audit/primitives/mod.rs`
4. Import and use in relevant pipelines

**New Output Format:**
1. Add variant to `QueryOutputFormat` enum in `src/cli.rs`
2. Implement formatting function in `src/format.rs` (follow pattern of `format_outline`, `format_snippet`, etc.)
3. Add case in `build_wrapper()` match statement

**Tests:**
- Unit tests: Inline in module (convention `#[cfg(test)] mod tests { #[test] fn ... }`)
- Integration tests: `tests/integration_test.rs`
- Parser tests: `src/languages/{language}.rs` (test queries directly)
- Pipeline tests: Included in pipeline file or grouped in `src/audit/pipelines/tests.rs`

## Special Directories

**`src/audit/builtin/`:**
- Purpose: Checked-in JSON audit files (user overrides via ~/.virgil-cli/audits/ or project root)
- Generated: No (user-provided or shipped with binary)
- Committed: Yes (audit templates)

**`src/audit/primitives/`:**
- Purpose: Low-level helpers for CFG analysis, identifier counting, coupling detection
- Generated: No
- Committed: Yes

**`~/.virgil-cli/`:**
- Purpose: User home directory; contains projects.json registry and optional audits/ subdir
- Generated: Yes (created on first project creation)
- Committed: No (user-local)

**`target/`:**
- Purpose: Build artifacts (not shown in directory layout above)
- Generated: Yes (by cargo build)
- Committed: No

---

*Structure analysis: 2026-04-16*
