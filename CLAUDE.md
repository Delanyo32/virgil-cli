# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript codebases into structured Parquet files and queries them with DuckDB.

## Build & Run

```bash
cargo build
cargo run -- parse <DIR> [--output <dir>] [--language ts,tsx,js,jsx]
cargo run -- search <QUERY> [--data-dir <dir>] [--kind <kind>] [--exported]
cargo run -- query <SQL> [--data-dir <dir>] [--format table|json|csv]
```

Use `uv run --with pyarrow --with pandas` to run Python scripts for inspecting parquet output.

## Subcommands

| Command | Description |
|---------|-------------|
| `parse` | Parse a codebase and output parquet files |
| `overview` | Show codebase overview (language breakdown, top symbols, directories, dependency summary) |
| `search` | Search for symbols by name (fuzzy match) |
| `outline` | Show all symbols in a file |
| `files` | List parsed files |
| `read` | Read source file content with optional line ranges |
| `query` | Execute raw SQL against parquet files |
| `deps` | Show what a file imports (dependencies) |
| `dependents` | Show what files import a given file (reverse dependencies) |
| `callers` | Find which files import a specific symbol |
| `imports` | List all imports with filters (`--module`, `--kind`, `--file`, `--type-only`, `--external`, `--internal`) |

## Project Structure

```
src/
├── main.rs            # CLI entry, pipeline orchestration, rayon parallelism
├── cli.rs             # Clap subcommand definitions (11 subcommands + OutputFormat/FileSortField enums)
├── discovery.rs       # File walking with .gitignore support (ignore crate)
├── language.rs        # Language enum, extension mapping, parser selection
├── models.rs          # Data structs: FileMetadata, SymbolInfo, SymbolKind, ImportInfo
├── parser.rs          # Tree-sitter parsing, file metadata collection
├── output.rs          # Arrow schemas + parquet writing (files, symbols, imports)
├── languages/
│   ├── mod.rs         # Language-agnostic dispatch: compile queries, extract symbols/imports
│   └── typescript.rs  # All TS/JS/TSX/JSX tree-sitter queries and extraction (symbols + imports)
└── query/
    ├── mod.rs         # Module re-exports
    ├── db.rs          # QueryEngine: DuckDB connection, view registration (files, symbols, imports)
    ├── format.rs      # Output formatting (table/json/csv)
    ├── search.rs      # Fuzzy symbol search
    ├── overview.rs    # Codebase overview (languages, top symbols, directories, dependency summary)
    ├── outline.rs     # File symbol outline
    ├── files.rs       # File listing with filters
    ├── read.rs        # Source file reading with line ranges
    ├── deps.rs        # File dependency listing (what does this file import?)
    ├── dependents.rs  # Reverse dependency lookup (what files import this file?)
    ├── callers.rs     # Find which files import a specific symbol
    └── imports.rs     # Import listing with filters
```

## Architecture

- **Parsing**: tree-sitter with S-expression queries. `tree-sitter-typescript` for .ts/.tsx/.jsx, `tree-sitter-javascript` for .js.
- **Language modules**: All tree-sitter queries and extraction logic for a language family live in one file (`languages/typescript.rs`). The `languages/mod.rs` dispatches based on `Language` enum. Adding a new language = add a new file, update the dispatch.
- **File discovery**: `ignore` crate — respects .gitignore, skips node_modules/dist/build automatically.
- **Parallelism**: rayon. `Parser` is not Send — create per rayon task. `Query` objects are Arc-shared.
- **tree-sitter 0.25**: `QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`.
- **Output**: Three parquet files — `files.parquet` (file metadata), `symbols.parquet` (extracted symbols), `imports.parquet` (extracted imports).
- **Querying**: DuckDB in-memory connection. `QueryEngine::new()` registers parquet files as views (`files`, `symbols`, conditionally `imports`) so all SQL uses plain table names. The `imports` view is backward-compatible — only registered if `imports.parquet` exists.
- **Output formats**: `OutputFormat` enum (table/json/csv) shared across all query subcommands. Formatting logic in `query/format.rs`.

## Supported Languages

TypeScript (.ts), TSX (.tsx), JavaScript (.js), JSX (.jsx)

## Symbol Kinds

function, class, method, variable, interface, type_alias, enum, arrow_function

## Import Kinds

static, dynamic, require, re_export

## Key Decisions

- Export detection: checks if definition node's parent is an `export_statement`
- Arrow functions: detected via variable declarator value child node kind
- Destructured variables: skipped (name child is not an identifier)
- Parse errors: warn + skip file, continue processing
- DuckDB views: parquet files registered as `files`, `symbols`, and `imports` views at connection time — no raw `read_parquet()` paths in queries
- Import `kind` is a free-form String (not an enum) so new languages can define their own kinds without modifying a central type
- `imports` view registered conditionally for backward compatibility with data dirs that predate import support
- `is_external` classification: internal = starts with `.` or `#` (relative paths, Node.js subpath imports); external = everything else (bare specifiers, scoped packages, builtins). Computed at parse time and stored in parquet. Old parquet files without this column get it synthesized via SQL in the view registration.
