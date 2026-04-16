# External Integrations

**Analysis Date:** 2026-04-16

## APIs & External Services

**Cloud Storage:**
- AWS S3 / S3-compatible storage (Cloudflare R2, MinIO)
  - What it's used for: Remote codebase scanning and audit without local registration
  - SDK/Client: `aws-sdk-s3` 1.x
  - URI format: `s3://bucket/prefix`
  - Used in: `--s3 s3://bucket/prefix` flag on `query` and `audit` commands
  - Location: `src/s3.rs` - S3 client building, object listing, concurrent download

**GitHub:**
- Release artifact hosting
  - What it's used for: Distributing precompiled binaries via GitHub Releases
  - Pipeline: `.github/workflows/release.yml` builds for 5 targets, creates release with artifacts
  - Supports: cargo-binstall for `cargo install --git` workflow

## Data Storage

**Databases:**
- None - Not applicable

**File Storage:**
- Local filesystem: Project registry stored at `~/.virgil-cli/projects.json`
  - Format: JSON (serde-serialized `Vec<ProjectEntry>`)
  - Atomic writes: `.tmp` file + rename pattern (see `src/registry.rs`)
  - Contents: project name, path, language filter, file count, language breakdown, created_at
  
- S3-compatible storage (optional): Remote codebase files
  - Connection: Configured via env vars (`S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`, `S3_ENDPOINT`)
  - Client: `aws-sdk-s3::Client` (see `src/s3.rs`)
  - Credentials: Checks env vars first, then standard AWS credential chain

**Memory-based:**
- In-memory workspace: All project files loaded into `MemoryFileSource` at startup
  - Storage: `HashMap<String, Arc<str>>` with zero-copy Arc strings
  - Purpose: Fast repeated parsing without disk I/O
  - Location: `src/workspace.rs` - `Workspace::load()` and `Workspace::load_from_s3()`

**Caching:**
- None - Each query/audit parses files fresh from memory. No caching layer.

## Authentication & Identity

**Auth Provider:**
- None - CLI tool with no user authentication
- S3 credentials: Static env vars (`S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`)
- AWS credential chain: Falls back to standard locations (`~/.aws/credentials`, IAM roles)

## Monitoring & Observability

**Error Tracking:**
- None - Not integrated

**Logs:**
- stderr: Diagnostic messages and warnings (file parse failures, progress)
- stdout: Structured JSON output (query results, audit findings)
- eprintln! used for: project creation confirmations, progress bars (indicatif)

## CI/CD & Deployment

**Hosting:**
- GitHub Actions - CI/CD pipeline
- Self-hosted runners: Ubuntu (x86_64, aarch64), macOS (Intel, Apple Silicon), Windows

**CI Pipeline:**
- `.github/workflows/ci.yml`: Format, clippy, tests on every push/PR
  - Format: `cargo fmt --check` (rustfmt)
  - Linting: `cargo clippy --all-targets -- -D warnings`
  - Tests: `cargo test` on Ubuntu, macOS, Windows
  - Caching: Swatinem/rust-cache@v2
  
- `.github/workflows/release.yml`: Cross-platform binary builds on version tags
  - Trigger: `push` with tag matching `release/v*`
  - Targets: 5 (x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin, x86_64-windows)
  - Archive: tar.gz (Unix), zip (Windows)
  - Upload: GitHub Releases with checksums (sha256sum)
  - Release notes: Auto-generated from git history

**Distribution:**
- GitHub Releases: Direct downloads of precompiled binaries
- cargo-binstall: `cargo install virgil-cli` via binary installer metadata in Cargo.toml

## Environment Configuration

**Required env vars (S3 mode):**
- `S3_ACCESS_KEY_ID` (or `AWS_ACCESS_KEY_ID`) - Access key ID
- `S3_SECRET_ACCESS_KEY` (or `AWS_SECRET_ACCESS_KEY`) - Secret access key
- `S3_ENDPOINT` (or `AWS_ENDPOINT_URL`) - Custom endpoint (optional, for R2 or MinIO)
- `AWS_REGION` - Region (optional, defaults to "auto")

**Optional env vars:**
- `AWS_REGION` - Defaults to "auto" if not set (R2 compatibility)
- Standard AWS credential chain as fallback

**Secrets location:**
- Environment variables (CI: GitHub Actions secrets)
- Local development: `.env` file (never committed, listed in `.gitignore`)

## Webhooks & Callbacks

**Incoming:**
- HTTP POST endpoints (server mode only):
  - `POST /query` - JSON query execution
  - `POST /audit/summary` - All audit categories
  - `POST /audit/{category}` - Specific audit (architecture, security, scalability, code-quality)
  - Location: `src/server.rs` - axum router and handlers

**Outgoing:**
- None

**Health:**
- `GET /health` - Readiness check (returns `{"status": "ok"}`)
- Server startup signal: stdout writes `{"ready": true, "port": N}` on successful bind

## Query Input/Output

**Input Formats:**
- Inline JSON: `--q '{"find": "function", "name": "handle*"}'`
- File: `--file query.json`
- stdin: piped JSON (fallback)
- Location: `src/query_lang.rs` - `TsQuery` deserialization, filter schema

**Output Formats (all JSON):**
- outline - name, kind, file, line, signature (default)
- snippet - outline + preview + docstring
- full - outline + full body
- tree - hierarchical (file → class → methods)
- locations - file:line only
- summary - counts by kind and file
- Wrapping: `{"project": "...", "query_ms": N, "files_parsed": N, "total": N, "results": [...]}`

**Pretty-print:**
- `--pretty` flag enables indentation (serde_json pretty printing)

## Language Grammar Sources

**tree-sitter Grammars (external repos, compiled into binary):**
- `tree-sitter-typescript` 0.23 - TypeScript, TSX, JavaScript, JSX
- `tree-sitter-javascript` 0.25 - JavaScript parsing (supplementary)
- `tree-sitter-c` 0.23 - C language
- `tree-sitter-cpp` 0.23 - C++ language
- `tree-sitter-c-sharp` 0.23 - C# language
- `tree-sitter-rust` 0.24 - Rust language
- `tree-sitter-python` 0.25 - Python language
- `tree-sitter-go` 0.25 - Go language
- `tree-sitter-java` 0.23 - Java language
- `tree-sitter-php` 0.24 - PHP language
- All grammars compiled into the binary; no runtime fetching

---

*Integration audit: 2026-04-16*
