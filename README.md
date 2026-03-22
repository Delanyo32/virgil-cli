# virgil-cli

A fast Rust CLI that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases on-demand with [tree-sitter](https://tree-sitter.github.io/) and queries them via a composable JSON query language. Includes static analysis auditing across 4 categories. No database, no pre-indexing — projects are registered by name and parsed at query time. Supports S3-compatible storage (AWS S3, Cloudflare R2, MinIO) for querying and auditing remote codebases directly.

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

- **Core workflow**: Register → Query → Drill-down exploration pattern
- **6 strategic playbooks**: Architecture understanding, symbol tracing, onboarding, bug investigation, dependency mapping, API surface mapping
- **Full command reference**: 4 project commands + audit commands
- **Note**: Skill reference files (`.agents/skills/virgil/`) still reference old architecture — separate update needed

## Usage

Three top-level command groups:

```bash
virgil projects <COMMAND>
virgil audit [CATEGORY] <DIR|--s3 URI> [OPTIONS]
virgil serve --s3 <URI> [OPTIONS]
```

## Projects

All project commands are nested under `virgil projects`:

| Command | Description |
|---------|-------------|
| `create` | Register a project for querying (scans files, saves to `~/.virgil-cli/projects.json`) |
| `list` | List registered projects with file counts |
| `delete` | Remove a registered project |
| `query` | Query a project using inline JSON (`--q`), a file (`--file`), or stdin |

### `projects create`

```bash
virgil projects create <NAME> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Project name | required |
| `-p`, `--path` | Root directory of the project | `.` |
| `-e`, `--exclude` | Glob patterns to exclude (repeatable) | none |
| `-l`, `--lang` | Comma-separated language filter (ts,tsx,js,jsx,c,h,cpp,cc,cxx,hpp,cs,rs,py,pyi,go,java,php) | all supported |

### `projects list`

```bash
virgil projects list
```

No arguments. Lists all registered projects with file counts.

### `projects delete`

```bash
virgil projects delete <NAME>
```

| Option | Description |
|--------|-------------|
| `<NAME>` | Project name to delete |

### `projects query`

```bash
# Query a registered project
virgil projects query <NAME> [OPTIONS]

# Query an S3/R2 codebase directly (no registration needed)
virgil projects query --s3 s3://bucket/prefix [OPTIONS]
```

Pass a query via `--q` (inline JSON), `--file` (path to JSON file), or pipe JSON to stdin.

| Option | Description | Default |
|--------|-------------|---------|
| `<NAME>` | Project name (not needed with `--s3`) | — |
| `--s3` | S3 URI — query codebase directly from S3/R2 | — |
| `-l`, `--lang` | Comma-separated language filter (used with `--s3`) | all supported |
| `-e`, `--exclude` | Glob patterns to exclude (used with `--s3`, repeatable) | none |
| `-q`, `--q` | Inline JSON query | — |
| `-f`, `--file` | Path to a JSON query file | — |
| `-o`, `--out` | Output format (outline, snippet, full, tree, locations, summary) | `outline` |
| `--pretty` | Pretty-print JSON output | false |
| `-m`, `--max` | Maximum number of results | `100` |

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
  "depth": 2,
  "format": "full",
  "read": "src/main.rs"
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
| `format` | string | Override output format from within query JSON |
| `read` | string | File path to read (returns content instead of symbols). Combine with `lines` for a specific range |

## Query Output Formats

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

## Audit

Static analysis and tech debt detection. All audit commands are nested under `virgil audit`:

```bash
# Local directory
virgil audit <DIR>                         # Run all audit categories
virgil audit code-quality <DIR>            # All code quality checks
virgil audit code-quality tech-debt <DIR>  # Tech debt patterns
virgil audit code-quality complexity <DIR> # Complexity metrics
virgil audit code-quality code-style <DIR> # Code style issues
virgil audit security <DIR>               # Security vulnerabilities
virgil audit scalability <DIR>            # Scalability issues
virgil audit architecture <DIR>           # Architecture analysis

# S3/R2 (no registration needed)
virgil audit --s3 s3://bucket/prefix
virgil audit security --s3 s3://bucket/prefix --language rs
```

### Common Options

All audit subcommands support:

| Option | Description | Default |
|--------|-------------|---------|
| `--s3` | S3 URI — audit codebase directly from S3/R2 (replaces `<DIR>`) | — |
| `-l`, `--language` | Comma-separated language filter | all supported |
| `--pipeline` | Comma-separated pipeline filter | all pipelines |
| `--format` | Output format (table, json, csv) | `table` |
| `--per-page` | Findings per page | `20` |
| `--page` | Page number (1-indexed) | `1` |

### `audit code-quality tech-debt`

Detect tech debt patterns in source code.

Pipelines: `panic_detection`

### `audit code-quality complexity`

Measure code complexity metrics.

Pipelines: `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`

### `audit code-quality code-style`

Detect code style issues.

Pipelines: `dead_code`, `duplicate_code`, `coupling`

### `audit security`

Security vulnerability detection.

Pipelines: injection, unsafe memory, race conditions

### `audit scalability`

Scalability analysis.

Pipelines: `n_plus_one_queries`, `sync_blocking_in_async`, `memory_leak_indicators`

### `audit architecture`

Architecture analysis.

Pipelines: `module_size_distribution`, `circular_dependencies`, `dependency_graph_depth`, `api_surface_area`

## Examples

```bash
# Register a project
virgil projects create myapp --path ./src

# Register with language filter and exclusions
virgil projects create myapp --path ./src --lang ts,tsx,js,jsx --exclude "vendor/**"

# List registered projects
virgil projects list

# Find all exported functions
virgil projects query myapp --q '{"find": "function", "visibility": "exported"}'

# Search by name pattern with preview
virgil projects query myapp --q '{"name": "handle*", "preview": 5}' --pretty

# Methods inside a specific class
virgil projects query myapp --q '{"find": "method", "inside": "AuthService"}'

# Large functions (50+ lines) in a directory
virgil projects query myapp --q '{"files": "src/api/**", "find": "function", "lines": {"min": 50}}'

# Functions missing docstrings
virgil projects query myapp --q '{"find": "function", "has": {"not": "docstring"}}'

# Name regex — all getters
virgil projects query myapp --q '{"name": {"regex": "^get[A-Z]"}}'

# Call graph — what does authenticate() call?
virgil projects query myapp --q '{"name": "authenticate", "calls": "down", "depth": 2}'

# Summary of an entire project
virgil projects query myapp --q '{}' --out summary --pretty

# Read a file
virgil projects query myapp --q '{"read": "src/main.rs"}' --pretty

# Read specific lines from a file
virgil projects query myapp --q '{"read": "src/main.rs", "lines": {"min": 10, "max": 25}}'

# File:line locations only
virgil projects query myapp --q '{"find": "class"}' --out locations

# Query from a file
virgil projects query myapp --file query.json

# Run all audit categories
virgil audit ./src

# Run security audit with JSON output
virgil audit security ./src --format json

# Run complexity analysis filtered to Rust
virgil audit code-quality complexity ./src --language rs

# Run a specific architecture pipeline
virgil audit architecture ./src --pipeline circular_dependencies

# Delete a project
virgil projects delete myapp

# --- S3 / Cloudflare R2 ---

# Query an S3 codebase directly (no project registration)
virgil projects query --s3 s3://bucket/my-repo --q '{"find": "function"}' --out summary --pretty

# Query with language filter
virgil projects query --s3 s3://bucket/my-repo --q '{}' --out summary --lang rs

# Audit an S3 codebase
virgil audit --s3 s3://bucket/my-repo --language rs

# Security audit on S3 codebase
virgil audit security --s3 s3://bucket/my-repo --language rs

# --- Server Mode ---

# Start a persistent HTTP server (loads codebase once, serves queries/audits over HTTP)
virgil serve --s3 s3://bucket/my-repo

# With language filter and custom port
virgil serve --s3 s3://bucket/my-repo --lang rs --port 8080

# Expose on all interfaces (default is 127.0.0.1)
virgil serve --s3 s3://bucket/my-repo --host 0.0.0.0 --port 8080
```

## Serve (Server Mode)

Persistent HTTP server that loads a codebase from S3 once and serves queries and audits over HTTP. Designed for use by orchestrators (e.g. AI agents) that make many queries against the same codebase — avoids re-downloading and re-parsing on every request.

```bash
virgil serve --s3 <URI> [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--s3` | S3 URI — load codebase at startup | required |
| `--host` | Host to bind (use `0.0.0.0` for all interfaces) | `127.0.0.1` |
| `--port` | Port to bind (use `0` for OS-assigned) | `0` |
| `-l`, `--lang` | Comma-separated language filter | all supported |
| `-e`, `--exclude` | Glob patterns to exclude (repeatable) | none |

### Lifecycle

1. Downloads codebase from S3 into memory
2. Prints ready signal to stdout: `{"ready": true, "port": <actual_port>}`
3. Serves HTTP requests until killed (SIGTERM/SIGINT)

### HTTP API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check — returns `{"status": "ok"}` |
| `/query` | POST | Codebase query (same JSON query language as `projects query`) |
| `/audit/summary` | POST | Audit summary (files scanned, files with findings) |
| `/audit/{category}` | POST | Audit by category: `architecture`, `security`, `scalability`, `code-quality` |

**Query request body:**

```json
{
  "query": {"find": "function", "name": "*handle*"},
  "format": "outline",
  "max": 50
}
```

**Audit category request body (optional):**

```json
{
  "per_page": 100000
}
```

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

- **Multi-language** — TypeScript, JavaScript, C, C++, C#, Rust, Python, Go, Java, and PHP with language-specific parsers
- **Fast** — parallel file processing with rayon
- **Accurate** — tree-sitter parsing (same parsers used by editors like Neovim and Zed)
- **Gitignore-aware** — automatically skips `node_modules`, `dist`, `build`, and anything in `.gitignore`
- **On-demand parsing** — no pre-indexing or database, projects are parsed at query time
- **JSON query language** — composable filters for symbols, files, visibility, call graphs, and more
- **Call graph** — name-based callee/caller traversal with configurable depth
- **Export detection** — tracks whether symbols are exported (ES exports, C linkage, C#/Java access modifiers, Rust visibility, Go capitalization, Python underscore convention, PHP visibility)
- **Static analysis** — 4 audit categories (code quality, security, scalability, architecture) with multiple pipelines
- **File reading** — read source files or specific line ranges via the `read` query field
- **Multiple output formats** — outline, snippet, full, tree, locations, summary (all JSON)
- **In-memory workspace** — files loaded upfront for fast repeated queries
- **S3 support** — query and audit codebases directly from AWS S3, Cloudflare R2, MinIO, or any S3-compatible storage
- **Server mode** — persistent HTTP server that loads a codebase once and serves queries/audits over HTTP, avoiding repeated S3 downloads and re-parsing

## License

MIT
