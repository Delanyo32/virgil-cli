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
| `find_function_by_name` | `name` | Function/method symbols whose `name` or `qualified_name` matches |
| `find_callers` | `name` | Direct callers of the callee `$name` |
| `find_callees` | `name` | Direct callees of the caller `$name` |
| `find_cycles` | — | Pairs of mutually-reachable symbols in the call graph |
| `import_depth` | — | Longest file-import chain ending at each file |
| `export_surface` | — | Public exported symbols whose host file is imported elsewhere |
| `unused_symbols` | — | Exported symbols with no inbound `references` rows |
| `find_implementations_of` | `name` | Types that `implements`/`extends` `$name` |
| `find_writers_of` | `name` | Source locations writing to a symbol named `$name` (`references.ref_kind = "write"`) |
| `complexity_hotspots` | `cc_threshold`, `length_threshold` | Functions exceeding cyclomatic or length thresholds; excludes tests |

`complexity_hotspots` is a Rust-side handler — it queries `*symbol` + `*span` + `*file_classification` from Cozo, then calls tree-sitter to compute metrics on demand. Output uses the audit-shape convention (see below).

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
| `file` | `{path => language, repo_id}` |
| `symbol` | `{id => kind, name, qualified_name, language, visibility, file_path, parent_id?, is_async, is_static, is_abstract, is_mutable, exported}` |
| `span` | `{entity_id, file_path => start_byte, end_byte, start_line, end_line, start_col, end_col}` — positional metadata for symbols/comments/call sites |
| `calls` | `{caller_id, callee_id => call_site_file, call_site_start_byte, call_site_end_byte, is_direct}` |
| `references` | `{referrer_id, site_file, site_start_byte, match_index => referent_id?, ref_kind}` — `ref_kind` is `read`/`write`/`call`/`type_use`/`import_use`; `referent_id` is null when unresolved |
| `occurrence` | `{id => name, file_path, start_byte, end_byte, enclosing_symbol_id?, enclosing_scope_id, occurrence_kind}` — raw identifier occurrences (input to the resolver) |
| `scope` | `{id => parent_id?, file_path, kind, start_byte, end_byte}` — lexical scope chain |
| `binding` | `{scope_id, name, start_byte => symbol_id?, binding_kind}` — name → symbol within a scope |
| `extends` | `{child_id, parent_id}` |
| `implements` | `{impl_id, interface_id}` |
| `imports` | `{importer_file_id, imported_id}` |
| `raw_import` | `{file_path, position => raw_path, language, kind}` (pre-resolution imports for incremental refresh) |
| `parameter` | `{id => name, function_id, position, type_id?, is_optional, has_default, is_taint_source}` |
| `returns_type` | `{function_id => type_id}` |
| `throws` | `{function_id, exception_type_id}` |
| `field_type` | `{symbol_id => type_id}` |
| `type` | `{id => kind, language, display_name, canonical_name?}` — `kind` is `primitive`/`named`/`generic`/`union`/`intersection`/`function`/`tuple`/`array` |
| `comment` | `{id => documents_id?, file_path, kind, is_doc, text, todo_kind?, start_byte, end_byte}` |
| `<lang>_attrs` | per-language attribute table (`rust_attrs`, `python_attrs`, `typescript_attrs`, `cpp_attrs`, `csharp_attrs`, `go_attrs`, `php_attrs`, `c_attrs`, `java_attrs`) |
| `file_classification` | `{path => is_test, is_barrel, is_generated}` |
| `nolint` | `{file_path, line => suppressed_pattern}` |
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

# Who calls `authenticate`?
virgil-cli projects query myapp --template find_callers --param name=authenticate

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
  "params": {"name": "authenticate"}
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
- **Scales to multi-thousand-file codebases** — staged Cozoscript resolver + Rust-side `match_index` assignment; cold-builds a 5k-file workload in ~3 min
- **Incremental refresh** — modifying / adding / removing one file re-parses only that file and re-resolves cross-file edges
- **Audit-shape output convention** — `(file, line, severity, pattern, message)` columns auto-format as findings
- **Parameter binding** — `--param key=value`; user input never interpolated into the script body
- **S3 support** — AWS S3, Cloudflare R2, MinIO, or any S3-compatible storage
- **Server mode** — persistent HTTP server that loads a codebase once and serves Cozoscript queries

## License

MIT
