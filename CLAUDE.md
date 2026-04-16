# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases on-demand and queries them with a composable JSON query language. No database, no pre-indexing — projects are registered by name and parsed at query time. Supports S3-compatible storage (AWS S3, Cloudflare R2, MinIO) for querying and auditing remote codebases directly via `--s3 s3://bucket/prefix`.

## Build & Run

```bash
cargo build
cargo run -- projects create myapp --path ./src [--lang ts,tsx,js,jsx] [--exclude "vendor/**"]
cargo run -- projects list
cargo run -- projects delete myapp
cargo run -- projects query myapp --q '{"find": "function", "name": "handle*"}' [--out outline|snippet|full|tree|locations|summary] [--pretty] [--max 100]
cargo run -- projects query myapp --file query.json
# S3/R2 (no registration needed)
cargo run -- projects query --s3 s3://bucket/prefix --q '{"find": "function"}' [--lang rs] [--out summary] [--pretty]
cargo run -- audit --s3 s3://bucket/prefix [--language rs]
# Serve mode (persistent HTTP server)
cargo run -- serve --s3 s3://bucket/prefix [--host 127.0.0.1] [--port 0] [--lang rs]
```

## Subcommands

All commands are nested under `virgil projects`:

| Command | Description |
|---------|-------------|
| `projects create` | Register a project for querying (scans files, saves to `~/.virgil-cli/projects.json`) |
| `projects list` | List registered projects with file counts |
| `projects delete` | Remove a registered project |
| `projects query` | Query a project using inline JSON (`--q`), a file (`--file`), or stdin |
| `projects query --s3` | Query an S3/R2 codebase directly (no registration needed) |

Audit commands are under `virgil audit`:

| Command | Description |
|---------|-------------|
| `audit <DIR>` | Run all audit categories |
| `audit code-quality <DIR>` | All code quality checks (summary) |
| `audit code-quality tech-debt <DIR>` | Tech debt patterns (`--pipeline`: panic_detection) |
| `audit code-quality complexity <DIR>` | Complexity metrics (`--pipeline`: cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio) |
| `audit code-quality code-style <DIR>` | Code style issues (`--pipeline`: dead_code, duplicate_code, coupling) |
| `audit security <DIR>` | Security vulnerabilities (injection, unsafe memory, race conditions) |
| `audit scalability <DIR>` | Scalability issues (`--pipeline`: n_plus_one_queries, sync_blocking_in_async, memory_leak_indicators) |
| `audit architecture <DIR>` | Architecture analysis (`--pipeline`: module_size_distribution, circular_dependencies, dependency_graph_depth, api_surface_area) |

Common audit options: `--s3` (S3 URI, replaces `<DIR>`), `--language` (comma-separated), `--pipeline` (comma-separated), `--format` (table|json|csv), `--per-page` (default 20), `--page` (default 1).

Server command is under `virgil serve`:

| Command | Description |
|---------|-------------|
| `serve --s3 <URI>` | Start persistent HTTP server, load codebase from S3 at startup |

Server options: `--host` (default `127.0.0.1`), `--port` (default `0` for OS-assigned), `--lang` (comma-separated), `--exclude` (repeatable globs).

HTTP API: `GET /health`, `POST /query`, `POST /audit/summary`, `POST /audit/{category}` (architecture, security, scalability, code-quality).

## JSON Query Language

Queries are JSON objects with composable filters:

```json
{
  "files": "src/api/**",
  "files_exclude": ["**/test/**"],
  "find": "function",
  "name": "handle*",
  "visibility": "exported",
  "inside": "AuthService",
  "has": "@deprecated",
  "lines": {"min": 10, "max": 100},
  "body": true,
  "preview": 5,
  "calls": "down",
  "depth": 2
}
```

| Field | Type | Description |
|-------|------|-------------|
| `files` | string or [strings] | Glob pattern(s) to filter files |
| `files_exclude` | [strings] | Glob pattern(s) to exclude files |
| `find` | string or [strings] | Symbol kind(s): function, method, class, type, enum, struct, trait, variable, constant, property, namespace, module, macro, union, arrow_function, constructor, import, any |
| `name` | string or {contains, regex} | Name filter: glob string, `{"contains": "auth"}`, or `{"regex": "^get[A-Z]"}` |
| `visibility` | string | Filter by visibility: exported, public, private, protected, internal |
| `inside` | string | Only symbols inside a parent with this name |
| `has` | string, [strings], or {not: string} | Filter by associated comment/decorator text; `{"not": "docstring"}` for inverse |
| `lines` | {min, max} | Filter by line count |
| `body` | bool | Include full source body in results |
| `preview` | number | Number of preview lines to include |
| `calls` | string | Call graph traversal: "down" (callees), "up" (callers), "both" |
| `depth` | number | Call graph depth (default 1, max 5) |
| `format` | string | Override output format from within query JSON |
| `read` | string | File path to read (returns content instead of symbols). Combine with `lines` for range |

## Output Formats

`--out` flag controls result format (all output is JSON):

| Format | Content |
|--------|---------|
| `outline` | name, kind, file, line, signature (default) |
| `snippet` | outline + preview lines + docstring |
| `full` | outline + full body |
| `tree` | hierarchical: file -> class -> methods |
| `locations` | `file:line` only |
| `summary` | counts by kind and file |

