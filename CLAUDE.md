# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases into structured Parquet files and queries them with DuckDB.

## Build & Run

```bash
cargo build

# Project management (persistent, query by name)
cargo run -- project create <DIR> [--name <name>] [--language <filter>]
cargo run -- project list
cargo run -- project delete <NAME>

# Project query (native subcommands, auto-resolves data paths)
cargo run -- project query <NAME> overview [--format] [--depth]
cargo run -- project query <NAME> search <QUERY> [--kind] [--exported] [--limit] [--offset] [--format]
cargo run -- project query <NAME> file get <PATH> [--format]
cargo run -- project query <NAME> file list [--language] [--directory] [--limit] [--offset] [--sort] [--format]
cargo run -- project query <NAME> file read <PATH> [--start-line] [--end-line]
cargo run -- project query <NAME> symbol get <NAME> [--format]
cargo run -- project query <NAME> comments list [--file] [--kind] [--documented] [--symbol] [--limit] [--format]
cargo run -- project query <NAME> comments search <QUERY> [--file] [--kind] [--limit] [--format]

# Audit management (complexity + quality analysis)
cargo run -- audit create <DIR> [--name <name>] [--language <filter>]
cargo run -- audit list
cargo run -- audit delete <NAME>
cargo run -- audit complexity <NAME> [--file] [--kind] [--sort cyclomatic|cognitive|name|file|lines] [--limit] [--threshold] [--format]
cargo run -- audit overview <NAME> [--format]                          # Combined complexity + quality overview
cargo run -- audit quality <NAME> dead-code [--file] [--kind] [--limit] [--format]
cargo run -- audit quality <NAME> coupling [--file] [--sort instability|fan-in|fan-out|file] [--limit] [--cycles] [--format]
cargo run -- audit quality <NAME> duplication [--file] [--min-group] [--limit] [--format]
cargo run -- audit security <NAME> unsafe-calls [--file] [--limit] [--format]
cargo run -- audit security <NAME> string-risks [--file] [--limit] [--format]
cargo run -- audit security <NAME> hardcoded-secrets [--file] [--limit] [--format]
```

Use `uv run --with pyarrow --with pandas` to run Python scripts for inspecting parquet output.

## Subcommands

| Command | Description |
|---------|-------------|
| `project` | Manage persistent projects (`create`, `list`, `delete`, `query`) |
| `audit` | Run code audits with complexity + quality analysis (`create`, `list`, `delete`, `complexity`, `overview`, `quality`) |

## Project Structure

