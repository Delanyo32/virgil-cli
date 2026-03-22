# Plan: `virgil-cli serve` — Persistent HTTP Server Mode

## Context

Virgil Live's audit pipeline currently spawns a fresh `virgil-cli` process for every query and audit scan. Each spawn re-downloads the codebase from S3 and re-parses it. During a single audit with investigation, this means 5+ process spawns for the scan phase and 10-30+ spawns during investigation as the AI agent reads files and queries symbols.

This plan adds a `serve` subcommand that loads the codebase once and serves queries and audit scans over HTTP for the lifetime of a single audit run.

---

## New Subcommand

```
virgil-cli serve --s3 <uri> --port <port>
```

### Behavior

1. Parse `--s3 <uri>` (e.g., `s3://bucket/userId/owner/repo/branch`)
2. Parse `--port <port>` (use `0` for OS-assigned dynamic port)
3. Download the entire codebase from S3 into memory
4. Parse the codebase (build AST, symbol index — same as current `query` and `audit` commands)
5. Bind an HTTP server on the specified port
6. Print a JSON ready signal to stdout: `{"ready": true, "port": <actual_port>}`
7. Serve HTTP requests until the process is killed (SIGTERM/SIGINT)

### Startup Errors

If loading from S3 fails or the codebase cannot be parsed, print the error to stderr and exit with a non-zero code **before** printing the ready signal. The caller (TypeScript wrapper) reads stderr and knows the server failed to start.

---

## HTTP API

### `GET /health`

Health check endpoint.

**Response** `200`:
```json
{"status": "ok"}
```

---

### `POST /query`

Codebase query — symbol search, file reads, relationships. Uses the existing JSON query language that `virgil-cli projects query` already supports.

**Request body**:
```json
{
  "query": {"find": "function", "name": "*handle*"},
  "format": "outline",
  "max": 50
}
```

- `query` — the JSON query object (same schema as `--q` flag on `projects query`)
- `format` — output format: `outline | snippet | full | tree | locations | summary`
- `max` — max results (optional, default 50)

File reads use the existing query syntax:
```json
{
  "query": {"read": "src/auth.ts"},
  "format": "full"
}
```

**Response** `200`:
```json
{
  "results": [...]
}
```

Same response format as the current `projects query` CLI output.

**Response** `400`:
```json
{"error": "Invalid query: ..."}
```

---

### `POST /audit/summary`

Run the audit summary scan (file counts, finding counts).

**Response** `200`:
```json
{
  "files_scanned": 150,
  "files_with_findings": 23
}
```

Same response format as `virgil-cli audit --format json`.

---

### `POST /audit/:category`

Run an audit scan for a specific category. Returns individual findings.

**Path params**: `category` — one of `architecture`, `security`, `scalability`, `code-quality`

For `code-quality`, run all subcategories (`tech-debt`, `complexity`, `code-style`) in parallel and merge results — same as the current `runAuditCategory` behavior.

**Request body** (optional):
```json
{
  "per_page": 100000
}
```

Default `per_page` is `100000` (return all findings).

**Response** `200`:
```json
[
  {
    "file_path": "src/auth.ts",
    "line": 42,
    "column": 0,
    "severity": "warning",
    "pipeline": "input_validation",
    "pattern": "missing_check",
    "message": "Missing input validation before processing",
    "snippet": "const data = process(input);"
  }
]
```

Same finding format as `virgil-cli audit <category> --format json`.

**Response** `400`:
```json
{"error": "Unknown category: xyz"}
```

---

## Lifecycle

```
Caller (TypeScript)                    virgil-cli serve
       |                                      |
       |--- spawn process ------------------>|
       |                                      |-- load codebase from S3
       |                                      |-- parse + index
       |                                      |-- bind HTTP server
       |<--- stdout: {"ready":true,"port":N} -|
       |                                      |
       |--- POST /audit/summary ------------->|-- serve from memory
       |<-- response ------------------------|
       |                                      |
       |--- POST /audit/security ----------->|-- serve from memory
       |<-- response ------------------------|
       |                                      |
       |--- POST /query -------------------->|-- serve from memory
       |<-- response ------------------------|
       |                                      |
       |  (many more queries during           |
       |   investigation batches)             |
       |                                      |
       |--- SIGTERM ------------------------>|
       |                                      |-- exit(0)
```

### Key Properties

- **One process per audit run** — not per query, not per category
- **Codebase loaded once** — all subsequent requests served from memory
- **Dynamic port** — use `--port 0`, actual port reported in ready signal
- **No state between requests** — each request is independent (no sessions)
- **Graceful shutdown** — handle SIGTERM, close HTTP listener, exit cleanly

---

## Implementation Notes

### Recommended Rust Crates

- **HTTP server**: `axum` or `actix-web` (both async, production-ready)
- **Port binding**: Use `TcpListener::bind("0.0.0.0:0")` for OS-assigned port, read `local_addr()` for actual port
- **Ready signal**: `println!("{}", serde_json::to_string(&ReadySignal { ready: true, port }))`

### Code Reuse

The `serve` command reuses the same internal functions as the existing CLI commands:

| Endpoint | Reuses |
|----------|--------|
| `POST /query` | Same query engine as `virgil-cli projects query` |
| `POST /audit/summary` | Same scanner as `virgil-cli audit --format json` |
| `POST /audit/:category` | Same scanner as `virgil-cli audit <category> --format json` |

The difference is that the codebase is loaded once at startup and shared across all handlers, rather than loaded per invocation.

### Memory Considerations

The server holds the entire codebase in memory for the duration of the audit. For large repos (100K+ files), this could use significant RAM. The server process is short-lived (minutes, not hours) and is killed after the audit completes, so memory is reclaimed.

---

## Testing

1. **Unit**: Query and audit handlers return correct results from in-memory codebase
2. **Integration**: `spawn → ready signal → HTTP requests → kill` lifecycle
3. **Edge cases**: Invalid S3 URI (error before ready), unknown category (400), server killed mid-request (graceful), port 0 allocation
4. **Performance**: Compare query latency (spawn-per-call vs. server mode) on a medium repo (~1K files)
