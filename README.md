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
- **Full command reference**: Flag-by-flag docs for all project commands
- **30+ SQL recipes**: Reusable DuckDB queries for file, symbol, dependency, and comment analysis

## Usage

```bash
virgil-cli <COMMAND> [OPTIONS]
```

### Global Options

| Option | Description |
|--------|-------------|
| `--env` | Use S3 storage — reads credentials from environment variables (see [S3 Storage](#s3-storage)) |

### Subcommands

| Command | Description |
|---------|-------------|
| `project` | Manage persistent projects (create, list, delete, query) |
| `audit` | Run code audits with complexity, quality, and security analysis (create, list, delete, complexity, overview, quality, security) |

### `project`

Manage persistent projects stored in `~/.virgil/`. Parse once, query by name — no need to track `--data-dir` paths manually.

```bash
virgil-cli project <SUBCOMMAND> [OPTIONS]
```

#### `project create`

```bash
virgil-cli project create <DIR> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<DIR>` | Directory to parse | required |
| `-n`, `--name` | Custom project name | directory basename |
| `-l`, `--language` | Comma-separated language filter | all supported |

#### `project list`

```bash
virgil-cli project list
```

Lists all registered projects with name, repo path, and creation timestamp.

#### `project delete`

```bash
virgil-cli project delete <NAME>
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Project name to delete | required |

#### `project query`

```bash
virgil-cli project query <NAME> <SUBCOMMAND> [ARGS...]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Project name | required |
| `<SUBCOMMAND>` | Any query subcommand (overview, search, files, read, etc.) | required |
| `[ARGS...]` | Arguments forwarded to the subcommand | none |

Auto-injects `--data-dir` pointing to the project's stored parquet data. For `read`, also auto-injects `--root` pointing to the original repo path.

### Examples

```bash
# Register a project (parse once, query by name)
virgil-cli project create ./my-app
virgil-cli project create ./my-app --name my-app-v2

# Only parse TypeScript files
virgil-cli project create ./my-app --language ts,tsx

# List registered projects
virgil-cli project list

# Query a project by name
virgil-cli project query my-app overview
virgil-cli project query my-app search "handleClick" --kind function
virgil-cli project query my-app file list --language typescript
virgil-cli project query my-app file read src/index.ts --start-line 1 --end-line 20
virgil-cli project query my-app file get src/components/App.tsx
virgil-cli project query my-app symbol get handleClick
virgil-cli project query my-app comments list --kind doc
virgil-cli project query my-app comments search "TODO"

# Delete a project
virgil-cli project delete my-app
```

### `audit`

Run code audits with complexity, quality, and security analysis. Parses a codebase, computes cyclomatic complexity, cognitive complexity, and function/method line counts per symbol, and stores the results in `~/.virgil/audits/`. Quality analysis includes dead code detection, coupling/cohesion metrics, and structural duplication. Security analysis detects unsafe function calls, inline SQL/HTML string risks, and hardcoded secrets.

```bash
virgil-cli audit <SUBCOMMAND> [OPTIONS]
```

#### `audit create`

```bash
virgil-cli audit create <DIR> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<DIR>` | Directory to parse | required |
| `-n`, `--name` | Custom audit name | directory basename |
| `-l`, `--language` | Comma-separated language filter | all supported |

#### `audit list`

```bash
virgil-cli audit list
```

Lists all registered audits with name, repo path, and creation timestamp.

#### `audit delete`

```bash
virgil-cli audit delete <NAME>
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name to delete | required |

#### `audit complexity`

```bash
virgil-cli audit complexity <NAME> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--file` | Filter by file path prefix | none |
| `--kind` | Filter by symbol kind (function, method, arrow_function) | none |
| `--sort` | Sort by field (cyclomatic, cognitive, name, file, lines) | cyclomatic |
| `--limit` | Maximum results to return | 20 |
| `--threshold` | Only show symbols with cyclomatic complexity >= threshold | none |
| `--format` | Output format (table, json, csv) | table |

#### `audit overview`

```bash
virgil-cli audit overview <NAME> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--format` | Output format (table, json, csv) | table |

Shows combined complexity + quality + security overview: summary stats (avg/max cyclomatic, cognitive, and line count), cyclomatic distribution buckets, top 10 most complex symbols, per-file complexity rankings, dead code summary, coupling summary, duplication summary, and security issue counts.

#### `audit quality dead-code`

Find exported symbols with no internal imports (potential dead code).

```bash
virgil-cli audit quality <NAME> dead-code [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--file` | Filter by file path prefix | none |
| `--kind` | Filter by symbol kind | none |
| `--limit` | Maximum results to return | 50 |
| `--format` | Output format (table, json, csv) | table |

#### `audit quality coupling`

Analyze file coupling (fan-in/fan-out) and detect circular dependencies.

```bash
virgil-cli audit quality <NAME> coupling [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--file` | Filter by file path prefix | none |
| `--sort` | Sort by field (instability, fan-in, fan-out, file) | instability |
| `--limit` | Maximum results to return | 20 |
| `--cycles` | Show circular dependencies (Tarjan's SCC) | false |
| `--format` | Output format (table, json, csv) | table |

#### `audit quality duplication`

Find structurally similar functions (DRY violations) using AST structural hashing.

```bash
virgil-cli audit quality <NAME> duplication [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--file` | Filter by file path prefix | none |
| `--min-group` | Minimum group size | 2 |
| `--limit` | Maximum results to return | 20 |
| `--format` | Output format (table, json, csv) | table |

#### `audit security unsafe-calls`

Find calls to dangerous functions (eval, exec, system, etc.) across all supported languages.

```bash
virgil-cli audit security <NAME> unsafe-calls [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--file` | Filter by file path prefix | none |
| `--limit` | Maximum results to return | 50 |
| `--format` | Output format (table, json, csv) | table |

#### `audit security string-risks`

Find string literals containing inline SQL or HTML patterns (injection vectors).

```bash
virgil-cli audit security <NAME> string-risks [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--file` | Filter by file path prefix | none |
| `--limit` | Maximum results to return | 50 |
| `--format` | Output format (table, json, csv) | table |

#### `audit security hardcoded-secrets`

Find variables with secret-like names (api_key, password, token, etc.) assigned to string literals.

```bash
virgil-cli audit security <NAME> hardcoded-secrets [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Audit name | required |
| `--file` | Filter by file path prefix | none |
| `--limit` | Maximum results to return | 50 |
| `--format` | Output format (table, json, csv) | table |

#### Audit Examples

```bash
# Create an audit
virgil-cli audit create ./my-app
virgil-cli audit create ./my-app --name my-app-audit --language rs

# View complexity overview
virgil-cli audit overview my-app

# List most complex functions
virgil-cli audit complexity my-app --sort cyclomatic --limit 10

# Find complex functions in a specific file
virgil-cli audit complexity my-app --file src/main.rs

# Find functions exceeding a complexity threshold
virgil-cli audit complexity my-app --threshold 10

# Sort by longest functions
virgil-cli audit complexity my-app --sort lines

# Export as JSON
virgil-cli audit complexity my-app --format json

# Find potential dead code
virgil-cli audit quality my-app dead-code
virgil-cli audit quality my-app dead-code --kind function --file src/

# Analyze file coupling
virgil-cli audit quality my-app coupling --sort fan-out
virgil-cli audit quality my-app coupling --cycles

# Find duplicate function structures
virgil-cli audit quality my-app duplication
virgil-cli audit quality my-app duplication --min-group 3

# Find security issues
virgil-cli audit security my-app unsafe-calls
virgil-cli audit security my-app string-risks --file src/
virgil-cli audit security my-app hardcoded-secrets --format json

# Delete an audit
virgil-cli audit delete my-app
```

## Output Formats

Most subcommands support three output formats via `--format`:

| Format | Description |
|--------|-------------|
| `table` | Human-readable table (default) |
| `json` | JSON for programmatic use |
| `csv` | CSV for spreadsheets and pipelines |

## Output

Parquet files are generated per command. `project create` produces four files (files, symbols, imports, comments). `audit create` produces those same four plus `complexity.parquet` and `security.parquet`.

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

### complexity.parquet

Generated by `audit create` only. Contains per-symbol complexity metrics for functions, methods, and arrow functions.

| Column | Type | Description |
|--------|------|-------------|
| file_path | Utf8 | Relative file path |
| symbol_name | Utf8 | Function/method name |
| symbol_kind | Utf8 | Symbol kind (function, method, arrow_function) |
| start_line | UInt32 | 0-based start line |
| end_line | UInt32 | 0-based end line |
| line_count | UInt32 | Number of lines in the function/method body |
| cyclomatic_complexity | UInt32 | Cyclomatic complexity (base 1, +1 per decision point) |
| cognitive_complexity | UInt32 | Cognitive complexity (Sonar-style, nesting-weighted) |
| structural_hash | UInt64 | Hash of AST node-kind sequence (used for duplication detection) |

### security.parquet

Generated by `audit create` only. Contains security issues detected via AST traversal.

| Column | Type | Description |
|--------|------|-------------|
| file_path | Utf8 | Relative file path |
| issue_type | Utf8 | Issue type (unsafe_call, string_risk, hardcoded_secret) |
| severity | Utf8 | Severity level (high, medium) |
| line | UInt32 | 0-based start line |
| column | UInt32 | 0-based start column |
| end_line | UInt32 | 0-based end line |
| end_column | UInt32 | 0-based end column |
| description | Utf8 | Human-readable description of the issue |
| snippet | Utf8 | Offending source text (truncated to 200 chars) |
| symbol_name | Utf8 | Enclosing function/method name (empty if top-level) |

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
- **Project management** — register codebases as named projects, query by name without tracking paths
- **Complexity analysis** — cyclomatic complexity, cognitive complexity (Sonar-style), and function line counts per symbol across all 12 languages
- **Audit management** — register audits with complexity metrics, query with filters, thresholds, and sorting
- **Quality analysis** — dead code detection, file coupling/cohesion (fan-in/fan-out/instability), circular dependency detection via Tarjan's SCC, and structural duplication via AST hashing
- **Security analysis** — detects unsafe function calls (eval, exec, system, etc.), inline SQL/HTML string risks, and hardcoded secrets across all 12 languages

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
