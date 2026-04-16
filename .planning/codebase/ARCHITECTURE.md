# Architecture

**Analysis Date:** 2026-04-16

## Pattern Overview

**Overall:** On-demand parsing with composable pipeline architecture

**Key Characteristics:**
- No pre-indexing or database — files are parsed at query time
- Parallel processing via rayon for I/O-bound discovery and CPU-bound parsing
- Multi-layered command pipeline: CLI → Registry/Workspace → Query/Audit Engine → Language-specific parsers → Output formatter
- In-memory file loading (workspace layer) decouples from filesystem; enables S3 support
- Pluggable `FileSource` trait for different storage backends
- CodeGraph abstraction for cross-file analysis (call graphs, taint propagation, resource lifecycles)
- Per-language tree-sitter queries compiled once and shared via Arc across rayon threads
- Audit pipelines as trait objects allowing heterogeneous pipeline implementations

## Layers

**CLI Layer:**
- Purpose: Command dispatch and argument parsing
- Location: `src/cli.rs`, `src/main.rs`
- Contains: Clap command/subcommand definitions for `projects`, `audit`, `serve` subcommands; argument validation
- Depends on: Registry, Workspace, QueryEngine, AuditEngine, Server
- Used by: Users via command line

**Registry Layer:**
- Purpose: Project CRUD operations; persistent project metadata storage
- Location: `src/registry.rs`
- Contains: `ProjectEntry` struct (name, path, language filter, file count, breakdown); atomic file write via `.tmp` + rename pattern
- Depends on: Language module for language parsing
- Used by: CLI projects subcommand, Workspace initialization

**Workspace Layer:**
- Purpose: File discovery and in-memory loading; abstracts away filesystem/S3 differences
- Location: `src/workspace.rs`
- Contains: `Workspace` struct holding `Arc<str>` file contents, language map; file discovery via ignore crate (respects .gitignore)
- Depends on: FileSource trait, Discovery module, Language module, S3 module (optional, for S3 workspaces)
- Used by: Query engine, Audit engine, Server, Graph builder

**FileSource Layer (Abstraction):**
- Purpose: Pluggable file access backend
- Location: `src/file_source.rs`
- Contains: `FileSource` trait (read_file, list_files); `MemoryFileSource` implementation backing both disk and S3 workspaces
- Depends on: None
- Used by: Workspace

**Query Language & Parsing:**
- Purpose: Parse JSON query DSL and compile tree-sitter queries
- Location: `src/query_lang.rs`, `src/parser.rs`, `src/languages/mod.rs`
- Contains: `TsQuery` struct (filters: files, find, name, visibility, inside, has, lines, calls, depth, read, graph); filter enums (FindFilter, NameFilter, HasFilter); per-language tree-sitter query compilation and symbol/import/comment extraction
- Depends on: Language module, Models, tree-sitter crate
- Used by: Query engine, Format layer

**Query Engine:**
- Purpose: Execute parsed queries against workspace; symbol filtering and result construction
- Location: `src/query_engine.rs`
- Contains: `execute()` function with rayon-parallel per-file parsing and filtering; `QueryOutput` and `QueryResult` structs; support for call graph traversal (via CodeGraph)
- Depends on: Workspace, Language module, Query language, Models, CodeGraph (optional, for call graph queries)
- Used by: CLI, Server

**Parser Layer:**
- Purpose: Tree-sitter setup and file metadata extraction
- Location: `src/parser.rs`
- Contains: `create_parser()` for per-thread parser instantiation; `parse_file()` for disk files, `parse_content()` for in-memory; metadata collection (size, lines, name, extension)
- Depends on: Language module, tree-sitter crate
- Used by: Query engine, Audit engine, Graph builder

**Language Modules:**
- Purpose: Language-specific tree-sitter S-expression queries and symbol/import/comment extraction
- Location: `src/languages/` (one file per language family: typescript.rs, rust_lang.rs, c_lang.rs, cpp.rs, csharp.rs, python.rs, go.rs, java.rs, php.rs)
- Contains: Per-language query compilation (symbol_query, import_query, comment_query); extract_symbols(), extract_imports(), extract_comments() implementations
- Depends on: Models, tree-sitter crate, tree-sitter-language crates
- Used by: Query engine, Audit engine, Graph builder

