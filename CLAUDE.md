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
| `overview` | Show codebase overview (language breakdown, top symbols, directories) |
| `search` | Search for symbols by name (fuzzy match) |
| `outline` | Show all symbols in a file |
| `files` | List parsed files |
| `read` | Read source file content with optional line ranges |
| `query` | Execute raw SQL against parquet files |

## Project Structure

```
src/
├── main.rs            # CLI entry, pipeline orchestration, rayon parallelism
├── cli.rs             # Clap subcommand definitions (7 subcommands + OutputFormat enum)
├── discovery.rs       # File walking with .gitignore support (ignore crate)
├── language.rs        # Language enum, extension mapping, parser selection
├── models.rs          # Data structs: FileMetadata, SymbolInfo, SymbolKind
├── parser.rs          # Tree-sitter parsing, file metadata collection
├── symbols.rs         # Symbol extraction via tree-sitter S-expression queries
├── output.rs          # Arrow schemas + parquet writing
└── query/
    ├── mod.rs         # Module re-exports
    ├── db.rs          # QueryEngine: DuckDB connection, view registration
    ├── format.rs      # Output formatting (table/json/csv)
    ├── search.rs      # Fuzzy symbol search
    ├── overview.rs    # Codebase overview (languages, top symbols, directories)
    ├── outline.rs     # File symbol outline
    ├── files.rs       # File listing with filters
    └── read.rs        # Source file reading with line ranges
```

## Architecture

- **Parsing**: tree-sitter with S-expression queries. `tree-sitter-typescript` for .ts/.tsx/.jsx, `tree-sitter-javascript` for .js.
- **File discovery**: `ignore` crate — respects .gitignore, skips node_modules/dist/build automatically.
- **Parallelism**: rayon. `Parser` is not Send — create per rayon task. `Query` objects are Arc-shared.
- **tree-sitter 0.25**: `QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`.
- **Output**: Two parquet files — `files.parquet` (file metadata) and `symbols.parquet` (extracted symbols).
- **Querying**: DuckDB in-memory connection. `QueryEngine::new()` registers parquet files as views (`files`, `symbols`) so all SQL — both internal and user-supplied via `query` — uses plain table names.
- **Output formats**: `OutputFormat` enum (table/json/csv) shared across all query subcommands. Formatting logic in `query/format.rs`.

## Supported Languages

TypeScript (.ts), TSX (.tsx), JavaScript (.js), JSX (.jsx)

## Symbol Kinds

function, class, method, variable, interface, type_alias, enum, arrow_function

## Key Decisions

- Export detection: checks if definition node's parent is an `export_statement`
- Arrow functions: detected via variable declarator value child node kind
- Destructured variables: skipped (name child is not an identifier)
- Parse errors: warn + skip file, continue processing
- DuckDB views: parquet files registered as `files` and `symbols` views at connection time — no raw `read_parquet()` paths in queries
