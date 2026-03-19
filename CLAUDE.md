# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases on-demand and queries them with a composable JSON query language. No database, no pre-indexing — projects are registered by name and parsed at query time.

## Build & Run

```bash
cargo build
cargo run -- projects create myapp --path ./src [--lang ts,tsx,js,jsx] [--exclude "vendor/**"]
cargo run -- projects list
cargo run -- projects delete myapp
cargo run -- projects query myapp --q '{"find": "function", "name": "handle*"}' [--out outline|snippet|full|tree|locations|summary] [--pretty] [--max 100]
cargo run -- projects query myapp --file query.json
```

## Subcommands

All commands are nested under `virgil projects`:

| Command | Description |
|---------|-------------|
| `projects create` | Register a project for querying (scans files, saves to `~/.virgil-cli/projects.json`) |
| `projects list` | List registered projects with file counts |
| `projects delete` | Remove a registered project |
| `projects query` | Query a project using inline JSON (`--q`), a file (`--file`), or stdin |

Audit commands remain under `virgil audit` (unchanged).

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
├── call_graph.rs      # Call graph traversal (BFS, find callees/callers via tree-sitter call expressions)
├── discovery.rs       # File walking with .gitignore support (ignore crate)
├── language.rs        # Language enum, extension mapping, parser selection
├── models.rs          # Data structs: FileMetadata, SymbolInfo, SymbolKind, ImportInfo, CommentInfo
├── parser.rs          # Tree-sitter parsing, file metadata collection
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
└── audit/             # Static analysis pipelines (unchanged)
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
- **Call graph**: Name-based resolution (heuristic, no type info). BFS traversal up to configurable depth. Per-language call expression node types. "down" = callees within symbol subtree, "up" = scan all files for callers.
- **Signature extraction**: Takes source text from symbol start line to first `{`, trims. Multi-line support (up to 5 lines). Python stops at `:`.
- **Output formats**: All JSON. `QueryOutputFormat` enum (outline/snippet/full/tree/locations/summary). `--pretty` controls indentation.

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
- Call graph: name-based resolution is heuristic — no type info. Documented as best-effort. BFS with configurable depth (max 5). Per-language call expression node types.
- `find: "function"` matches both `Function` and `ArrowFunction` kinds. `find: "constructor"` matches `Method` kind (post-filter by name: "constructor", "__init__", "__construct", "new").
- `inside` filter: containment check via line range comparison against all symbols in the same file.
- `has` filter: cross-references with comment extraction. `{"not": "docstring"}` = inverse match for symbols without doc comments.
- Audit `architecture` category: 6th audit category with 4 pipelines (`module_size_distribution`, `circular_dependencies`, `dependency_graph_depth`, `api_surface_area`) and 9 patterns across all 11 supported languages. Uses per-file proxy approach for circular dependency detection (fan-out counting) since the `Pipeline::check()` trait operates on single files. True cross-file cycle detection deferred to future engine-level pass.
- Architecture thresholds: oversized_module >= 30 symbols OR >= 1000 lines (warning), monolithic_export_surface >= 20 exported symbols (info), anemic_module == 1 symbol excl. entry files (info), hub_module_bidirectional >= 5 intra-project imports (info), barrel_file_reexport >= 5 re-exports (warning), deep_import_chain >= 4 path depth (info), excessive_public_api >= 10 symbols AND > 80% exported (info), leaky_abstraction_boundary = exported types with public fields (warning).