Wrapping structure:
```json
{
  "project": "myapp",
  "query_ms": 42,
  "files_parsed": 8,
  "total": 3,
  "results": [ ... ]
}
```

## Project Structure

```
src/
├── main.rs            # CLI entry, dispatch to registry/query/audit
├── cli.rs             # Clap subcommand definitions (Create/List/Delete/Query + audit commands)
├── registry.rs        # Project CRUD: ~/.virgil-cli/projects.json (atomic write via .tmp + rename)
├── query_lang.rs      # JSON query schema deserialization (TsQuery, filters, serde untagged enums)
├── query_engine.rs    # On-demand parse + filter pipeline (rayon parallel, per-file symbol extraction)
├── format.rs          # Query output formatting (outline/snippet/full/tree/locations/summary)
├── signature.rs       # One-line signature extraction from AST nodes
├── discovery.rs       # File walking with .gitignore support (ignore crate)
├── file_source.rs     # FileSource trait + MemoryFileSource (in-memory, Arc<str> zero-copy)
├── language.rs        # Language enum, extension mapping, parser selection
├── lib.rs             # Public API: re-exports all modules
├── models.rs          # Data structs: FileMetadata, SymbolInfo, SymbolKind, ImportInfo, CommentInfo
├── parser.rs          # Tree-sitter parsing, file metadata collection
├── s3.rs              # S3/R2 client: URI parsing, object listing, concurrent download
├── server.rs          # Persistent HTTP server mode (axum, serves queries/audits from in-memory workspace)
├── workspace.rs       # Workspace: discover + load files into memory (rayon parallel), S3 loading
├── graph/
│   ├── mod.rs         # CodeGraph struct, NodeWeight/EdgeWeight enums, traversal methods
│   ├── builder.rs     # GraphBuilder: parallel extraction, graph assembly, CFG building
│   ├── cfg.rs         # CFG types: BasicBlock, CfgStatement, CfgEdge, FunctionCfg
│   ├── taint.rs       # Taint propagation engine + source/sink/sanitizer tables
│   ├── resource.rs    # Resource lifecycle analysis (acquire/release tracking)
│   └── cfg_languages/
│       ├── mod.rs     # CfgBuilder trait + language dispatch
│       ├── typescript.rs  # JS/TS/TSX/JSX CFG builder
│       ├── python.rs      # Python CFG builder
│       ├── rust_lang.rs   # Rust CFG builder
│       ├── go.rs          # Go CFG builder
│       ├── java.rs        # Java CFG builder
│       ├── c_lang.rs      # C CFG builder
│       ├── cpp.rs         # C++ CFG builder
│       ├── csharp.rs      # C# CFG builder
│       └── php.rs         # PHP CFG builder
├── languages/
│   ├── mod.rs         # Language-agnostic dispatch: compile queries, extract symbols/imports/comments
│   ├── typescript.rs  # All TS/JS/TSX/JSX tree-sitter queries and extraction
│   ├── c_lang.rs      # C tree-sitter queries and extraction
│   ├── cpp.rs         # C++ tree-sitter queries and extraction
│   ├── csharp.rs      # C# tree-sitter queries and extraction
│   ├── rust_lang.rs   # Rust tree-sitter queries and extraction
│   ├── python.rs      # Python tree-sitter queries and extraction
│   ├── go.rs          # Go tree-sitter queries and extraction
│   ├── java.rs        # Java tree-sitter queries and extraction
│   └── php.rs         # PHP tree-sitter queries and extraction
└── audit/             # Static analysis pipelines
```

## Architecture

