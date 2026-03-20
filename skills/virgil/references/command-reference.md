# Virgil Command Reference

Complete flag-by-flag documentation for all 13 virgil commands.

## parse

Parse a codebase and output Parquet files.

```bash
virgil projects parse <DIR> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<DIR>` | path | required | Root directory to parse |
| `-o`, `--output` | path | `.` | Output directory for Parquet files |
| `-l`, `--language` | string | all | Comma-separated language filter |

**Language filter values:** `ts`, `tsx`, `js`, `jsx`, `c`, `h`, `cpp`, `cc`, `cxx`, `hpp`, `hxx`, `hh`, `cs`, `rs`, `py`, `pyi`, `go`, `java`, `php`

**Output files:** `files.parquet`, `symbols.parquet`, `imports.parquet`, `comments.parquet`, `errors.parquet`

**Examples:**

```bash
virgil projects parse ./my-app --output ./data
virgil projects parse ./my-app --language ts,tsx
virgil projects parse ./my-lib --language c,h,cpp,hpp
```

## overview

Show codebase overview: language breakdown, top symbols, directory structure, hub files, dependency summary.

```bash
virgil projects overview [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--format` | table\|json\|csv | `table` | Output format |
| `--depth` | integer | `3` | Maximum directory depth for module tree |

**Examples:**

```bash
virgil projects overview --data-dir ./data --format json
virgil projects overview --depth 5
```

## search

Search for symbols by name using fuzzy matching, ranked by usage count.

```bash
virgil projects search <QUERY> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<QUERY>` | string | required | Search query (fuzzy match) |
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--kind` | string | all | Filter by symbol kind |
| `--exported` | flag | false | Only show exported symbols |
| `--limit` | integer | `20` | Maximum results to return |
| `--offset` | integer | `0` | Number of results to skip |
| `--format` | table\|json\|csv | `table` | Output format |

**Symbol kind values:** `function`, `class`, `method`, `variable`, `interface`, `type_alias`, `enum`, `arrow_function`, `struct`, `union`, `namespace`, `macro`, `property`, `typedef`, `trait`, `constant`, `module`

**Output columns:** name, kind, file_path, line, exported

**Examples:**

```bash
virgil projects search handleClick --data-dir ./data --format json
virgil projects search handler --kind function --exported
virgil projects search User --kind class --limit 10
```

## outline

Show all symbols in a specific file, ordered by position.

```bash
virgil projects outline <FILE_PATH> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<FILE_PATH>` | string | required | File path (relative, as in Parquet) |
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--format` | table\|json\|csv | `table` | Output format |

**Output columns:** name, kind, start_line, end_line, exported

**Examples:**

```bash
virgil projects outline src/components/App.tsx --data-dir ./data --format json
virgil projects outline src/main.rs
```

## files

List parsed files with filters and sorting.

```bash
virgil projects files [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--language` | string | all | Filter by language |
| `--directory` | string | none | Filter by directory prefix |
| `--limit` | integer | `100` | Maximum results to return |
| `--offset` | integer | `0` | Number of results to skip |
| `--sort` | field | `path` | Sort by field |
| `--format` | table\|json\|csv | `table` | Output format |

**Sort field values:** `path`, `lines`, `size`, `imports`, `dependents`

**Output columns:** path, language, lines, size, imports_count, dependents_count

**Examples:**

```bash
virgil projects files --language typescript --data-dir ./data --format json
virgil projects files --directory src/components --sort dependents
virgil projects files --sort lines --limit 20
```

## read

Read source file content, optionally with line ranges. Resolves the relative path stored in Parquet against the source root.

```bash
virgil projects read <FILE_PATH> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<FILE_PATH>` | string | required | File path (relative, as in Parquet) |
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--root` | path | `.` | Root directory of the source project |
| `--start-line` | integer | beginning | Start line (1-indexed) |
| `--end-line` | integer | end of file | End line (1-indexed, inclusive) |

**Examples:**

```bash
virgil projects read src/index.ts --data-dir ./data --root ./my-app
virgil projects read src/index.ts --start-line 10 --end-line 50 --root ./my-app
```

## query

Execute raw DuckDB SQL against the Parquet files. Available tables: `files`, `symbols`, `imports`, `comments`, `errors`.

```bash
virgil projects query <SQL> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<SQL>` | string | required | SQL query to execute |
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--format` | table\|json\|csv | `table` | Output format |

