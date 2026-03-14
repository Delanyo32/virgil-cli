# virgil-cli

A fast Rust CLI that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases into structured Parquet files and queries the results with DuckDB. Built with [tree-sitter](https://tree-sitter.github.io/) for accurate parsing, [Apache Arrow](https://arrow.apache.org/) for efficient columnar output, and [DuckDB](https://duckdb.org/) for SQL-powered querying.

## Installation

```bash
cargo install --path .
```

## AI Skill

Virgil includes an [Agent Skills](https://agentskills.io/) compatible skill that teaches AI assistants how to use virgil-cli for codebase exploration. Works with Claude Code, Cursor, Gemini CLI, VS Code, and other compatible agents.

### Install via npx

```bash
npx skills add delanyo32/virgil-cli
```

### Manual install (Claude Code)

Copy the `virgil/` skill directory to your skills folder:

```bash
cp -r .agents/skills/virgil ~/.claude/skills/
```

### What the skill provides

- **Core workflow**: Parse → Overview → Drill-down exploration pattern
- **6 strategic playbooks**: Architecture understanding, symbol tracing, onboarding, bug investigation, dependency mapping, API surface mapping
- **Full command reference**: Flag-by-flag docs for all 13 commands
- **30+ SQL recipes**: Reusable DuckDB queries for file, symbol, dependency, and comment analysis

## Usage

```bash
virgil projects <COMMAND> [OPTIONS]
```

### Global Options

| Option | Description |
|--------|-------------|
| `--env` | Use S3 storage — reads credentials from environment variables (see [S3 Storage](#s3-storage)) |

### Subcommands

All commands are nested under `virgil projects`:

| Command | Description |
|---------|-------------|
| `projects parse` | Parse a codebase and output parquet files |
| `projects overview` | Show codebase overview (language breakdown, top symbols, directories, dependency summary) |
| `projects search` | Search for symbols by name (fuzzy match) |
| `projects outline` | Show all symbols in a file |
| `projects files` | List parsed files |
| `projects read` | Read source file content |
| `projects query` | Execute raw SQL against parquet files |
| `projects deps` | Show what a file imports (dependencies) |
| `projects dependents` | Show what files import a given file (reverse dependencies) |
| `projects callers` | Find which files import a specific symbol |
| `projects imports` | List all imports with filters |
| `projects comments` | List comments with filters |

### `parse`

```bash
virgil projects parse <DIR> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<DIR>` | Root directory to parse | required |
| `-o`, `--output` | Output directory for parquet files | `.` |
| `-l`, `--language` | Comma-separated language filter (ts,tsx,js,jsx,c,h,cpp,cc,cxx,hpp,hxx,hh,cs,rs,py,pyi,go,java,php) | all supported |

### `overview`

```bash
virgil projects overview [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### `search`

```bash
virgil projects search <QUERY> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<QUERY>` | Search query (fuzzy match) | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--kind` | Filter by symbol kind | all |
| `--exported` | Only show exported symbols | false |
| `--limit` | Maximum results to return | `20` |
| `--offset` | Number of results to skip | `0` |
| `--format` | Output format (table, json, csv) | `table` |

### `outline`

```bash
virgil projects outline <FILE_PATH> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<FILE_PATH>` | File path to get outline for | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### `files`

```bash
virgil projects files [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--data-dir` | Directory containing parquet files | `.` |
| `--language` | Filter by language | all |
| `--directory` | Filter by directory prefix | none |
| `--limit` | Maximum results to return | `100` |
| `--offset` | Number of results to skip | `0` |
| `--sort` | Sort by field (path, lines, size, imports, dependents) | `path` |
| `--format` | Output format (table, json, csv) | `table` |

### `read`

```bash
virgil projects read <FILE_PATH> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<FILE_PATH>` | File path to read (relative, as stored in parquet) | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--root` | Root directory of the source project | `.` |
| `--start-line` | Start line (1-indexed) | beginning |
| `--end-line` | End line (1-indexed, inclusive) | end of file |

### `query`

```bash
virgil projects query <SQL> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<SQL>` | SQL query to execute | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### `deps`

```bash
virgil projects deps <FILE_PATH> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<FILE_PATH>` | File path to show dependencies for | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### `dependents`

```bash
virgil projects dependents <FILE_PATH> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<FILE_PATH>` | File path to find dependents for | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### `callers`

```bash
virgil projects callers <SYMBOL_NAME> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<SYMBOL_NAME>` | Symbol name to search for (fuzzy match) | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--limit` | Maximum results to return | `50` |
| `--format` | Output format (table, json, csv) | `table` |

### `imports`

```bash
virgil projects imports [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--data-dir` | Directory containing parquet files | `.` |
| `--module` | Filter by module specifier (fuzzy match) | none |
| `--kind` | Filter by import kind (static, dynamic, require, re_export, include, using, use, import, from) | all |
| `--file` | Filter by source file prefix | none |
| `--type-only` | Only show type-only imports | false |
| `--external` | Only show external (library) imports | false |
| `--internal` | Only show internal (user code) imports | false |
| `--limit` | Maximum results to return | `50` |
| `--format` | Output format (table, json, csv) | `table` |

### `comments`

```bash
virgil projects comments [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--data-dir` | Directory containing parquet files | `.` |
| `--file` | Filter by file path prefix | none |
| `--kind` | Filter by comment kind (line, block, doc) | all |
| `--documented` | Only show comments associated with a symbol | false |
| `--symbol` | Filter by associated symbol name (fuzzy match) | none |
| `--limit` | Maximum results to return | `50` |
| `--format` | Output format (table, json, csv) | `table` |

### Examples

```bash
# Parse an entire project
virgil projects parse ./my-app

# Output to a specific directory
virgil projects parse ./my-app --output ./data

# Only parse TypeScript files
virgil projects parse ./my-app --language ts,tsx

# Parse a C/C++ project
virgil projects parse ./my-lib --language c,h,cpp,hpp

# Parse a C# project
virgil projects parse ./my-project --language cs

# Parse a Java project
virgil projects parse ./my-project --language java

# Parse a PHP project
virgil projects parse ./my-project --language php

# Show codebase overview
virgil projects overview --data-dir ./data

# Search for symbols matching "handleClick"
virgil projects search handleClick --data-dir ./data

# Search for exported functions only
virgil projects search handler --kind function --exported --data-dir ./data

# Show all symbols in a specific file
virgil projects outline src/components/App.tsx --data-dir ./data

# List all TypeScript files
virgil projects files --language typescript --data-dir ./data

# List files in a specific directory
virgil projects files --directory src/components --data-dir ./data

# Read a source file
virgil projects read src/index.ts --data-dir ./data --root ./my-app

# Read specific lines from a file
virgil projects read src/index.ts --start-line 10 --end-line 50 --data-dir ./data --root ./my-app

# Show what a file imports
virgil projects deps src/app.ts --data-dir ./data

# Show what files import a given module
virgil projects dependents src/utils/api.ts --data-dir ./data

# Find which files import a specific symbol
virgil projects callers useState --data-dir ./data

# List all imports from a specific module
virgil projects imports --module react --data-dir ./data

# List re-exports only
virgil projects imports --kind re_export --data-dir ./data

# List only external (library) imports
virgil projects imports --external --data-dir ./data

# List only internal (user code) imports
virgil projects imports --internal --data-dir ./data

# List C/C++ #include directives
virgil projects imports --kind include --data-dir ./data

# List C# using directives
virgil projects imports --kind using --data-dir ./data

# List Java imports
virgil projects imports --kind import --file .java --data-dir ./data

# List PHP use statements
virgil projects imports --kind use --file .php --data-dir ./data

# Sort files by number of dependents
virgil projects files --sort dependents --data-dir ./data

# List all doc comments
virgil projects comments --kind doc --data-dir ./data

# List comments associated with symbols (documentation)
virgil projects comments --documented --data-dir ./data

# Find comments mentioning a specific symbol
virgil projects comments --symbol handleClick --data-dir ./data

# List comments in a specific file
virgil projects comments --file src/utils --data-dir ./data

# Run a raw SQL query (tables: files, symbols, imports, comments)
virgil projects query "SELECT name, kind FROM symbols WHERE is_exported = true" --data-dir ./data

# Get output as JSON
virgil projects search handleClick --data-dir ./data --format json
```

## Output Formats

Most subcommands support three output formats via `--format`:

| Format | Description |
|--------|-------------|
| `table` | Human-readable table (default) |
| `json` | JSON for programmatic use |
| `csv` | CSV for spreadsheets and pipelines |

## Output

Four Parquet files are generated:

### files.parquet

| Column | Type | Description |
|--------|------|-------------|
| path | Utf8 | Relative path from project root |
| name | Utf8 | File name |
| extension | Utf8 | Extension without dot |
| language | Utf8 | typescript / tsx / javascript / jsx / c / cpp / csharp / rust / python / go / java / php |
| size_bytes | UInt64 | File size in bytes |
| line_count | UInt64 | Number of lines |

### symbols.parquet

| Column | Type | Description |
|--------|------|-------------|
| name | Utf8 | Symbol name |
| kind | Utf8 | Symbol kind (see below) |
| file_path | Utf8 | Relative file path |
| start_line | UInt32 | 0-based start line |
| start_column | UInt32 | 0-based start column |
| end_line | UInt32 | 0-based end line |
| end_column | UInt32 | 0-based end column |
| is_exported | Boolean | Whether the symbol is exported |

### imports.parquet

| Column | Type | Description |
|--------|------|-------------|
| source_file | Utf8 | File containing the import |
| module_specifier | Utf8 | Import path (e.g., `react`, `./utils/api`) |
| imported_name | Utf8 | Imported symbol name (or `*` for namespace imports) |
| local_name | Utf8 | Local binding name |
| kind | Utf8 | Import kind (static, dynamic, require, re_export, include, using, use, import, from) |
| is_type_only | Boolean | Whether the import is type-only |
| line | UInt32 | Line number of the import |
| is_external | Boolean | Whether the import is from an external library (true) or user code (false) |

### comments.parquet

| Column | Type | Description |
|--------|------|-------------|
| file_path | Utf8 | Relative file path |
| text | Utf8 | Raw comment text including delimiters |
| kind | Utf8 | Comment kind (line, block, doc) |
| start_line | UInt32 | 0-based start line |
| start_column | UInt32 | 0-based start column |
| end_line | UInt32 | 0-based end line |
| end_column | UInt32 | 0-based end column |
| associated_symbol | Utf8 (nullable) | Name of the symbol this comment documents |
| associated_symbol_kind | Utf8 (nullable) | Kind of the associated symbol |

### Symbol Kinds

`function`, `class`, `method`, `variable`, `interface`, `type_alias`, `enum`, `arrow_function`, `struct`, `union`, `namespace`, `macro`, `property`, `typedef`, `trait`, `constant`, `module`

### Comment Kinds

`line` (`//`), `block` (`/* */`), `doc` (`/** */`)

## Supported Languages

| Language | Extensions |
|----------|------------|
| TypeScript | `.ts` |
| TSX | `.tsx` |
| JavaScript | `.js` |
| JSX | `.jsx` |
| C | `.c`, `.h` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh` |
| C# | `.cs` |
| Rust | `.rs` |
| Python | `.py`, `.pyi` |
| Go | `.go` |
| Java | `.java` |
| PHP | `.php` |

## Features

- **Multi-language** — TypeScript, JavaScript, C, C++, C#, Rust, Python, Go, Java, and PHP with language-specific parsers
- **Fast** — parallel file processing with rayon
- **Accurate** — tree-sitter parsing (same parsers used by editors like Neovim and Zed)
- **Gitignore-aware** — automatically skips `node_modules`, `dist`, `build`, and anything in `.gitignore`
- **Export detection** — tracks whether symbols are exported (ES exports, C linkage, C#/Java access modifiers, Rust visibility, Go capitalization, Python underscore convention, PHP visibility)
- **Arrow function support** — distinguishes arrow functions from regular variables
- **Import tracking** — full import graph with kind, type-only, re-export detection, and external/internal classification
- **DuckDB-powered querying** — run raw SQL against parsed parquet data
- **Fuzzy symbol search** — find symbols by approximate name match, ranked by usage count
- **Dependency navigation** — explore imports, dependents, and callers across the codebase
- **Rich overview** — hub files, popular symbols, import kind distribution, barrel file detection
- **File reading with line ranges** — read source files or specific line ranges directly from the CLI
- **Comment tracking** — extracts comments with classification (line/block/doc) and automatic symbol association
- **Multiple output formats** — table, JSON, and CSV output for all query commands
- **S3 storage** — parse codebases from S3, write parquet output to S3, and query parquet files stored in S3

## S3 Storage

All commands support reading from and writing to S3-compatible storage (AWS S3, MinIO, Cloudflare R2, etc.) via the `--env` global flag. When `--env` is set, path arguments (`dir`, `--output`, `--data-dir`, `--root`) are reinterpreted as S3 key prefixes.

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `S3_ACCESS_KEY_ID` | Yes | S3 access key ID |
| `S3_SECRET_ACCESS_KEY` | Yes | S3 secret access key |
| `S3_BUCKET_NAME` | Yes | S3 bucket name |
| `S3_ENDPOINT` | Yes | S3 endpoint URL (e.g., `https://s3.amazonaws.com`, `http://localhost:9000` for MinIO, or `https://<account_id>.r2.cloudflarestorage.com` for Cloudflare R2) |
| `S3_REGION` | No | S3 region (default: `us-east-1`) |

### S3 Examples

```bash
# Parse files from S3 and write parquet to S3
virgil projects parse my-prefix/src --output parsed --env

# Query parquet files stored in S3
virgil projects search "main" --data-dir parsed --env
virgil projects overview --data-dir parsed --env

# Read a source file from S3
virgil projects read src/main.ts --root my-prefix/src --env

# Run raw SQL against S3-hosted parquet
virgil projects query "SELECT * FROM symbols LIMIT 10" --data-dir parsed --env
```

## Inspecting Output

```python
import pyarrow.parquet as pq

files = pq.read_table("files.parquet").to_pandas()
symbols = pq.read_table("symbols.parquet").to_pandas()
imports = pq.read_table("imports.parquet").to_pandas()
comments = pq.read_table("comments.parquet").to_pandas()

print(files)
print(symbols)
print(imports)
print(comments)
```

## License

MIT