- **On-demand parsing**: No pre-indexing or database. Every query discovers files, parses them with tree-sitter in parallel (rayon), applies filters, and returns results. Scoped by `files` glob to avoid parsing the entire project when unnecessary.
- **Project registry**: `~/.virgil-cli/projects.json` stores project name, path, language filter, and file count stats. Atomic writes via `.tmp` + rename.
- **Parsing**: tree-sitter with S-expression queries. Language-specific modules in `src/languages/`.
- **Language modules**: All tree-sitter queries and extraction logic for a language family live in one file. The `languages/mod.rs` dispatches based on `Language` enum.
- **File discovery**: `ignore` crate — respects .gitignore, skips node_modules/dist/build automatically.
- **Parallelism**: rayon. `Parser` is not Send — create per rayon task. `Query` objects are Arc-shared.
- **tree-sitter 0.25**: `QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`.
- **Query pipeline**: `query_engine::execute()` follows the same rayon pattern as `AuditEngine::run()` — discover files, pre-compile queries into `HashMap<Language, Arc<Query>>`, `par_iter` with per-task Parser, filter per-file, flatten + sort + limit.
- **Name matching**: glob (via `globset`), substring (`contains`), or regex (`regex` crate).
- **CodeGraph**: petgraph-backed `DiGraph<NodeWeight, EdgeWeight>` providing cross-file analysis. Nodes: File, Symbol, CallSite, Parameter, ExternalSource. Edges: DefinedIn, Calls, Imports, FlowsTo, SanitizedBy, Exports, Acquires, ReleasedBy, Contains. Built by `GraphBuilder` in parallel (rayon), includes per-function CFGs built by language-specific `CfgBuilder` implementations. Taint analysis via `TaintEngine` computes FlowsTo/SanitizedBy edges. Resource lifecycle via `ResourceAnalyzer` computes Acquires/ReleasedBy edges.
- **PipelineContext**: Wraps tree + source + graph reference, passed to `Pipeline::check_with_context()`. Default delegates to `check_with_ids()` for backward compat. Pipelines override `check_with_context()` to access graph-based taint paths.
- **Call graph**: Name-based resolution via `CodeGraph.symbols_by_name` lookup. BFS traversal up to configurable depth via `traverse_callees()`/`traverse_callers()` methods on `CodeGraph`.
- **Signature extraction**: Takes source text from symbol start line to first `{`, trims. Multi-line support (up to 5 lines). Python stops at `:`.
- **Output formats**: All JSON. `QueryOutputFormat` enum (outline/snippet/full/tree/locations/summary). `--pretty` controls indentation.
- **In-memory workspace**: `Workspace::load()` reads all project files into memory via rayon, stored as `Arc<str>`. Query engine and audit engine operate on workspace, not disk.
- **FileSource trait**: Pluggable file source (`file_source.rs`). `MemoryFileSource` backed by HashMap.
- **Server mode**: `serve` command loads codebase from S3 once, starts an axum HTTP server, serves queries/audits from in-memory workspace. Blocking work (`query_engine::execute`, `AuditEngine::run`) runs via `tokio::task::spawn_blocking` to avoid blocking the async runtime. 120-second request timeout. Binds to `127.0.0.1` by default (no auth). Ready signal `{"ready": true, "port": N}` printed to stdout after successful startup.
- **S3 support**: `--s3 s3://bucket/prefix` on query and audit commands. Downloads files from S3-compatible storage (AWS S3, Cloudflare R2, MinIO) into `MemoryFileSource`. No project registration needed. Uses `aws-sdk-s3` with custom endpoint support. Credentials via `S3_ACCESS_KEY_ID`/`S3_SECRET_ACCESS_KEY`/`S3_ENDPOINT` env vars (falls back to `AWS_*` equivalents and standard AWS credential chain). Concurrent downloads with 64-semaphore bounded parallelism. Region defaults to "auto" for R2 compatibility.

## Supported Languages

TypeScript (.ts), TSX (.tsx), JavaScript (.js), JSX (.jsx), C (.c, .h), C++ (.cpp, .cc, .cxx, .hpp, .hxx, .hh), C# (.cs), Rust (.rs), Python (.py, .pyi), Go (.go), Java (.java), PHP (.php)

## Symbol Kinds

function, class, method, variable, interface, type_alias, enum, arrow_function, struct, union, namespace, macro, property, typedef, trait, constant, module

## Import Kinds

static, dynamic, require, re_export, include, using, use, import, from

## Comment Kinds

line, block, doc

## Key Decisions

