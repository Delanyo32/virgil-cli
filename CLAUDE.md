# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript codebases and outputs structured data as Parquet files.

## Build & Run

```bash
cargo build
cargo run -- <DIR> [--output <dir>] [--language ts,tsx,js,jsx]
```

Use `uv run --with pyarrow --with pandas` to run Python scripts for inspecting parquet output.

## Project Structure

```
src/
├── main.rs          # CLI entry, pipeline orchestration, rayon parallelism
├── cli.rs           # Clap argument definitions
├── discovery.rs     # File walking with .gitignore support (ignore crate)
├── language.rs      # Language enum, extension mapping, parser selection
├── models.rs        # Data structs: FileMetadata, SymbolInfo, SymbolKind
├── parser.rs        # Tree-sitter parsing, file metadata collection
├── symbols.rs       # Symbol extraction via tree-sitter S-expression queries
└── output.rs        # Arrow schemas + parquet writing
```

## Architecture

- **Parsing**: tree-sitter with S-expression queries. `tree-sitter-typescript` for .ts/.tsx/.jsx, `tree-sitter-javascript` for .js.
- **File discovery**: `ignore` crate — respects .gitignore, skips node_modules/dist/build automatically.
- **Parallelism**: rayon. `Parser` is not Send — create per rayon task. `Query` objects are Arc-shared.
- **tree-sitter 0.25**: `QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`.
- **Output**: Two parquet files — `files.parquet` (file metadata) and `symbols.parquet` (extracted symbols).

## Supported Languages

TypeScript (.ts), TSX (.tsx), JavaScript (.js), JSX (.jsx)

## Symbol Kinds

function, class, method, variable, interface, type_alias, enum, arrow_function

## Key Decisions

- Export detection: checks if definition node's parent is an `export_statement`
- Arrow functions: detected via variable declarator value child node kind
- Destructured variables: skipped (name child is not an identifier)
- Parse errors: warn + skip file, continue processing
