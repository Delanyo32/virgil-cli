# Technology Stack

**Analysis Date:** 2026-04-16

## Languages

**Primary:**
- Rust 2024 edition - CLI application, core parsing engine, AST analysis, audit pipelines

## Runtime

**Environment:**
- Rust toolchain (stable)
- Multi-platform: Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64)

**Package Manager:**
- Cargo
- Lockfile: `Cargo.lock` (present)

## Frameworks

**Core:**
- tree-sitter 0.25 - AST parsing for 13 programming languages
- clap 4.5 - CLI argument parsing with `derive` macros

**HTTP Server:**
- axum 0.8 - Async HTTP server for persistent query/audit API
- tokio 1 - Async runtime with multi-threaded executor

**Analysis & Graphs:**
- petgraph 0.7 - Control flow graphs and call graphs
- rayon 1.11 - Parallel file parsing and filter pipelines

**Utilities:**
- serde + serde_json 1 - JSON serialization (queries, audit results, project registry)
- regex 1 - Name matching filters
- globset 0.4 - Glob pattern matching for file discovery and exclusion
- ignore 0.4 - .gitignore-aware file discovery
- streaming-iterator 0.1 - tree-sitter QueryMatches streaming
- indicatif 0.17 - Progress bars for CLI output
- dirs 5 - Platform-aware home directory detection
- chrono 0.4 - Timestamps in project registry and metadata
- anyhow 1.0 - Error handling

**Cloud Storage:**
- aws-sdk-s3 1 - S3/R2/MinIO client
- aws-config 1 - AWS credential chain and configuration

## Key Dependencies

**Critical:**
- tree-sitter v0.25 with language grammars - Powers all symbol extraction and AST analysis. Must not be downgraded due to `QueryMatches` API changes.
- clap - CLI definition and dispatch
- axum + tokio - HTTP server for persistent query mode
- petgraph - Call graph and control flow graph construction
- rayon - Parallelism for file discovery and per-file analysis

**Infrastructure:**
- aws-sdk-s3 - S3-compatible storage access (AWS S3, Cloudflare R2, MinIO)
- globset - File filtering with glob patterns
- ignore - .gitignore compatibility

**Secondary:**
- serde_json - All output is JSON (queries, audit results, registry)
- regex - Pattern matching in symbol name filters
- streaming-iterator - tree-sitter streaming API requirement

## Configuration

**Environment:**
- `.env` file present - contains environment variables (note: never read contents)
- AWS/S3 credentials via env vars:
  - `S3_ACCESS_KEY_ID` or `AWS_ACCESS_KEY_ID` (access key)
  - `S3_SECRET_ACCESS_KEY` or `AWS_SECRET_ACCESS_KEY` (secret key)
  - `S3_ENDPOINT` or `AWS_ENDPOINT_URL` (custom endpoint for R2, MinIO)
  - `AWS_REGION` (defaults to "auto" for R2 compatibility)
  - Fallback: standard AWS credential chain (`~/.aws/credentials`, IAM roles)

**Build:**
- `Cargo.toml` - Single workspace, 13 language tree-sitter parsers as dependencies
- Release profile (`[profile.release]`):
  - `strip = true` - Binary stripping for size
  - `lto = "thin"` - Link-time optimization
  - `opt-level = 3` - Full optimization
  - `codegen-units = 1` - Single codegen unit for optimization
- Binary installer metadata: `[package.metadata.binstall]` - Cargo binstall support for precompiled releases

## Platform Requirements

**Development:**
- Rust stable toolchain (tested on ci.yml)
- Platform-specific: Ubuntu, macOS, Windows CI runners
- Dependencies: tree-sitter C/C++ libraries compiled per-platform

**Production:**
- Deployment: Standalone binary (pre-built for Linux/macOS/Windows, x86_64/aarch64)
- Storage: Local filesystem (project registry at `~/.virgil-cli/projects.json`) OR S3-compatible bucket
- HTTP: Optional persistent server mode (tokio async, no external process manager needed)
- Memory: In-memory file loading (all project files loaded at startup into `MemoryFileSource`)

## External Services

**S3-Compatible Storage:**
- AWS S3 - Primary cloud storage option
- Cloudflare R2 - Supported via custom endpoint
- MinIO - Supported via custom endpoint
- Optional: queries and audits can read directly from S3 without local registration

---

*Stack analysis: 2026-04-16*
