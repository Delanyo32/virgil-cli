# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go codebases into structured Parquet files and queries them with DuckDB.

## Build & Run

```bash
cargo build
cargo run -- parse <DIR> [--output <dir>] [--language ts,tsx,js,jsx,c,h,cpp,cc,cxx,hpp,cs,rs,py,pyi,go]
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
| `comments` | List comments with filters (`--file`, `--kind`, `--documented`, `--symbol`) |

## Project Structure

```
src/
├── main.rs            # CLI entry, pipeline orchestration, rayon parallelism
├── cli.rs             # Clap subcommand definitions (12 subcommands + OutputFormat/FileSortField enums)
├── discovery.rs       # File walking with .gitignore support (ignore crate)
├── language.rs        # Language enum, extension mapping, parser selection
├── models.rs          # Data structs: FileMetadata, SymbolInfo, SymbolKind, ImportInfo, CommentInfo
├── parser.rs          # Tree-sitter parsing, file metadata collection
├── output.rs          # Arrow schemas + parquet writing (files, symbols, imports, comments)
├── languages/
│   ├── mod.rs         # Language-agnostic dispatch: compile queries, extract symbols/imports/comments
│   ├── typescript.rs  # All TS/JS/TSX/JSX tree-sitter queries and extraction (symbols + imports + comments)
│   ├── c_lang.rs      # C tree-sitter queries and extraction (symbols + #include imports + comments)
│   ├── cpp.rs         # C++ tree-sitter queries and extraction (extends C with classes, namespaces)
│   ├── csharp.rs      # C# tree-sitter queries and extraction (classes, interfaces, using directives)
│   ├── rust_lang.rs   # Rust tree-sitter queries and extraction (functions, structs, traits, use imports)
│   ├── python.rs      # Python tree-sitter queries and extraction (functions, classes, imports, docstrings)
│   └── go.rs          # Go tree-sitter queries and extraction (functions, methods, structs, interfaces)
└── query/
    ├── mod.rs         # Module re-exports
    ├── db.rs          # QueryEngine: DuckDB connection, view registration (files, symbols, imports, comments)
    ├── format.rs      # Output formatting (table/json/csv)
    ├── search.rs      # Fuzzy symbol search
    ├── overview.rs    # Codebase overview (languages, top symbols, directories, dependency summary)
    ├── outline.rs     # File symbol outline
    ├── files.rs       # File listing with filters
    ├── read.rs        # Source file reading with line ranges
    ├── deps.rs        # File dependency listing (what does this file import?)
    ├── dependents.rs  # Reverse dependency lookup (what files import this file?)
    ├── callers.rs     # Find which files import a specific symbol
    ├── imports.rs     # Import listing with filters
    └── comments.rs    # Comment listing with filters
```

## Architecture

- **Parsing**: tree-sitter with S-expression queries. `tree-sitter-typescript` for .ts/.tsx/.jsx, `tree-sitter-javascript` for .js, `tree-sitter-c` for .c/.h, `tree-sitter-cpp` for .cpp/.cc/.cxx/.hpp/.hxx/.hh, `tree-sitter-c-sharp` for .cs, `tree-sitter-rust` for .rs, `tree-sitter-python` for .py/.pyi, `tree-sitter-go` for .go.
- **Language modules**: All tree-sitter queries and extraction logic for a language family live in one file (`languages/typescript.rs`). The `languages/mod.rs` dispatches based on `Language` enum. Adding a new language = add a new file, update the dispatch.
- **File discovery**: `ignore` crate — respects .gitignore, skips node_modules/dist/build automatically.
- **Parallelism**: rayon. `Parser` is not Send — create per rayon task. `Query` objects are Arc-shared.
- **tree-sitter 0.25**: `QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`.
- **Output**: Four parquet files — `files.parquet` (file metadata), `symbols.parquet` (extracted symbols), `imports.parquet` (extracted imports), `comments.parquet` (extracted comments).
- **Querying**: DuckDB in-memory connection. `QueryEngine::new()` registers parquet files as views (`files`, `symbols`, conditionally `imports`, conditionally `comments`) so all SQL uses plain table names. The `imports` and `comments` views are backward-compatible — only registered if their parquet files exist.
- **Output formats**: `OutputFormat` enum (table/json/csv) shared across all query subcommands. Formatting logic in `query/format.rs`.

## Supported Languages

TypeScript (.ts), TSX (.tsx), JavaScript (.js), JSX (.jsx), C (.c, .h), C++ (.cpp, .cc, .cxx, .hpp, .hxx, .hh), C# (.cs), Rust (.rs), Python (.py, .pyi), Go (.go)

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