```
src/
├── main.rs            # CLI entry, pipeline orchestration, rayon parallelism, project + audit handlers
├── cli.rs             # Clap subcommand definitions (project + audit commands, OutputFormat/FileSortField/ComplexitySortField enums)
├── discovery.rs       # File walking with .gitignore support (ignore crate)
├── language.rs        # Language enum, extension mapping, parser selection
├── models.rs          # Data structs: FileMetadata, SymbolInfo, SymbolKind, ImportInfo, CommentInfo, ComplexityInfo
├── parser.rs          # Tree-sitter parsing, file metadata collection (parse_file + parse_content for S3)
├── output.rs          # Arrow schemas + parquet writing (local + S3 variants, including complexity)
├── project.rs         # Project metadata types, JSON persistence (~/.virgil/), path helpers
├── audit.rs           # Audit metadata types, JSON persistence (~/.virgil/audits/), path helpers
├── complexity.rs      # Complexity engine: cyclomatic + cognitive + line count, per-language node-kind configs
├── security.rs        # Security engine: unsafe calls, string risks, hardcoded secrets detection via AST traversal
├── s3.rs              # S3 configuration (S3Config), client (S3Client), file listing/download/upload
├── languages/
│   ├── mod.rs         # Language-agnostic dispatch: compile queries, extract symbols/imports/comments
│   ├── typescript.rs  # All TS/JS/TSX/JSX tree-sitter queries and extraction (symbols + imports + comments)
│   ├── c_lang.rs      # C tree-sitter queries and extraction (symbols + #include imports + comments)
│   ├── cpp.rs         # C++ tree-sitter queries and extraction (extends C with classes, namespaces)
│   ├── csharp.rs      # C# tree-sitter queries and extraction (classes, interfaces, using directives)
│   ├── rust_lang.rs   # Rust tree-sitter queries and extraction (functions, structs, traits, use imports)
│   ├── python.rs      # Python tree-sitter queries and extraction (functions, classes, imports, docstrings)
│   ├── go.rs          # Go tree-sitter queries and extraction (functions, methods, structs, interfaces)
│   ├── java.rs        # Java tree-sitter queries and extraction (classes, interfaces, enums, records, imports)
│   └── php.rs         # PHP tree-sitter queries and extraction (classes, traits, namespaces, use/require imports)
└── query/
    ├── mod.rs         # Module re-exports
    ├── db.rs          # QueryEngine: DuckDB connection, view registration (local + S3 via httpfs)
    ├── format.rs      # Output formatting (table/json/csv)
    ├── search.rs      # Fuzzy symbol search
    ├── overview.rs    # Codebase overview (languages, top symbols, directories, dependency summary)
    ├── outline.rs     # File symbol outline
    ├── files.rs       # File listing with filters
    ├── read.rs        # Source file reading with line ranges (local + S3)
    ├── deps.rs        # File dependency listing (what does this file import?)
    ├── dependents.rs  # Reverse dependency lookup (what files import this file?)
    ├── callers.rs     # Find which files import a specific symbol
    ├── imports.rs     # Import listing with filters
    ├── comments.rs    # Comment listing with filters + text search
    ├── complexity.rs  # Complexity query functions (run_complexity, run_complexity_overview)
    ├── quality.rs     # Quality analysis (dead code, coupling/cohesion, duplication)
    ├── security.rs    # Security query functions (unsafe calls, string risks, hardcoded secrets)
    └── symbol.rs      # Symbol detail view (definition, callers, deps, docs)
```

## Architecture

