# virgil-cli

A fast Rust CLI that parses TypeScript/JavaScript codebases and outputs structured data as Parquet files. Built with [tree-sitter](https://tree-sitter.github.io/) for accurate parsing and [Apache Arrow](https://arrow.apache.org/) for efficient columnar output.

## Installation

```bash
cargo install --path .
```

## Usage

```bash
virgil-cli <DIR> [--output <dir>] [--language <filter>]
```

### Arguments

| Argument | Description | Default |
|----------|-------------|---------|
| `<DIR>` | Root directory to parse | required |
| `--output`, `-o` | Output directory for parquet files | `.` |
| `--language`, `-l` | Comma-separated language filter | all supported |

### Examples

```bash
# Parse an entire project
virgil-cli ./my-app

# Output to a specific directory
virgil-cli ./my-app --output ./data

# Only parse TypeScript files
virgil-cli ./my-app --language ts,tsx
```

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