- Export detection: checks if definition node's parent is an `export_statement`
- Arrow functions: detected via variable declarator value child node kind
- Destructured variables: skipped (name child is not an identifier)
- Parse errors: warn + skip file, continue processing
- Import `kind` is a free-form String (not an enum) so new languages can define their own kinds without modifying a central type
- `is_external` classification: internal = starts with `.` or `#` (relative paths, Node.js subpath imports); external = everything else (bare specifiers, scoped packages, builtins).
- Comment classification: `/**` → "doc", `/*` → "block", `//` → "line". Associated symbol detected via `next_named_sibling()` of comment node, drilling through `export_statement` and `variable_declarator` as needed.
- C export detection: `static` storage class = not exported, everything else = exported (external linkage). Macros/types always exported.
- C/C++ imports: `#include <header>` → external, `#include "header"` → internal. Kind = "include".
- C# export detection: `public`/`internal` modifier = exported, `private`/`protected` = not exported. Namespaces always exported. Default = not exported (conservative).
- C# imports: `using` directives. Kind = "using". All treated as external (no syntactic way to distinguish).
- `.h` files map to C (design choice). C++ headers should use `.hpp`/`.hxx`/`.hh`.
- `Language::all_extensions()` returns all extensions per language (C++ has 6). Used in file discovery via `flat_map`.
- Rust export detection: `visibility_modifier` child = exported (any `pub` variant). No modifier = not exported.
- Rust imports: `use` declarations. Kind = "use". Internal = starts with `crate::`, `self::`, `super::`. External = everything else.
- Rust methods: `function_item` inside `impl_item` or `trait_item` (via `declaration_list` parent).
- Go export detection: first letter uppercase = exported, lowercase = not exported.
- Go imports: `import` declarations. Kind = "import". All treated as external. Last path segment = imported_name.
- Go type declarations: `struct_type` → Struct, `interface_type` → Interface, otherwise TypeAlias.
- Python export detection: name starts with `_` = not exported, otherwise exported.
- Python imports: `import` statements (kind = "import"), `from ... import` statements (kind = "from"). Relative imports (starts with `.`) = internal, absolute = external.
- Python methods: `function_definition` inside `class_definition` (walk parent chain, stop at function boundary).
- Python docstrings: `expression_statement > string` as first statement in function/class/module body → "doc" comment. Associated symbol from enclosing definition.
- Python `decorated_definition`: unwrap to inner function/class; skip bare `function_definition`/`class_definition` if parent is `decorated_definition` (deduplication).
- Java export detection: `public` modifier (inside `modifiers` wrapper node) = exported; `private`/`protected`/package-private = not exported. Default = not exported (conservative).
- Java imports: `import` declarations. Kind = "import" (or "static" for static imports). All treated as external. Wildcard `import java.util.*` uses `*` as imported_name.
- Java `modifiers` wrapper: unlike C#'s flat `modifier` children, Java wraps access modifiers in a `modifiers` parent node.
- Java symbol mapping: `record_declaration` → Class, `annotation_type_declaration` → Interface.
- PHP grammar: uses `LANGUAGE_PHP` (handles `<?php` tags), not `LANGUAGE_PHP_ONLY`. Only `.php` extension (not `.phtml`).
- PHP export detection: top-level functions/classes/interfaces/traits/enums/namespaces = always exported. Methods/properties/constants: `visibility_modifier` checked — `public` = exported, `private`/`protected` = not. Default = exported (PHP's default is public).
- PHP imports: `use` statements (kind = "use", always external). `require`/`include` (kind = "require"/"include", starts with `.` = internal, else external). Grouped use (`use App\Models\{User, Post}`) expanded to individual imports.
- PHP property names: `$` prefix stripped from variable names for clean symbol output.
- CodeGraph: petgraph `DiGraph` is `Send` but not `Sync`. After building, share via `Arc<CodeGraph>` for read-only access across rayon threads. CFG building is parallelized per-file during graph construction.
- Call graph: name-based resolution via `symbols_by_name` lookup — heuristic, no type info. BFS with configurable depth (max 5). Replaces old `call_graph.rs`.
- `find: "function"` matches both `Function` and `ArrowFunction` kinds. `find: "constructor"` matches `Method` kind (post-filter by name: "constructor", "__init__", "__construct", "new").
- `inside` filter: containment check via line range comparison against all symbols in the same file.
- `has` filter: cross-references with comment extraction. `{"not": "docstring"}` = inverse match for symbols without doc comments.
- Audit `architecture` category: 6th audit category with 4 pipelines (`module_size_distribution`, `circular_dependencies`, `dependency_graph_depth`, `api_surface_area`) and 9 patterns across all 11 supported languages. Uses per-file proxy approach for circular dependency detection (fan-out counting) since the `Pipeline::check()` trait operates on single files. True cross-file cycle detection deferred to future engine-level pass.
- Architecture thresholds: oversized_module >= 30 symbols OR >= 1000 lines (warning), monolithic_export_surface >= 20 exported symbols (info), anemic_module == 1 symbol excl. entry files (info), hub_module_bidirectional >= 5 intra-project imports (info), barrel_file_reexport >= 5 re-exports (warning), deep_import_chain >= 4 path depth (info), excessive_public_api >= 10 symbols AND > 80% exported (info), leaky_abstraction_boundary = exported types with public fields (warning).
- Workspace loads files upfront — trades memory for I/O speed. All file reads during query/audit come from in-memory `Arc<str>`.
- `read` query field bypasses symbol extraction, returns file content directly. Combine with `lines` for range reads.
- `lib.rs` re-exports all modules for library use (allows `use virgil_cli::query_engine` etc.).
- S3 does NOT use the project registry. `--s3` bypasses `projects create` entirely — files are downloaded into memory and used directly.
- S3 workspace root is a synthetic `s3://bucket/prefix` path. `execute_read()` disk fallback is guarded by `root.exists()` to avoid filesystem access on S3 workspaces.
- S3 credentials: `S3_ACCESS_KEY_ID`/`S3_SECRET_ACCESS_KEY`/`S3_ENDPOINT` env vars checked first, then `AWS_*` equivalents, then standard AWS credential chain.
- S3 `--s3` flag conflicts with positional `name`/`dir` args via `#[arg(conflicts_with)]`.
- S3 query constructs a minimal `ProjectEntry` with dummy values so `query_engine::execute()` can reuse the same code path.
- Server mode is S3-only (no `--path` flag). The caller is always Virgil Live (cloud service), codebases always come from S3.
- Server binds to `127.0.0.1` by default (no auth). Override with `--host 0.0.0.0` for network access.
- Server uses `tokio::task::spawn_blocking` for all query/audit handlers because `query_engine::execute()` and `AuditEngine::run()` use rayon internally (CPU-bound, would block tokio worker threads).
- Server request timeout: 120 seconds via `tokio::time::timeout`. Returns HTTP 504 on expiry.
- Server `--lang` filter stored in `AppState.languages` and used by all audit handlers. Without it, audit handlers would default to `Language::all()` ignoring the startup filter.
- Server audit summary collects per-category errors instead of silently swallowing them. Returns partial results with `"errors"` array. Returns 500 only if all categories fail.
- Server ready signal goes to stdout (`{"ready": true, "port": N}`), diagnostic messages to stderr. Caller reads stdout to detect readiness.
- Server `--port 0` uses OS-assigned dynamic port. Actual port reported in ready signal.

<!-- GSD:project-start source:PROJECT.md -->
## Project

**virgil-cli — Audit Pipeline JSON Migration**

virgil-cli is a Rust CLI tool that parses TypeScript, JavaScript, C, C++, C#, Rust, Python, Go, Java, and PHP codebases on-demand and queries them with a composable JSON query language and runs static analysis audits. The audit system currently has two coexisting implementations: hundreds of legacy Rust pipeline files and a newer JSON-driven engine. This milestone migrates all remaining Rust pipelines to the JSON-driven approach, removes the old code, and restores test health.

**Core Value:** All audit pipelines run as declarative JSON definitions — no Rust code required to add, modify, or ship an audit rule.

### Constraints

- **Tech stack**: Rust — all pipeline definitions must be valid JSON that the existing `json_audit.rs` engine can parse
- **Compatibility**: Pipeline names must remain identical (they appear in CLI output, API responses, and `--pipeline` filter flags)
- **No regressions**: `cargo test` must pass after every phase; no partial states where Rust + JSON pipelines conflict
- **Specs first**: audit_plans/ documents are authoritative — JSON pipelines should reflect the improved detection logic described there, not just re-implement the Rust bugs
<!-- GSD:project-end -->

<!-- GSD:stack-start source:codebase/STACK.md -->
## Technology Stack

## Languages
- Rust 2024 edition - CLI application, core parsing engine, AST analysis, audit pipelines
## Runtime
- Rust toolchain (stable)
- Multi-platform: Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64)
- Cargo
- Lockfile: `Cargo.lock` (present)
## Frameworks
- tree-sitter 0.25 - AST parsing for 13 programming languages
- clap 4.5 - CLI argument parsing with `derive` macros
- axum 0.8 - Async HTTP server for persistent query/audit API
- tokio 1 - Async runtime with multi-threaded executor
- petgraph 0.7 - Control flow graphs and call graphs
- rayon 1.11 - Parallel file parsing and filter pipelines
- serde + serde_json 1 - JSON serialization (queries, audit results, project registry)
- regex 1 - Name matching filters
- globset 0.4 - Glob pattern matching for file discovery and exclusion
- ignore 0.4 - .gitignore-aware file discovery
- streaming-iterator 0.1 - tree-sitter QueryMatches streaming
- indicatif 0.17 - Progress bars for CLI output
- dirs 5 - Platform-aware home directory detection
- chrono 0.4 - Timestamps in project registry and metadata
- anyhow 1.0 - Error handling
- aws-sdk-s3 1 - S3/R2/MinIO client
- aws-config 1 - AWS credential chain and configuration
## Key Dependencies
- tree-sitter v0.25 with language grammars - Powers all symbol extraction and AST analysis. Must not be downgraded due to `QueryMatches` API changes.
- clap - CLI definition and dispatch
- axum + tokio - HTTP server for persistent query mode
- petgraph - Call graph and control flow graph construction
- rayon - Parallelism for file discovery and per-file analysis
- aws-sdk-s3 - S3-compatible storage access (AWS S3, Cloudflare R2, MinIO)
- globset - File filtering with glob patterns
- ignore - .gitignore compatibility
- serde_json - All output is JSON (queries, audit results, registry)
- regex - Pattern matching in symbol name filters
- streaming-iterator - tree-sitter streaming API requirement
## Configuration
- `.env` file present - contains environment variables (note: never read contents)
- AWS/S3 credentials via env vars:
- `Cargo.toml` - Single workspace, 13 language tree-sitter parsers as dependencies
- Release profile (`[profile.release]`):
- Binary installer metadata: `[package.metadata.binstall]` - Cargo binstall support for precompiled releases
## Platform Requirements
- Rust stable toolchain (tested on ci.yml)
- Platform-specific: Ubuntu, macOS, Windows CI runners
- Dependencies: tree-sitter C/C++ libraries compiled per-platform
- Deployment: Standalone binary (pre-built for Linux/macOS/Windows, x86_64/aarch64)
- Storage: Local filesystem (project registry at `~/.virgil-cli/projects.json`) OR S3-compatible bucket
- HTTP: Optional persistent server mode (tokio async, no external process manager needed)
- Memory: In-memory file loading (all project files loaded at startup into `MemoryFileSource`)
## External Services
- AWS S3 - Primary cloud storage option
- Cloudflare R2 - Supported via custom endpoint
- MinIO - Supported via custom endpoint
- Optional: queries and audits can read directly from S3 without local registration
<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->
## Conventions

