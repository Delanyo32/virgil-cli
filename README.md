# virgil-cli

A fast Rust CLI that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases on-demand with [tree-sitter](https://tree-sitter.github.io/), materialises them into a [CozoDB](https://www.cozodb.org/) fact store, and answers queries via Cozoscript. Persistent on-disk cache with warm-start in tens of milliseconds. Supports S3-compatible storage (AWS S3, Cloudflare R2, MinIO) for querying remote codebases directly.

## Installation

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Delanyo32/virgil-cli/master/install.sh | sh
```

### Windows

```powershell
irm https://raw.githubusercontent.com/Delanyo32/virgil-cli/master/install.ps1 | iex
```

### From source (requires Rust)

```bash
cargo install --path .
```

## Usage

Two top-level command groups:

```bash
virgil-cli projects <COMMAND>
virgil-cli serve --s3 <URI> [OPTIONS]
```

## Projects

All project commands are nested under `virgil-cli projects`:

| Command | Description |
|---------|-------------|
| `create` | Register a project for querying (scans files, saves to `~/.virgil-cli/projects.json`) |
| `list` | List registered projects with file counts |
| `delete` | Remove a registered project |
| `query` | Run a Cozoscript template, file, or inline query against the project's fact store |

### `projects create`

```bash
virgil-cli projects create <NAME> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Project name | required |
| `-p`, `--path` | Root directory of the project | `.` |
| `-e`, `--exclude` | Glob patterns to exclude (repeatable) | none |
| `-l`, `--lang` | Comma-separated language filter (ts,tsx,js,jsx,c,h,cpp,cc,cxx,hpp,cs,rs,py,pyi,go,java,php) | all supported |

### `projects list`

```bash
virgil-cli projects list
```

No arguments. Lists all registered projects with file counts.

### `projects delete`

```bash
virgil-cli projects delete <NAME>
```

### `projects query`

```bash
# Query a registered project — exactly one of --template / --cozoscript / --file required
virgil-cli projects query <NAME> --template <name> [--param k=v ...] [OPTIONS]
virgil-cli projects query <NAME> --cozoscript '<inline>' [--param k=v ...] [OPTIONS]
virgil-cli projects query <NAME> --file <path.cozoql> [--param k=v ...] [OPTIONS]

# Query an S3/R2 codebase directly (no registration needed)
virgil-cli projects query --s3 s3://bucket/prefix --template <name> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Project name (not needed with `--s3`) | — |
| `--s3` | S3 URI — query codebase directly from S3/R2 | — |
| `-l`, `--lang` | Comma-separated language filter (used with `--s3`) | all supported |
| `-e`, `--exclude` | Glob patterns to exclude (used with `--s3`, repeatable) | none |
| `--template` | Built-in template name (see list below) | — |
| `--cozoscript` | Inline Cozoscript query | — |
| `-f`, `--file` | Path to a Cozoscript file | — |
| `--param` | Parameter binding for `$param` references in the script (repeatable; `key=value`) | none |
| `--rebuild` | Force a fresh rebuild of the cached fact store | false |
| `--pretty` | Pretty-print JSON output | false |

Parameter values bind via `BTreeMap<String, DataValue>` — integers and `true`/`false` are auto-coerced, everything else binds as a string. User input is never interpolated into the script body.

## Built-in Templates

Templates live under `src/queries/builtin/` (pure Cozoscript) and `src/queries/rust_templates.rs` (Rust-side handlers that need source-level access).

| Template | Params | What it returns |
|---|---|---|
| `find_function_by_name` | `name` | Symbols with an exact-name match |
| `find_callers` | `target`, `max_depth` | Transitive callers of `$target` up to `$max_depth` hops |
| `find_callees` | `target`, `max_depth` | Transitive callees of `$target` |
| `find_cycles` | — | Files participating in an import cycle |
| `import_depth` | `max_depth` | Longest known import chain ending at each file (depth-bounded) |
| `export_surface` | — | Per-file `(file, exported_count, total_count)` |
| `unused_symbols` | — | Symbols with no inbound `edge_calls` (heuristic; name-based) |
| `complexity_hotspots` | `cc_threshold`, `length_threshold` | Functions exceeding cyclomatic or length thresholds; excludes tests |

`complexity_hotspots` is a Rust-side handler — it queries `*symbol` + `*file_classification` from Cozo, then calls tree-sitter to compute metrics on demand. Output uses the audit-shape convention (see below).

## Audit-shape Output

A query (or template handler) returning columns `(file, line, severity, pattern, message)` is auto-formatted as audit findings. Extra columns are preserved alongside as `extras`. Other column shapes return raw row tables.

## Output Shape

```json
{
  "project": "myapp",
  "query_ms": 17,
  "cache": "warm",
  "result": {
    "headers": ["name", "kind", "file_path", "start_line", "end_line"],
    "rows": [ ... ]
  }
}
```

`cache` is one of:

- `cold` — full parse + populate (first run on a fresh workspace)
- `warm` — reused the persistent SQLite store without any rebuild
- `incremental` — re-parsed only the changed files since last build

## Cozoscript Schema (queryable relations)

Authored queries can reach into any of these relations. See `src/cozo/schema.rs` for the canonical definitions.

| Relation | Keys / Values |
|---|---|
| `file` | `{path: String => language: String}` |
| `symbol` | `{id: Int => name, kind, file_path, start_line, end_line, exported}` |
| `callsite` | `{id: Int => name, file_path, line, caller_symbol_id, enclosing_test_name}` |
| `edge_defined_in` | `{symbol_id, file_path}` |
| `edge_calls` | `{caller_id, callee_id}` |
| `edge_imports` | `{from_path, to_path}` |
| `edge_exports` | `{file_path, symbol_id}` |
| `edge_contains` | `{parent_id, child_id}` |
| `file_classification` | `{path => is_test, is_barrel, is_generated}` |
| `nolint` | `{file_path, line => suppressed_pattern}` |
| `raw_import` | `{file_path, position => raw_path, language, kind}` (pre-resolution imports for incremental refresh) |
| `build_meta` | `{key => value}` — includes `schema_version` |
| `build_meta_files` | `{file_path => hash, size, mtime}` |

## Persistence

The fact store is persisted to `~/.cache/virgil/<hash>.cozo` (a single SQLite file). Subsequent invocations against the same workspace warm-start by reopening the existing store.

- **Schema version check**: `build_meta.schema_version` is compared on open; mismatch wipes the file and triggers a clean rebuild.
- **Warm-start check**: each file's `(size, mtime)` is compared against `build_meta_files`. Unchanged workspace → skip parsing entirely.
- **Incremental refresh**: when files change, only the touched ones re-parse; deletions cascade-delete owned facts; cross-file edges (`edge_calls`, `edge_imports`) are re-resolved from facts.
- **Force a cold rebuild** with `--rebuild`.

## Examples

```bash
# Register a project
virgil-cli projects create myapp --path ./src

# Find every function named `login`
virgil-cli projects query myapp --template find_function_by_name --param name=login

# Who calls `authenticate`? (up to depth 2)
virgil-cli projects query myapp --template find_callers \
    --param target=authenticate --param max_depth=2

# Import cycles
virgil-cli projects query myapp --template find_cycles --pretty

# Complexity hotspots above threshold
virgil-cli projects query myapp --template complexity_hotspots \
    --param cc_threshold=15 --param length_threshold=100

# Custom Cozoscript inline
virgil-cli projects query myapp --cozoscript \
    '?[name, file] := *symbol{name, file_path: file, exported: true}'

# Cozoscript from a file
virgil-cli projects query myapp --file my_query.cozoql --param target=login

# Force a fresh rebuild
virgil-cli projects query myapp --template find_cycles --rebuild

# S3 / Cloudflare R2 (no project registration)
virgil-cli projects query --s3 s3://bucket/my-repo --template find_cycles --pretty
virgil-cli projects query --s3 s3://bucket/my-repo --template find_function_by_name \
    --param name=handle --lang rs

# Server mode (Virgil Live)
virgil-cli serve --s3 s3://bucket/my-repo
virgil-cli serve --s3 s3://bucket/my-repo --lang rs --port 8080
virgil-cli serve --s3 s3://bucket/my-repo --host 0.0.0.0 --port 8080
```

## Serve (Server Mode)

Persistent HTTP server that loads a codebase from S3 once and serves Cozoscript queries over HTTP. The same persistence and warm-start logic as the CLI — the cached SQLite store is shared.

```bash
virgil-cli serve --s3 <URI> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--s3` | S3 URI — load codebase at startup | required (unless `--dir`) |
| `--dir` | Local directory (alternative to `--s3`) | — |
| `--host` | Host to bind (use `0.0.0.0` for all interfaces) | `127.0.0.1` |
| `--port` | Port to bind (use `0` for OS-assigned) | `0` |
| `-l`, `--lang` | Comma-separated language filter | all supported |
| `-e`, `--exclude` | Glob patterns to exclude (repeatable) | none |

### Lifecycle

1. Loads codebase (S3 download or local read) and builds / opens the fact store.
2. Prints ready signal to stdout: `{"ready": true, "port": <actual_port>}`.
3. Serves HTTP requests until killed (SIGTERM/SIGINT).

### HTTP API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check — returns `{"status": "ok"}` |
| `/query` | POST | Runs a Cozoscript template or inline body against the store |

**Query request body** — exactly one of `cozoscript` or `template`:

```json
{
  "template": "find_callers",
  "params": {"target": "authenticate", "max_depth": "2"}
}
```

```json
{
  "cozoscript": "?[name, file] := *symbol{name, file_path: file, exported: true}",
  "params": {}
}
```

Response shape mirrors the CLI's `result` envelope.

## S3 Configuration

S3 support works with AWS S3, Cloudflare R2, MinIO, and any S3-compatible storage. Configure via environment variables:

| Variable | Fallback | Description |
|----------|----------|-------------|
| `S3_ENDPOINT` | `AWS_ENDPOINT_URL` | Custom endpoint URL (required for R2/MinIO) |
| `S3_ACCESS_KEY_ID` | `AWS_ACCESS_KEY_ID` | Access key |
| `S3_SECRET_ACCESS_KEY` | `AWS_SECRET_ACCESS_KEY` | Secret key |
| `AWS_REGION` | `auto` | AWS region (defaults to `auto` for R2) |

Standard AWS credential chain (env vars, `~/.aws/credentials`, IAM roles) is also supported.

Example `.env` for Cloudflare R2:

```bash
S3_ACCESS_KEY_ID=your_access_key
S3_SECRET_ACCESS_KEY=your_secret_key
S3_ENDPOINT=https://your-account-id.r2.cloudflarestorage.com
```

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

- **Multi-language** — TypeScript, JavaScript, C, C++, C#, Rust, Python, Go, Java, and PHP via tree-sitter
- **Cozoscript query language** — Datalog over a fact store
- **Persistent fact store** — SQLite-backed Cozo store cached at `~/.cache/virgil/<hash>.cozo`
- **Warm-start in milliseconds** — unchanged workspaces skip parsing entirely; ~17ms on the reference workspace vs ~850ms cold
- **Incremental refresh** — modifying / adding / removing one file re-parses only that file and re-resolves cross-file edges
- **Audit-shape output convention** — `(file, line, severity, pattern, message)` columns auto-format as findings
- **Parameter binding** — `--param key=value`; user input never interpolated into the script body
- **S3 support** — AWS S3, Cloudflare R2, MinIO, or any S3-compatible storage
- **Server mode** — persistent HTTP server that loads a codebase once and serves Cozoscript queries

## License

MIT
