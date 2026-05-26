# virgil-cli

A fast Rust CLI that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases on-demand with [tree-sitter](https://tree-sitter.github.io/), materialises them into a [DuckDB](https://duckdb.org/) fact store, and answers queries via SQL — including graph traversal via the [duckpgq](https://duckpgq.org/) SQL/PGQ extension. Persistent on-disk cache with warm-start in tens of milliseconds.

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

The DuckDB backend is bundled (no system DuckDB required). `duckpgq` is installed from the community extension repository the first time you run a cold build; the binary is cached under `~/.duckdb/extensions/` for subsequent runs.

## Usage

```bash
virgil-cli projects <COMMAND>
```

## Projects

All commands are nested under `virgil-cli projects`:

| Command | Description |
|---------|-------------|
| `create` | Register a project for querying (scans files, saves to `~/.virgil-cli/projects.json`) |
| `list` | List registered projects with file counts |
| `delete` | Remove a registered project |
| `query` | Run a SQL template, file, or inline query against the project's fact store |

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
# Exactly one of --template / --sql / --file required
virgil-cli projects query <NAME> --template <name> [--param k=v ...] [OPTIONS]
virgil-cli projects query <NAME> --sql '<inline>' [--param k=v ...] [OPTIONS]
virgil-cli projects query <NAME> --file <path.sql> [--param k=v ...] [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Project name | required |
| `-l`, `--lang` | Comma-separated language filter | all supported |
| `-e`, `--exclude` | Glob patterns to exclude (repeatable) | none |
| `--template` | Built-in template name (see list below) | — |
| `--sql` | Inline SQL query | — |
| `-f`, `--file` | Path to a SQL file | — |
| `--param` | Parameter binding for `$param` references in the script (repeatable; `key=value`) | none |
| `--rebuild` | Force a fresh rebuild of the cached fact store | false |
| `--pretty` | Pretty-print JSON output | false |

Parameters substitute into `$name` placeholders in the SQL as quoted literals. Integers and `true`/`false` are auto-coerced; everything else binds as a string. (DuckDB's positional `?` binding isn't used because duckpgq's `GRAPH_TABLE(... WHERE ...)` doesn't consume placeholders — see [`docs/experiments/duckdb-swap.md`](docs/experiments/duckdb-swap.md) for the long story.)

## Built-in Templates

Templates live under `src/queries/builtin/` (pure SQL) and `src/queries/rust_templates.rs` (Rust-side handlers that need source-level access).

| Template | Params | What it returns |
|---|---|---|
| `find_function_by_name` | `name` | Function/method symbols whose `name` or `qualified_name` matches |
| `find_callers` | `name` | Direct callers of the callee `$name` (PGQ MATCH on `call_edge`) |
| `find_callees` | `name` | Direct callees of the caller `$name` (PGQ MATCH on `call_edge`) |
| `find_cycles` | — | Pairs of mutually-reachable symbols (recursive CTE over `call_edge`) |
| `import_depth` | — | Longest file-import chain ending at each file (recursive CTE) |
| `export_surface` | — | Public exported symbols whose host file is imported elsewhere |
| `find_implementations_of` | `name` | Types that `implements`/`extends` `$name` |
| `complexity_hotspots` | `cc_threshold`, `length_threshold` | Functions exceeding cyclomatic or length thresholds; excludes tests |

`complexity_hotspots` is a Rust-side handler — it queries `symbol` + `span` + `file_classification` from DuckDB, then calls tree-sitter to compute metrics on demand. Output uses the audit-shape convention (see below).

### Why PGQ for some templates and plain SQL for others

The schema defines `CREATE PROPERTY GRAPH codegraph` with vertex tables `file` + `symbol` and edge tables `call_edge`, `imports`, `extends`, `implements`. Templates that traverse those edges single-hop (`find_callers`, `find_callees`) use the PGQ `GRAPH_TABLE(... MATCH ...)` form for declarative clarity. Templates that need transitive closure (`find_cycles`, `import_depth`) fall back to `WITH RECURSIVE` CTEs over the underlying tables — duckpgq 1.x crashes when `GRAPH_TABLE` is wrapped in a `WITH` clause.

### Resolved call edges (`call_edge`)

`call_edge {caller_id, callee_id, file_path}` is a precomputed call-graph edge table populated at build time by the parallel Rust resolver in `from_code_graph::resolve_and_emit_call_edges`. Intra-file matches by `(name, kind)` plus cross-file via `imports` + `exported=true`. Queries join it directly instead of recomputing the resolution.

```sql
-- All symbols that transitively call `parse`:
SELECT DISTINCT caller.name, caller.file_path
FROM GRAPH_TABLE (codegraph
    MATCH ANY ACYCLIC (caller:symbol)-[e:calls]->+(callee:symbol)
    WHERE callee.name = 'parse'
    COLUMNS (caller.name AS name, caller.file_path AS file_path)
);
```

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
- `warm` — reused the persistent DuckDB store without any rebuild

(Incremental refresh is not implemented in the DuckDB branch — pass `--rebuild` to force a fresh parse.)

## Schema (queryable tables)

Authored queries can reach into any of these tables. See `src/db/schema.rs` for the canonical DDL.

| Table | Columns |
|---|---|
| `file` | `path PK, language, repo_id` |
| `symbol` | `id PK, kind, name, qualified_name, language, visibility, file_path, parent_id, is_async, is_static, is_abstract, is_mutable, exported` |
| `span` | `(entity_id, file_path) PK, start_byte, end_byte, start_line, end_line, start_col, end_col` — positional metadata for symbols / comments / call sites |
| `calls` | `(caller_id, callee_id) PK, call_site_file, call_site_start_byte, call_site_end_byte, is_direct` |
| `call_site` | `id PK, caller_id, callee_name, file_path, start_byte, end_byte` — raw, unresolved call sites |
| `call_edge` | `(caller_id, callee_id) PK, file_path` — resolved direct call edges (PGQ edge table for `codegraph`) |
| `occurrence` | `id PK, name, file_path, start_byte, end_byte, enclosing_symbol_id, enclosing_scope_id, occurrence_kind` |
| `scope` | `id PK, parent_id, file_path, kind, start_byte, end_byte` |
| `binding` | `(scope_id, name, start_byte) PK, symbol_id, binding_kind` |
| `extends` | `(child_id, parent_id) PK` (PGQ edge table for `codegraph`) |
| `implements` | `(impl_id, interface_id) PK` (PGQ edge table for `codegraph`) |
| `imports` | `(importer_file_id, imported_id) PK` (PGQ edge table for `codegraph`) |
| `raw_import` | `(file_path, position) PK, raw_path, language, kind` |
| `parameter` | `id PK, name, function_id, position, type_id, is_optional, has_default, is_taint_source` |
| `returns_type` | `function_id PK, type_id` |
| `throws` | `(function_id, exception_type_id) PK` |
| `field_type` | `symbol_id PK, type_id` |
| `type` | `id PK, kind, language, display_name, canonical_name` |
| `comment` | `id PK, documents_id, file_path, kind, is_doc, text, todo_kind, start_byte, end_byte` |
| `<lang>_attrs` | per-language attribute table (`rust_attrs`, `python_attrs`, `typescript_attrs`, `cpp_attrs`, `csharp_attrs`, `go_attrs`, `php_attrs`, `c_attrs`, `java_attrs`) |
| `file_classification` | `path PK, is_test, is_barrel, is_generated` |
| `nolint` | `(file_path, line) PK, suppressed_pattern` |
| `build_meta` | `key PK, value` — includes `schema_version` |
| `build_meta_files` | `file_path PK, hash, size, mtime` |

## Persistence

The fact store is persisted to `~/.cache/virgil/<hash>.duckdb` (a single DuckDB file). Subsequent invocations against the same workspace warm-start by reopening the existing store.

- **Schema version check**: `build_meta.schema_version` is compared on open; mismatch wipes the file and triggers a clean rebuild.
- **Force a cold rebuild** with `--rebuild`.
- **No incremental refresh** in this branch — changing files requires `--rebuild` to pick up the change. (See `docs/experiments/duckdb-swap.md` for why incremental was deferred.)

### Benchmark snapshot

From `docs/experiments/duckdb-swap-findings.md` (DuckDB branch vs Cozo on the same machine, 2 corpora available):

| Repo | Phase | Cozo | DuckDB | Speedup |
|---|---|---:|---:|---:|
| openclaw/discord (522 ts/tsx) | parse cold | 1.79 s / 204 MB | **0.47 s / 110 MB** | 3.8× / 1.9× memory |
| openclaw/discord | find_cycles | 12.23 s | **0.28 s** | 43× |
| openclaw/discord | import_depth | 30 s (TIMEOUT) / 2.8 GB | **0.28 s / 44 MB** | >100× |
| openclaw/ui (461 ts/tsx) | parse cold | 1.78 s / 248 MB | **0.45 s / 129 MB** | 4.0× / 1.9× memory |
| openclaw/ui | find_cycles | 30 s (TIMEOUT) | **0.28 s** | >100× |

Flat queries (`find_function_by_name`, `find_callers`, `find_callees`, etc.) tie within process-startup noise (~0.26-0.28s) because both engines are dominated by binary launch + extension load, not real query work.

## Examples

```bash
# Register a project
virgil-cli projects create myapp --path ./src

# Find every function named `login`
virgil-cli projects query myapp --template find_function_by_name --param name=login

# Who calls `authenticate`?
virgil-cli projects query myapp --template find_callers --param name=authenticate

# Cycle detection (recursive CTE over the materialised call graph)
virgil-cli projects query myapp --template find_cycles --pretty

# Complexity hotspots above threshold (Rust-side handler)
virgil-cli projects query myapp --template complexity_hotspots \
    --param cc_threshold=15 --param length_threshold=100

# Custom inline SQL
virgil-cli projects query myapp --sql \
    "SELECT name, file_path FROM symbol WHERE exported = true ORDER BY file_path"

# SQL from a file
virgil-cli projects query myapp --file my_query.sql --param target=login

# Force a fresh rebuild
virgil-cli projects query myapp --template find_cycles --rebuild
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
- **SQL query language** — standard SQL with `WITH RECURSIVE` for graph closures; SQL/PGQ via duckpgq for declarative `MATCH` patterns
- **Persistent fact store** — single-file DuckDB store cached at `~/.cache/virgil/<hash>.duckdb`
- **Warm-start in milliseconds** — unchanged workspaces skip parsing entirely
- **Scales to multi-thousand-file codebases** — streamed DuckDB writes during absorb (Arrow-backed `Appender` for scalar tables, batched `INSERT` for array columns)
- **Audit-shape output convention** — `(file, line, severity, pattern, message)` columns auto-format as findings
- **Parameter binding** — `--param key=value` substitutes into `$name` placeholders
- **Materialised call graph** — `call_edge` built at parse time so warm queries hit a join instead of a recursive resolver

## License

MIT