**Graph Layer:**
- Purpose: Cross-file analysis via multi-stage graph construction and traversal
- Location: `src/graph/` (mod.rs, builder.rs, cfg.rs, cfg_languages/, pipeline.rs, taint.rs, resource.rs, executor.rs)
- Contains:
  - `CodeGraph` (petgraph DiGraph with Node/EdgeWeight enums; file_nodes, symbol_nodes, symbols_by_name lookups; function_cfgs map)
  - `GraphBuilder` (parallel extraction of symbols/imports/call-sites; CFG building per function; assembly into DiGraph)
  - `FunctionCfg` (control flow graph per function; BasicBlock, CfgStatement, CfgEdge types)
  - Per-language CFG builders in `cfg_languages/` (typescript.rs, rust_lang.rs, python.rs, go.rs, java.rs, c_lang.rs, cpp.rs, csharp.rs, php.rs)
  - `TaintEngine` (taint propagation via source/sink/sanitizer tables; computes FlowsTo/SanitizedBy edges)
  - `ResourceAnalyzer` (resource lifecycle tracking; computes Acquires/ReleasedBy edges)
  - Graph pipelines (stateful execution on graph with Flag stage for findings)
- Depends on: Workspace, Language module, Models, petgraph crate
- Used by: Audit engine, Query engine (for call graph), Server

**Audit Engine & Pipelines:**
- Purpose: Static analysis across multiple code quality dimensions
- Location: `src/audit/engine.rs`, `src/audit/pipeline.rs`, `src/audit/pipelines/`, `src/audit/json_audit.rs`
- Contains:
  - `AuditEngine` with pipeline selector (TechDebt, Complexity, CodeStyle, Security, Scalability, Architecture)
  - `Pipeline` trait (check and check_with_context methods for per-file analysis)
  - Trait object `AnyPipeline` enabling heterogeneous pipeline storage
  - Per-language per-category pipeline implementations (e.g., `src/audit/pipelines/typescript/`, `src/audit/pipelines/rust/`)
  - JSON audit loading from project-local, user-global, or built-in JSON files
  - `AuditFinding` struct with pattern matching, severity, file/line, message, remediation
- Depends on: Workspace, Language module, Models, Graph, Parser
- Used by: CLI, Server

**Format Layer:**
- Purpose: Query result formatting and output wrapping
- Location: `src/format.rs`
- Contains: `format_results()` supporting outline/snippet/full/tree/locations/summary output formats; JSON wrapping with metadata (project, query_ms, files_parsed, total)
- Depends on: Query engine models, CLI output format enum
- Used by: CLI, Server

**S3 Support:**
- Purpose: Query/audit remote codebases via S3-compatible storage
- Location: `src/s3.rs`
- Contains: `S3Location` struct (bucket, prefix); list_objects() with concurrent filtered downloads; credential chain (S3_*, AWS_*, standard AWS SDK chain)
- Depends on: Language module, aws-sdk-s3 crate
- Used by: Workspace (load_from_s3), CLI S3 flag handlers

**Server (HTTP API):**
- Purpose: Persistent HTTP server for remote/programmatic query and audit access
- Location: `src/server.rs`
- Contains: Axum-based HTTP router with `/health`, `/query` (POST), `/audit/summary` (POST), `/audit/{category}` (POST) endpoints; tokio blocking task dispatch for CPU-bound query/audit work; 120-second request timeout
- Depends on: Workspace, Query engine, Audit engine, Graph builder, Language module, axum, tokio
- Used by: CLI serve subcommand

**Model Layer:**
- Purpose: Shared data structures across all layers
- Location: `src/models.rs`
- Contains: `FileMetadata` (path, name, extension, language, size_bytes, line_count); `SymbolInfo`, `ImportInfo`, `CommentInfo`; `SymbolKind` enum (17 kinds); `ParseError`
- Depends on: None
- Used by: All other layers

## Data Flow

**Query Execution:**