## Naming Patterns
- Snake case for module files: `query_engine.rs`, `file_source.rs`, `query_lang.rs`
- Enum variants use PascalCase: `Language::TypeScript`, `SymbolKind::Function`
- Descriptive module names reflect functionality: `parser.rs`, `discovery.rs`, `signature.rs`
- Snake case throughout: `create_parser()`, `parse_file()`, `extract_symbols()`, `discover_files()`
- Public functions use lowercase: `execute()`, `format_results()`, `load_registry()`
- Internal helpers start with underscores if needed, but mostly follow public convention
- Test functions use clear descriptive names: `full_pipeline_typescript()`, `discover_fixtures()`
- Snake case for all variables: `file_count`, `symbol_info`, `is_exported`, `per_file_results`
- Boolean prefixes: `is_exported`, `is_external`, `is_terminal()` pattern respected
- Destructured bindings from matches use clear names: `name_cap`, `def_cap`, `value_cap`
- Loop variables: `p` for projects, `sym` for symbols, `lang` for languages, `entry` for registry entries
- Structs use PascalCase: `FileMetadata`, `SymbolInfo`, `ProjectEntry`, `QueryResult`
- Enum variants use PascalCase: `Language::TypeScript`, `SymbolKind::Function`, `QueryOutputFormat::Outline`
- Trait implementations are inferred from context (no Trait suffix pattern, use `impl Display for SymbolKind`)
## Code Style
- Rust standard formatting (via implicit `rustfmt` conventions)
- 4-space indentation (Rust standard)
- Lines organized for readability with clear blank lines between logical sections
- Comments on their own line above code blocks (e.g., `// ── Symbol queries ──`)
- `#[allow(clippy::should_implement_trait)]` used sparingly when trait conversion is intentionally omitted (`SymbolKind::from_str()`)
- All unused imports are removed
- Pattern matching preferred over if-let chains in most cases
- Guard clauses used to exit early: `if let Some(ref kinds) = find_kinds && !kinds.contains(&sym.kind) { continue; }`
## Import Organization
- No path aliases used; full module paths preferred for clarity
- Imports grouped logically by domain: `use crate::query_lang::{FindFilter, HasFilter, NameFilter, TsQuery}`
- Common patterns: `use crate::language::Language`, `use crate::models::{SymbolInfo, SymbolKind}`
## Error Handling
- `anyhow::Result<T>` used universally for fallible operations
- `.context()` for adding contextual messages: `.context("failed to read projects.json")?`
- `.with_context(|| ...)` for formatted context: `.with_context(|| format!("failed to read {}", path.display()))?`
- Early exits with `bail!()` for validation errors: `bail!("project '{}' already exists", name)`
- `ok_or_else()` with context for conversions: `.ok_or_else(|| anyhow::anyhow!("..."))?`
- Match expressions for handling variants (no panic on common failures)
- `.unwrap_or_else()` for JSON serialization fallbacks: `serde_json::to_string(&wrapper).unwrap_or_else(|_| "{}".to_string())`
- `.filter_map()` for graceful skipping of problematic files during parsing
- Warnings printed to stderr via `eprintln!()`, not panics
- `.ok()?` chains in parallel iterators to skip files with parsing issues
## Logging
- Status messages to stderr: `eprintln!("Created project '{}'", entry.name);`
- User feedback always goes to stderr; JSON results to stdout only
- Progress printed when meaningful (file counts, language breakdown)
- No debug-level logging in final code (tests use `.expect()` liberally)
## Comments
- Section headers use ASCII art: `// ── Symbol queries ──`, `// ── Query compilation ──`
- Public struct documentation via doc comments (rarely used, most is self-explanatory)
- Complex tree-sitter queries explained near the constant definitions
- No inline comments for obvious code; only for non-obvious logic
- Tree-sitter S-expression queries documented at point of definition (see `TS_SYMBOL_QUERY`, `COMMENT_QUERY`)
- Language-specific rules noted in module docs (see CLAUDE.md integration)
## Function Design
- Most functions 15-50 lines
- Extraction functions (`extract_symbols`, `extract_imports`) are deterministic and focused
- Parallel processing functions leverage rayon without inline closures > 30 lines
- Functions accept required inputs as positional arguments
- Optional parameters use `Option<T>` or defaults
- File paths as `&Path` or `&str`, never `String` unless ownership needed
- References preferred: `&Workspace`, `&Query`, `&ProjectEntry`
- `Result<T>` for any fallible operation
- `Option<T>` for optional results (e.g., `extract_signature()` returns `Option<String>`)
- Tuples for related return values: `parse_file()` returns `(FileMetadata, Tree)`
- Vec for collections: `Vec<SymbolInfo>`, `Vec<QueryResult>`
## Module Design
- `pub fn` for public API; `fn` for internal
- `pub struct` for domain models; derive `Debug`, `Clone` where applicable
- Language-specific modules (e.g., `languages/typescript.rs`) export compilation and extraction functions
- `lib.rs` re-exports all public modules for library consumers: `pub use graph::*;`
- `src/languages/mod.rs` re-exports all language module functions (no barrel pattern for individual symbols)
- `src/graph/mod.rs` exports primary types: `CodeGraph`, `NodeWeight`, `EdgeWeight`
- Top-level `lib.rs` uses explicit re-exports: `pub use graph::builder;`
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->
## Architecture

