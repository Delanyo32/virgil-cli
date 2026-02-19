# virgil-cli

A fast Rust CLI that parses TypeScript/JavaScript codebases into structured Parquet files and queries the results with DuckDB. Built with [tree-sitter](https://tree-sitter.github.io/) for accurate parsing, [Apache Arrow](https://arrow.apache.org/) for efficient columnar output, and [DuckDB](https://duckdb.org/) for SQL-powered querying.

## Installation

```bash
cargo install --path .
```

## Usage

```bash
virgil <COMMAND> [OPTIONS]
```

### Subcommands

| Command | Description |
|---------|-------------|
| `parse` | Parse a codebase and output parquet files |
| `overview` | Show codebase overview (language breakdown, top symbols, directories) |
| `search` | Search for symbols by name (fuzzy match) |
| `outline` | Show all symbols in a file |
| `files` | List parsed files |
| `read` | Read source file content |
| `query` | Execute raw SQL against parquet files |

### `parse`

```bash
virgil parse <DIR> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<DIR>` | Root directory to parse | required |
| `-o`, `--output` | Output directory for parquet files | `.` |
| `-l`, `--language` | Comma-separated language filter (ts,tsx,js,jsx) | all supported |

### `overview`

```bash
virgil overview [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### `search`

```bash
virgil search <QUERY> [OPTIONS]
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
virgil outline <FILE_PATH> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<FILE_PATH>` | File path to get outline for | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### `files`

```bash
virgil files [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--data-dir` | Directory containing parquet files | `.` |
| `--language` | Filter by language | all |
| `--directory` | Filter by directory prefix | none |
| `--limit` | Maximum results to return | `100` |
| `--offset` | Number of results to skip | `0` |
| `--format` | Output format (table, json, csv) | `table` |

### `read`

```bash
virgil read <FILE_PATH> [OPTIONS]
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
virgil query <SQL> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<SQL>` | SQL query to execute | required |
| `--data-dir` | Directory containing parquet files | `.` |
| `--format` | Output format (table, json, csv) | `table` |

### Examples

```bash
# Parse an entire project
virgil parse ./my-app

# Output to a specific directory
virgil parse ./my-app --output ./data

# Only parse TypeScript files
virgil parse ./my-app --language ts,tsx

# Show codebase overview
virgil overview --data-dir ./data

# Search for symbols matching "handleClick"
virgil search handleClick --data-dir ./data

# Search for exported functions only
virgil search handler --kind function --exported --data-dir ./data

# Show all symbols in a specific file
virgil outline src/components/App.tsx --data-dir ./data

# List all TypeScript files
virgil files --language typescript --data-dir ./data

# List files in a specific directory
virgil files --directory src/components --data-dir ./data

# Read a source file
virgil read src/index.ts --data-dir ./data --root ./my-app

# Read specific lines from a file
virgil read src/index.ts --start-line 10 --end-line 50 --data-dir ./data --root ./my-app

# Run a raw SQL query (tables: files, symbols)
virgil query "SELECT name, kind FROM symbols WHERE is_exported = true" --data-dir ./data

# Get output as JSON
virgil search handleClick --data-dir ./data --format json
```

## Output Formats

Most subcommands support three output formats via `--format`:

| Format | Description |
|--------|-------------|
| `table` | Human-readable table (default) |
| `json` | JSON for programmatic use |
| `csv` | CSV for spreadsheets and pipelines |

## Output

Two Parquet files are generated:

### files.parquet

| Column | Type | Description |
|--------|------|-------------|
| path | Utf8 | Relative path from project root |
| name | Utf8 | File name |
| extension | Utf8 | Extension without dot |
| language | Utf8 | typescript / tsx / javascript / jsx |
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

### Symbol Kinds

`function`, `class`, `method`, `variable`, `interface`, `type_alias`, `enum`, `arrow_function`

## Supported Languages

| Language | Extensions |
|----------|------------|
| TypeScript | `.ts` |
| TSX | `.tsx` |
| JavaScript | `.js` |
| JSX | `.jsx` |

## Features

- **Fast** — parallel file processing with rayon
- **Accurate** — tree-sitter parsing (same parsers used by editors like Neovim and Zed)
- **Gitignore-aware** — automatically skips `node_modules`, `dist`, `build`, and anything in `.gitignore`
- **Export detection** — tracks whether symbols are exported
- **Arrow function support** — distinguishes arrow functions from regular variables
- **DuckDB-powered querying** — run raw SQL against parsed parquet data
- **Fuzzy symbol search** — find symbols by approximate name match
- **File reading with line ranges** — read source files or specific line ranges directly from the CLI
- **Multiple output formats** — table, JSON, and CSV output for all query commands

## Inspecting Output

```python
import pyarrow.parquet as pq

files = pq.read_table("files.parquet").to_pandas()
symbols = pq.read_table("symbols.parquet").to_pandas()

print(files)
print(symbols)
```

## License

MIT