1. User invokes `virgil projects query myapp --q '{...}'`
2. CLI parses command, loads query JSON via `TsQuery` deserialization
3. Registry looks up project entry (path, language filter)
4. Workspace is initialized: discovers files via ignore crate, loads into memory in parallel (rayon), returns ready-to-use Workspace
5. Query engine receives Workspace + TsQuery:
   - Pre-compiles tree-sitter queries per language (Arc-shared)
   - Groups files by language, applies file glob filters
   - Parallel per-file loop (rayon): parse → extract symbols → apply filters (kind, name, visibility, inside, has, lines) → collect QueryResult
   - If call graph requested: loads CodeGraph, performs BFS traversal (up/down/both) on callees/callers
   - Returns QueryOutput with results, files_parsed, total count
6. Format layer wraps results in JSON with metadata
7. Output to stdout (or via Server if HTTP)

**Audit Execution:**

1. User invokes `virgil audit <dir> --language ts`
2. CLI parses command, resolves pipeline selector from subcommand (e.g., code-quality → CodeStyle)
3. Workspace initialized (same as query flow)
4. Graph builder constructs CodeGraph in parallel:
   - Per-file extraction: symbols, imports, call-sites
   - CFG building per function (language-specific)
   - Graph assembly: nodes (file, symbol, call-site, parameter, external source), edges (DefinedIn, Calls, Imports, FlowsTo, SanitizedBy, Acquires, ReleasedBy)
   - Taint engine propagates sources through FlowsTo paths
   - Resource analyzer tracks acquire/release pairs
5. Audit engine selects pipelines for each language (filtered by selector + optional per-pipeline filter)
6. JSON audit discovery: loads user/global/built-in JSON files (overrides built-in Rust pipelines by name)
7. Parallel per-file execution (rayon): for each file, iterate selected pipelines, call `check_with_context(PipelineContext)`, collect AuditFinding
8. Findings aggregated, sorted by severity/file, formatted (table/json/csv)

**S3 Query Flow:**

1. User invokes `virgil projects query --s3 s3://bucket/prefix --q '{...}' --lang ts`
2. CLI parses S3 URI, creates no registry entry (S3 is on-demand)
3. Workspace::load_from_s3() called:
   - S3Location parsed (bucket, prefix)
   - list_objects() filters by language extensions, respects exclude patterns
   - download_objects() concurrently fetches all files into MemoryFileSource (bounded semaphore)
   - Language map built from extensions
   - Returns synthetic S3 workspace
4. Query engine executes identically to disk-based flow (FileSource abstraction hides backend)

**State Management:**

- Query execution is stateless (query_engine::execute is pure given workspace + query)
- Graph is immutable once built (petgraph DiGraph), Arc-shared for read-only access across rayon threads
- Workspace keeps file contents in Arc<str> (zero-copy sharing)
- Per-thread Parser instances created fresh in rayon tasks (Parser is !Send, must be per-task)
- Audit findings are accumulated into Vec during parallel iteration, then sorted/formatted centrally

## Key Abstractions

**FileSource Trait:**
- Purpose: Abstract file read/list operations; enables disk and S3 backends
- Examples: `src/file_source.rs`, `src/workspace.rs`
- Pattern: Trait object stored in Workspace; no dynamic dispatch in hot loop (file reads are outside rayon loop)

**Pipeline Trait:**
- Purpose: Pluggable analysis rules; enable Rust + JSON audit mixing
- Examples: `src/audit/pipeline.rs` (trait definition), `src/audit/pipelines/` (implementations)
- Pattern: Trait object `AnyPipeline` in pipeline map; heterogeneous Vec; per-file check() call in rayon loop

**CodeGraph:**
- Purpose: Multi-use cross-file analysis backbone
- Examples: `src/graph/mod.rs`, `src/graph/builder.rs`
- Pattern: petgraph DiGraph + HashMap indexes (file_nodes, symbol_nodes, symbols_by_name); built once, Arc-shared for reads; supports call graph BFS, taint analysis, resource tracking

**TsQuery (JSON Query DSL):**
- Purpose: User-friendly composable query language
- Examples: `src/query_lang.rs`
- Pattern: Serde-deserialized untagged enums (FindFilter, NameFilter, HasFilter) for flexible syntax; compile to tree-sitter queries during execution