## Pattern Overview
- No pre-indexing or database — files are parsed at query time
- Parallel processing via rayon for I/O-bound discovery and CPU-bound parsing
- Multi-layered command pipeline: CLI → Registry/Workspace → Query/Audit Engine → Language-specific parsers → Output formatter
- In-memory file loading (workspace layer) decouples from filesystem; enables S3 support
- Pluggable `FileSource` trait for different storage backends
- CodeGraph abstraction for cross-file analysis (call graphs, taint propagation, resource lifecycles)
- Per-language tree-sitter queries compiled once and shared via Arc across rayon threads
- Audit pipelines as trait objects allowing heterogeneous pipeline implementations
## Layers
- Purpose: Command dispatch and argument parsing
- Location: `src/cli.rs`, `src/main.rs`
- Contains: Clap command/subcommand definitions for `projects`, `audit`, `serve` subcommands; argument validation
- Depends on: Registry, Workspace, QueryEngine, AuditEngine, Server
- Used by: Users via command line
- Purpose: Project CRUD operations; persistent project metadata storage
- Location: `src/registry.rs`
- Contains: `ProjectEntry` struct (name, path, language filter, file count, breakdown); atomic file write via `.tmp` + rename pattern
- Depends on: Language module for language parsing
- Used by: CLI projects subcommand, Workspace initialization
- Purpose: File discovery and in-memory loading; abstracts away filesystem/S3 differences
- Location: `src/workspace.rs`
- Contains: `Workspace` struct holding `Arc<str>` file contents, language map; file discovery via ignore crate (respects .gitignore)
- Depends on: FileSource trait, Discovery module, Language module, S3 module (optional, for S3 workspaces)
- Used by: Query engine, Audit engine, Server, Graph builder
- Purpose: Pluggable file access backend
- Location: `src/file_source.rs`
- Contains: `FileSource` trait (read_file, list_files); `MemoryFileSource` implementation backing both disk and S3 workspaces
- Depends on: None
- Used by: Workspace
- Purpose: Parse JSON query DSL and compile tree-sitter queries
- Location: `src/query_lang.rs`, `src/parser.rs`, `src/languages/mod.rs`
- Contains: `TsQuery` struct (filters: files, find, name, visibility, inside, has, lines, calls, depth, read, graph); filter enums (FindFilter, NameFilter, HasFilter); per-language tree-sitter query compilation and symbol/import/comment extraction
- Depends on: Language module, Models, tree-sitter crate
- Used by: Query engine, Format layer
- Purpose: Execute parsed queries against workspace; symbol filtering and result construction
- Location: `src/query_engine.rs`
- Contains: `execute()` function with rayon-parallel per-file parsing and filtering; `QueryOutput` and `QueryResult` structs; support for call graph traversal (via CodeGraph)
- Depends on: Workspace, Language module, Query language, Models, CodeGraph (optional, for call graph queries)
- Used by: CLI, Server
- Purpose: Tree-sitter setup and file metadata extraction
- Location: `src/parser.rs`
- Contains: `create_parser()` for per-thread parser instantiation; `parse_file()` for disk files, `parse_content()` for in-memory; metadata collection (size, lines, name, extension)
- Depends on: Language module, tree-sitter crate
- Used by: Query engine, Audit engine, Graph builder
- Purpose: Language-specific tree-sitter S-expression queries and symbol/import/comment extraction
- Location: `src/languages/` (one file per language family: typescript.rs, rust_lang.rs, c_lang.rs, cpp.rs, csharp.rs, python.rs, go.rs, java.rs, php.rs)
- Contains: Per-language query compilation (symbol_query, import_query, comment_query); extract_symbols(), extract_imports(), extract_comments() implementations
- Depends on: Models, tree-sitter crate, tree-sitter-language crates
- Used by: Query engine, Audit engine, Graph builder
- Purpose: Cross-file analysis via multi-stage graph construction and traversal
- Location: `src/graph/` (mod.rs, builder.rs, cfg.rs, cfg_languages/, pipeline.rs, taint.rs, resource.rs, executor.rs)
- Contains:
- Depends on: Workspace, Language module, Models, petgraph crate
- Used by: Audit engine, Query engine (for call graph), Server
- Purpose: Static analysis across multiple code quality dimensions
- Location: `src/audit/engine.rs`, `src/audit/pipeline.rs`, `src/audit/pipelines/`, `src/audit/json_audit.rs`
- Contains:
- Depends on: Workspace, Language module, Models, Graph, Parser
- Used by: CLI, Server
- Purpose: Query result formatting and output wrapping
- Location: `src/format.rs`
- Contains: `format_results()` supporting outline/snippet/full/tree/locations/summary output formats; JSON wrapping with metadata (project, query_ms, files_parsed, total)
- Depends on: Query engine models, CLI output format enum
- Used by: CLI, Server
- Purpose: Query/audit remote codebases via S3-compatible storage
- Location: `src/s3.rs`
- Contains: `S3Location` struct (bucket, prefix); list_objects() with concurrent filtered downloads; credential chain (S3_*, AWS_*, standard AWS SDK chain)
- Depends on: Language module, aws-sdk-s3 crate
- Used by: Workspace (load_from_s3), CLI S3 flag handlers
- Purpose: Persistent HTTP server for remote/programmatic query and audit access
- Location: `src/server.rs`
- Contains: Axum-based HTTP router with `/health`, `/query` (POST), `/audit/summary` (POST), `/audit/{category}` (POST) endpoints; tokio blocking task dispatch for CPU-bound query/audit work; 120-second request timeout
- Depends on: Workspace, Query engine, Audit engine, Graph builder, Language module, axum, tokio
- Used by: CLI serve subcommand
- Purpose: Shared data structures across all layers
- Location: `src/models.rs`
- Contains: `FileMetadata` (path, name, extension, language, size_bytes, line_count); `SymbolInfo`, `ImportInfo`, `CommentInfo`; `SymbolKind` enum (17 kinds); `ParseError`
- Depends on: None
- Used by: All other layers
## Data Flow
- Query execution is stateless (query_engine::execute is pure given workspace + query)
- Graph is immutable once built (petgraph DiGraph), Arc-shared for read-only access across rayon threads
- Workspace keeps file contents in Arc<str> (zero-copy sharing)
- Per-thread Parser instances created fresh in rayon tasks (Parser is !Send, must be per-task)
- Audit findings are accumulated into Vec during parallel iteration, then sorted/formatted centrally
## Key Abstractions
- Purpose: Abstract file read/list operations; enables disk and S3 backends
- Examples: `src/file_source.rs`, `src/workspace.rs`
- Pattern: Trait object stored in Workspace; no dynamic dispatch in hot loop (file reads are outside rayon loop)
- Purpose: Pluggable analysis rules; enable Rust + JSON audit mixing
- Examples: `src/audit/pipeline.rs` (trait definition), `src/audit/pipelines/` (implementations)
- Pattern: Trait object `AnyPipeline` in pipeline map; heterogeneous Vec; per-file check() call in rayon loop
- Purpose: Multi-use cross-file analysis backbone
- Examples: `src/graph/mod.rs`, `src/graph/builder.rs`
- Pattern: petgraph DiGraph + HashMap indexes (file_nodes, symbol_nodes, symbols_by_name); built once, Arc-shared for reads; supports call graph BFS, taint analysis, resource tracking
- Purpose: User-friendly composable query language
- Examples: `src/query_lang.rs`
- Pattern: Serde-deserialized untagged enums (FindFilter, NameFilter, HasFilter) for flexible syntax; compile to tree-sitter queries during execution
- Purpose: Language-agnostic symbol classification
- Examples: `src/models.rs`
- Pattern: 17 kinds (function, class, method, variable, interface, type_alias, enum, arrow_function, struct, union, namespace, macro, property, typedef, trait, constant, module); used by all language modules for consistent output
## Entry Points
- Location: `src/main.rs`
- Triggers: `cargo run -- ...` or `virgil ...` (when installed)
- Responsibilities: CLI parsing via clap, dispatch to registry/query/audit/serve subcommands, error handling, eprintln for diagnostics
- Location: `src/lib.rs`
- Triggers: `use virgil_cli::...` in external code
- Responsibilities: Public API re-exports; enables library use (e.g., by server, by other tools)
- Location: `src/workspace.rs`
- Triggers: All query/audit flows; optionally followed by Workspace::load_from_s3()
- Responsibilities: File discovery, parallel loading into memory, language mapping
- Location: `src/query_engine.rs`
- Triggers: Query subcommand, Server /query endpoint
- Responsibilities: Compile queries, parallel per-file parse+filter, result collection, optional call graph traversal
- Location: `src/audit/engine.rs`
- Triggers: Audit subcommand, Server /audit/* endpoints
- Responsibilities: Pipeline selection, Graph building (optional), parallel per-file pipeline execution, finding aggregation
- Location: `src/server.rs`
- Triggers: Serve subcommand
- Responsibilities: Load workspace from S3, build CodeGraph, start axum HTTP server, dispatch requests to query/audit engines via tokio blocking
## Error Handling
- Parse errors: Per-file warnings to stderr, file skipped, processing continues (reported in audit summary)
- Registry I/O errors: Propagate anyhow::anyhow (user-facing); suggest creation if project not found
- Query deserialization errors: anyhow::anyhow with context (invalid JSON); user-friendly message
- Tree-sitter failures: Logged and skipped; continue with other files
- Graph construction failures: Audit engine provides fallback empty graph (graphs optional for some audits)
- S3 connection errors: Propagate from aws-sdk-s3 wrapped in anyhow
- Server timeouts: HTTP 504 after 120 seconds; query/audit interrupted by tokio::time::timeout
## Cross-Cutting Concerns
- File globs validated via globset crate (compile-time via Glob::new)
- Query JSON schema enforced via Serde (deserialization errors trap invalid fields)
- Language enum ensures only supported languages are used (from_extension returns Option)
- Symbol kind matched against SymbolKind enum (case-insensitive, unknown kinds rejected)
- rayon ThreadPool with configurable stack size (4MB in graph builder) to avoid stack overflow on deeply nested ASTs
- Per-thread Parser (tree-sitter::Parser is !Send)
- Arc-shared Query objects, Workspace FileSource, CodeGraph for read-only cross-thread access
- No locks in hot loop (grouping by language before par_iter avoids contention)
- Pre-compile tree-sitter queries per language once, Arc-share
- File discovery scoped by language (Language::all_extensions) to reduce parse attempts
- In-memory workspace trades RAM for I/O speed (no repeated disk reads)
- Parallel file loading (rayon) with max_file_size filter to skip large/binary files
- S3 concurrent downloads with 64-semaphore bounded parallelism
- Graph pipelines use stack-based iteration in helpers (avoid recursion depth limits)
<!-- GSD:architecture-end -->

<!-- GSD:skills-start source:skills/ -->
## Project Skills

| Skill | Description | Path |
|-------|-------------|------|
| grill-me | Interview the user relentlessly about a plan or design until reaching shared understanding, resolving each branch of the decision tree. Use when user wants to stress-test a plan, get grilled on their design, or mentions "grill me". | `.agents/skills/grill-me/SKILL.md` |
| skill-creator | Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Claude's capabilities with specialized knowledge, workflows, or tool integrations. | `.agents/skills/skill-creator/SKILL.md` |
| virgil | > Explore and query codebases using virgil-cli. Covers project registration, JSON query language for symbol search, call graph traversal, file reading, and static audit. Use when asked to analyze a codebase, understand architecture, find symbols, trace callers/callees, onboard to a project, investigate bugs, or map the API surface of any TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebase. | `.agents/skills/virgil/SKILL.md` |
<!-- GSD:skills-end -->

<!-- GSD:workflow-start source:GSD defaults -->
## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd-quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd-debug` for investigation and bug fixing
- `/gsd-execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

<!-- GSD:profile-start -->
## Developer Profile

> Profile not yet configured. Run `/gsd-profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