**Examples:**

```bash
virgil projects query "SELECT name, kind FROM symbols WHERE is_exported = true" --data-dir ./data
virgil projects query "SELECT COUNT(*) FROM files" --format json
```

## deps

Show what a file imports (its dependencies).

```bash
virgil projects deps <FILE_PATH> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<FILE_PATH>` | string | required | File path to show dependencies for |
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--format` | table\|json\|csv | `table` | Output format |

**Output columns:** module_specifier, imported_name, kind, is_external

**Examples:**

```bash
virgil projects deps src/app.ts --data-dir ./data --format json
virgil projects deps src/main.rs
```

## dependents

Show what files import a given file (reverse dependencies).

```bash
virgil projects dependents <FILE_PATH> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<FILE_PATH>` | string | required | File path to find dependents for |
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--format` | table\|json\|csv | `table` | Output format |

**Output columns:** source_file, imported_name, kind

**Examples:**

```bash
virgil projects dependents src/utils/api.ts --data-dir ./data --format json
virgil projects dependents src/lib.rs
```

## callers

Find which files import a specific symbol (fuzzy match on symbol name).

```bash
virgil projects callers <SYMBOL_NAME> [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `<SYMBOL_NAME>` | string | required | Symbol name to search for (fuzzy match) |
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--limit` | integer | `50` | Maximum results to return |
| `--format` | table\|json\|csv | `table` | Output format |

**Output columns:** source_file, imported_name, module_specifier, kind

**Examples:**

```bash
virgil projects callers useState --data-dir ./data --format json
virgil projects callers handleClick --limit 100
```

## imports

List all imports with filters for module, kind, file, type-only, and external/internal classification.

```bash
virgil projects imports [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--module` | string | none | Filter by module specifier (fuzzy match) |
| `--kind` | string | all | Filter by import kind |
| `--file` | string | none | Filter by source file prefix |
| `--type-only` | flag | false | Only show type-only imports |
| `--external` | flag | false | Only show external (library) imports |
| `--internal` | flag | false | Only show internal (user code) imports |
| `--limit` | integer | `50` | Maximum results to return |
| `--format` | table\|json\|csv | `table` | Output format |

**Import kind values:** `static`, `dynamic`, `require`, `re_export`, `include`, `using`, `use`, `import`, `from`

**Output columns:** source_file, module_specifier, imported_name, local_name, kind, is_type_only, is_external

**Examples:**

```bash
virgil projects imports --module react --data-dir ./data --format json
virgil projects imports --kind re_export
virgil projects imports --external --file src/
virgil projects imports --kind include  # C/C++ #include
virgil projects imports --kind using    # C# using
virgil projects imports --kind use --file .php  # PHP use
```

## errors

List parse errors with optional filters for error type and language.

```bash
virgil projects errors [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--error-type` | string | all | Filter by error type |
| `--language` | string | all | Filter by language |
| `--limit` | integer | `50` | Maximum results to return |
| `--format` | table\|json\|csv | `table` | Output format |

**Error type values:** `parser_creation`, `file_read`, `parse_failure`

**Output columns:** file_path, file_name, language, error_type, error_message, size_bytes

**Examples:**

```bash
virgil projects errors --data-dir ./data --format json
virgil projects errors --error-type parse_failure
virgil projects errors --language typescript --limit 10
```

## comments

List comments with filters for file, kind, documented symbols, and symbol name.

```bash
virgil projects comments [OPTIONS]
```

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--data-dir` | path | `.` | Directory containing Parquet files |
| `--file` | string | none | Filter by file path prefix |
| `--kind` | string | all | Filter by comment kind |
| `--documented` | flag | false | Only show comments associated with a symbol |
| `--symbol` | string | none | Filter by associated symbol name (fuzzy match) |
| `--limit` | integer | `50` | Maximum results to return |
| `--format` | table\|json\|csv | `table` | Output format |

**Comment kind values:** `line`, `block`, `doc`

**Output columns:** file_path, text, kind, start_line, associated_symbol, associated_symbol_kind

**Examples:**

```bash
virgil projects comments --kind doc --data-dir ./data --format json
virgil projects comments --documented
virgil projects comments --symbol handleClick
virgil projects comments --file src/utils
```