- **Parsing**: tree-sitter with S-expression queries. `tree-sitter-typescript` for .ts/.tsx/.jsx, `tree-sitter-javascript` for .js, `tree-sitter-c` for .c/.h, `tree-sitter-cpp` for .cpp/.cc/.cxx/.hpp/.hxx/.hh, `tree-sitter-c-sharp` for .cs, `tree-sitter-rust` for .rs, `tree-sitter-python` for .py/.pyi, `tree-sitter-go` for .go, `tree-sitter-java` for .java, `tree-sitter-php` (LANGUAGE_PHP) for .php.
- **Language modules**: All tree-sitter queries and extraction logic for a language family live in one file (`languages/typescript.rs`). The `languages/mod.rs` dispatches based on `Language` enum. Adding a new language = add a new file, update the dispatch.
- **File discovery**: `ignore` crate — respects .gitignore, skips node_modules/dist/build automatically.
- **Parallelism**: rayon. `Parser` is not Send — create per rayon task. `Query` objects are Arc-shared.
- **tree-sitter 0.25**: `QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`.
- **Output**: Four parquet files for projects — `files.parquet` (file metadata), `symbols.parquet` (extracted symbols), `imports.parquet` (extracted imports), `comments.parquet` (extracted comments). Audits additionally produce `complexity.parquet` (per-symbol complexity metrics).
- **Querying**: DuckDB in-memory connection. `QueryEngine::new()` registers parquet files as views (`files`, `symbols`, conditionally `imports`, conditionally `comments`) so all SQL uses plain table names. The `imports` and `comments` views are backward-compatible — only registered if their parquet files exist.
- **Output formats**: `OutputFormat` enum (table/json/csv) shared across all query subcommands. Formatting logic in `query/format.rs`.
- **View existence check**: `has_imports()`/`has_comments()`/`has_errors()`/`has_complexity()` query `information_schema.tables` instead of filesystem.
- **Project management**: `project` top-level command with `create`/`list`/`delete`/`query` actions. Metadata stored as JSON at `~/.virgil/projects.json`. Parquet data stored under `~/.virgil/projects/<name>/`. `project query` uses native clap nesting (`ProjectQueryCommand` → `FileAction`/`SymbolAction`/`CommentsAction`) with `dispatch_project_query()` — auto-resolves `data_dir` and `repo_path` from project metadata.
- **Audit management**: `audit` top-level command with `create`/`list`/`delete`/`complexity`/`overview` actions. Metadata stored as JSON at `~/.virgil/audits.json`. Parquet data stored under `~/.virgil/audits/<name>/`. Mirrors project pattern but adds complexity computation via `run_parse(..., compute_complexity: true)`.
- **Complexity engine**: Language-agnostic tree-sitter node traversal (not S-expression queries) with per-language node-kind lookup tables in `ComplexityConfig`. Computes cyclomatic complexity (base 1, +1 per decision point), cognitive complexity (Sonar-style nesting-weighted), and function/method line count. Only applied to Function, Method, and ArrowFunction symbol kinds.

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
- DuckDB views: parquet files registered as `files`, `symbols`, and `imports` views at connection time — no raw `read_parquet()` paths in queries
- Import `kind` is a free-form String (not an enum) so new languages can define their own kinds without modifying a central type
- `imports` view registered conditionally for backward compatibility with data dirs that predate import support
- `is_external` classification: internal = starts with `.` or `#` (relative paths, Node.js subpath imports); external = everything else (bare specifiers, scoped packages, builtins). Computed at parse time and stored in parquet. Old parquet files without this column get it synthesized via SQL in the view registration.
- Comment classification: `/**` → "doc", `/*` → "block", `//` → "line". Associated symbol detected via `next_named_sibling()` of comment node, drilling through `export_statement` and `variable_declarator` as needed.
- `comments` view registered conditionally for backward compatibility with data dirs that predate comment support.
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
- Project storage: `~/.virgil/projects/<name>/` for parquet data, `~/.virgil/projects.json` for metadata. `dirs` crate for cross-platform home directory.
- Project names: validated (no path separators, no leading dot, no empty). Derived from directory basename if `--name` not provided.
- `project create`: canonicalizes directory path immediately, stores absolute `repo_path` in metadata. Reuses `run_parse()` directly. Cleans up data dir on parse failure.
- `project query`: uses native clap subcommand hierarchy (`ProjectQueryCommand` with `FileAction`, `SymbolAction`, `CommentsAction` nested enums). `dispatch_project_query()` loads project metadata and calls `run_*` functions directly — no synthetic arg building or re-parsing. Supports: `overview`, `search`, `file {get,list,read}`, `symbol get`, `comments {list,search}`.
- `symbol get`: queries symbol definition, import count, callers (top 20), file dependencies, and doc comments. Table output uses sections (like overview). JSON returns full `SymbolDetail` struct. CSV returns definitions only.
- `comments search`: text search via `ILIKE '%query%'` with optional file/kind filters. Reuses `CommentEntry` struct.
- `dispatch_project_query()`: handles project-scoped query commands.
- Audit storage: `~/.virgil/audits/<name>/` for parquet data, `~/.virgil/audits.json` for metadata. Mirrors project pattern exactly.
- `audit create`: calls `run_parse(..., compute_complexity: true)`. Produces all standard parquet files plus `complexity.parquet`.
- `run_parse()` takes `compute_complexity: bool` parameter. When `false` (project create), no complexity computed — zero overhead. When `true` (audit create), `complexity::extract_complexity()` runs in the same rayon parallel closure after symbol extraction.
- Complexity engine uses recursive AST node traversal with `node.kind()` checked against `ComplexityConfig` lookup tables — not tree-sitter S-expression queries. This is because complexity counting needs nesting depth tracking and recursive child walks.
- `ComplexityConfig`: per-language struct mapping branching node kinds, nesting node kinds, logical operator patterns, ternary node kind. Covers all 12 languages.
- Cyclomatic complexity: base 1, +1 for each: if, else-if, for, while, do-while, switch-case/match-arm, catch, `&&`/`||`/`??`, ternary.
- Cognitive complexity (Sonar-style): nesting constructs (if, for, while, switch, catch, ternary) increment by 1 + current_nesting_depth, then recurse with nesting+1. Non-nesting constructs (else, elif) increment by 1, same nesting. Logical operators: flat +1 each.
- `line_count`: stored as `end_line - start_line + 1` (saturating). Stored in parquet as a first-class field, not computed on the fly.
- Only Function, Method, and ArrowFunction get complexity scores (`is_complexity_relevant()`). Classes, structs, enums, etc. are skipped.
- `complexity` view registered conditionally in `QueryEngine::new()` (same pattern as imports/comments).
- `ComplexitySortField` enum: Cyclomatic, Cognitive, Name, File, Lines. Default sort = Cyclomatic DESC.
- `audit complexity`: supports `--file`, `--kind`, `--sort`, `--limit`, `--threshold` filters. Threshold filters on `cyclomatic_complexity >= N`.
- `audit overview`: combined complexity + quality + security overview. Complexity sections: summary stats (avg/max cyclomatic, cognitive, line count), distribution buckets (1-5 simple, 6-10 moderate, 11-20 complex, 21+ very complex), top 10 most complex symbols, per-file complexity aggregation. Quality sections: dead code summary, coupling summary, duplication summary. Security section: issue counts by type and severity. Dispatched to `query::quality::run_audit_overview`.
- `structural_hash`: u64 hash of a function's AST node-kind sequence (identifiers/literals stripped). Stored in `complexity.parquet`. Old audits without this column get it synthesized as `0` in the view registration.
- `audit quality`: nested subcommand (`AuditAction::Quality { name, command: QualityCommand }`). `QualityCommand` has `DeadCode`, `Coupling`, `Duplication` variants.
- `audit quality dead-code`: LEFT JOIN exported symbols with internal imports by name. Matches by exact symbol name only — may produce false positives for dynamic references or renamed re-exports.
- `audit quality coupling`: Fan-in/fan-out computed from `imports` view (internal only). Instability = fan_out / (fan_in + fan_out). `--cycles` flag runs Tarjan's SCC algorithm on the import graph built in Rust.
- `audit quality duplication`: Groups functions by (structural_hash, symbol_kind, line_count, cyclomatic, cognitive) where hash != 0 and count >= min_group. Old audits with synthesized hash=0 are filtered out.
- `CouplingSortField` enum: Instability, FanIn, FanOut, File.
- `dispatch_audit_quality()`: follows the same pattern as `dispatch_audit_complexity()` — load metadata, create engine, match on `QualityCommand`, print output.
- Security engine (`security.rs`): AST traversal (like `complexity.rs`) with per-language `SecurityConfig` lookup tables. Detects unsafe calls, string risks (inline SQL/HTML), and hardcoded secrets. Runs under `compute_complexity` flag — only during `audit create`.
- `SecurityConfig`: per-language struct mapping call node kinds, unsafe function names, string node kinds, variable declaration kinds. Covers all 12 languages.
- `SecurityIssue`: single model with `issue_type` discriminator (`"unsafe_call"`, `"string_risk"`, `"hardcoded_secret"`). Severity: unsafe_call = high, hardcoded_secret = high, string_risk = medium.
- `security.parquet`: stored alongside `complexity.parquet` in audit data dir. Schema: file_path, issue_type, severity, line, column, end_line, end_column, description, snippet, symbol_name.
- `security` view registered conditionally in `QueryEngine::new()` (same pattern as complexity). `has_security()` method for backward compat.
- `audit security`: nested subcommand (`AuditAction::Security { name, command: SecurityCommand }`). `SecurityCommand` has `UnsafeCalls`, `StringRisks`, `HardcodedSecrets` variants. Each supports `--file` and `--limit` filters.
- `audit overview`: includes security summary section (total, unsafe calls, string risks, hardcoded secrets, high/medium severity counts). Gated behind `has_security()` for backward compat.
- `dispatch_audit_security()`: follows the same pattern as `dispatch_audit_quality()` — load metadata, create engine, match on `SecurityCommand`, print output.
- Unsafe function detection: matches function-position child text against per-language unsafe function list. Supports dotted names (e.g. `os.system`, `exec.Command`).
- String risk detection: case-insensitive pattern matching for SQL keywords (`SELECT...FROM`, `INSERT INTO`, etc.) and HTML tags (`<script`, `<iframe`, etc.) in string literal nodes.
- Hardcoded secret detection: variable name checked case-insensitively against secret patterns (`api_key`, `password`, `token`, etc.); value must be a string literal node kind.