**SymbolKind Enum:**
- Purpose: Language-agnostic symbol classification
- Examples: `src/models.rs`
- Pattern: 17 kinds (function, class, method, variable, interface, type_alias, enum, arrow_function, struct, union, namespace, macro, property, typedef, trait, constant, module); used by all language modules for consistent output

## Entry Points

**main.rs:**
- Location: `src/main.rs`
- Triggers: `cargo run -- ...` or `virgil ...` (when installed)
- Responsibilities: CLI parsing via clap, dispatch to registry/query/audit/serve subcommands, error handling, eprintln for diagnostics

**lib.rs:**
- Location: `src/lib.rs`
- Triggers: `use virgil_cli::...` in external code
- Responsibilities: Public API re-exports; enables library use (e.g., by server, by other tools)

**Workspace::load():**
- Location: `src/workspace.rs`
- Triggers: All query/audit flows; optionally followed by Workspace::load_from_s3()
- Responsibilities: File discovery, parallel loading into memory, language mapping

**query_engine::execute():**
- Location: `src/query_engine.rs`
- Triggers: Query subcommand, Server /query endpoint
- Responsibilities: Compile queries, parallel per-file parse+filter, result collection, optional call graph traversal

**AuditEngine::run():**
- Location: `src/audit/engine.rs`
- Triggers: Audit subcommand, Server /audit/* endpoints
- Responsibilities: Pipeline selection, Graph building (optional), parallel per-file pipeline execution, finding aggregation

**Server::run_server():**
- Location: `src/server.rs`
- Triggers: Serve subcommand
- Responsibilities: Load workspace from S3, build CodeGraph, start axum HTTP server, dispatch requests to query/audit engines via tokio blocking

## Error Handling

**Strategy:** Layered error propagation with anyhow Result<T> throughout; graceful degradation for non-critical issues

**Patterns:**

- Parse errors: Per-file warnings to stderr, file skipped, processing continues (reported in audit summary)
- Registry I/O errors: Propagate anyhow::anyhow (user-facing); suggest creation if project not found
- Query deserialization errors: anyhow::anyhow with context (invalid JSON); user-friendly message
- Tree-sitter failures: Logged and skipped; continue with other files
- Graph construction failures: Audit engine provides fallback empty graph (graphs optional for some audits)
- S3 connection errors: Propagate from aws-sdk-s3 wrapped in anyhow
- Server timeouts: HTTP 504 after 120 seconds; query/audit interrupted by tokio::time::timeout

## Cross-Cutting Concerns

**Logging:** stderr via eprintln!() for diagnostic output (progress, warnings, file counts); configurable via caller (e.g., ProgressBar in audit engine)

**Validation:**
- File globs validated via globset crate (compile-time via Glob::new)
- Query JSON schema enforced via Serde (deserialization errors trap invalid fields)
- Language enum ensures only supported languages are used (from_extension returns Option)
- Symbol kind matched against SymbolKind enum (case-insensitive, unknown kinds rejected)

**Authentication:** S3 credentials via environment variables (S3_ACCESS_KEY_ID, S3_SECRET_ACCESS_KEY, S3_ENDPOINT); falls back to AWS SDK credential chain

**Parallelism:**
- rayon ThreadPool with configurable stack size (4MB in graph builder) to avoid stack overflow on deeply nested ASTs
- Per-thread Parser (tree-sitter::Parser is !Send)
- Arc-shared Query objects, Workspace FileSource, CodeGraph for read-only cross-thread access
- No locks in hot loop (grouping by language before par_iter avoids contention)

**Performance Optimizations:**
- Pre-compile tree-sitter queries per language once, Arc-share
- File discovery scoped by language (Language::all_extensions) to reduce parse attempts
- In-memory workspace trades RAM for I/O speed (no repeated disk reads)
- Parallel file loading (rayon) with max_file_size filter to skip large/binary files
- S3 concurrent downloads with 64-semaphore bounded parallelism
- Graph pipelines use stack-based iteration in helpers (avoid recursion depth limits)

---

*Architecture analysis: 2026-04-16*
