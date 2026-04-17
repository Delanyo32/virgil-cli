# virgil-cli

Rust CLI tool that parses TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebases on-demand and queries them with a composable JSON query language. No database, no pre-indexing — projects are registered by name and parsed at query time. Supports S3-compatible storage (AWS S3, Cloudflare R2, MinIO) via `--s3 s3://bucket/prefix`.

## Build & Run

```bash
cargo build
cargo run -- projects create myapp --path ./src [--lang ts,tsx,js,jsx] [--exclude "vendor/**"]
cargo run -- projects list
cargo run -- projects delete myapp
cargo run -- projects query myapp --q '{"find": "function", "name": "handle*"}' [--out outline|snippet|full|tree|locations|summary] [--pretty]
cargo run -- projects query myapp --file query.json
# S3/R2 (no registration needed)
cargo run -- projects query --s3 s3://bucket/prefix --q '{"find": "function"}' [--lang rs]
cargo run -- audit --dir ./src [--language rs] [--category security] [--pipeline sql_injection_rust]
cargo run -- audit --s3 s3://bucket/prefix [--language rs]
# Serve mode
cargo run -- serve --s3 s3://bucket/prefix [--host 127.0.0.1] [--port 0] [--lang rs]
```

## Module Layout

- `src/pipeline/` — JSON pipeline layer (single owner of the DSL, executor, and file loading)
  - `dsl.rs` — `GraphStage`, `WhereClause`, `PipelineNode` and all DSL types
  - `executor.rs` — `run_pipeline` execution engine
  - `loader.rs` — `discover_json_audits` (project-local → user-global → built-ins)
  - `helpers.rs` — `is_test_file`, `is_barrel_file`, `is_excluded_for_arch_analysis`
- `src/audit/` — orchestration and output only
  - `engine.rs` — `AuditEngine` (discovers + runs JSON pipelines, collects findings)
  - `format.rs` — finding output formatting (table/json/csv)
  - `models.rs` — `AuditFinding`, `AuditSummary`
  - `project_index.rs` — `ProjectIndex` (used by `graph/mod.rs` compat methods)
- `src/graph/` — graph data structures and builder
  - `mod.rs` — `CodeGraph`, `NodeWeight`, `EdgeWeight`
  - `builder.rs` — `GraphBuilder` (parses workspace into `CodeGraph`)
  - `taint.rs` — `TaintEngine`, `TaintConfig` (internal engine used by `pipeline/executor.rs`)
  - `metrics.rs` — metric computation (cyclomatic complexity, function length, etc.)
  - `cfg.rs` / `cfg_languages/` — control flow graph construction

## JSON Query Language

Queries are JSON objects. All fields are optional and composable:

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
  "read": "src/main.rs"
}
```

- `name`: glob string, `{"contains": "auth"}`, or `{"regex": "^get[A-Z]"}`
- `has`: string, array, or `{"not": "docstring"}` for inverse (symbols *without* doc comments)
- `calls`: `"down"` (callees), `"up"` (callers), `"both"`; `depth` default 1, max 5
- `read`: returns raw file content instead of symbols; combine with `lines` for range reads
- `find: "function"` matches both `Function` and `ArrowFunction` kinds
- `find: "constructor"` matches `Method` kind (post-filtered by name: "constructor", "__init__", "__construct", "new")

## Non-obvious Implementation Notes

Critical gotchas and design decisions that are not obvious from reading the code:

**tree-sitter 0.25 API (do not downgrade)**
`QueryMatches` uses `streaming_iterator::StreamingIterator`, not `std::iter::Iterator`. Iterate with `while let Some(m) = matches.next()`. Downgrading tree-sitter breaks this.

**Threading constraints**
- `tree_sitter::Parser` is `!Send` — create a fresh instance per rayon task (never share or pool)
- `tree_sitter::Query` objects are `Arc`-shareable — compile once per language, share across threads
- `CodeGraph` (`petgraph::DiGraph`) is `Send` but not `Sync` — share via `Arc<CodeGraph>` for read-only access

**File extension mapping**
- `.h` files map to C (deliberate design choice). C++ headers must use `.hpp`/`.hxx`/`.hh`
- PHP grammar uses `LANGUAGE_PHP` (handles `<?php` tags), not `LANGUAGE_PHP_ONLY`

**Query behavior quirks**
- `find: "function"` matches `Function` AND `ArrowFunction` — both are returned
- `find: "constructor"` matches `Method` kind filtered by name (not a separate kind)
- `inside` filter uses line-range containment, not AST hierarchy
- `read` field bypasses the entire symbol extraction pipeline — returns raw content
- `has: {"not": "docstring"}` finds symbols *without* associated doc comments (inverse match)
- Import `kind` is a free-form `String` (not an enum) so language modules can add new kinds without changing `models.rs`

**Python parsing**
- `decorated_definition` nodes: unwrap to inner function/class; skip the bare `function_definition`/`class_definition` if its parent is a `decorated_definition`. This deduplication prevents double-reporting decorated symbols.

**Audit pipelines**
- `PipelineContext` and `GraphPipelineContext` are deleted — all analysis goes through `src/pipeline/executor.rs` via `run_pipeline`. The executor handles `select`, `compute_metric`, `taint_sources`/`taint_sanitizers`/`taint_sinks`, `flag`, and other stages directly.
- `WhereClause.metrics` is a `HashMap<String, NumericPredicate>` — metric predicates use `{"metrics": {"name": {...}}}` nesting, not flat named fields.
- `taint_sources` / `taint_sanitizers` / `taint_sinks` accumulate into a `TaintContext` per pipeline run. The old `taint` combined form desugars automatically.
- Architecture audit thresholds (not in JSON files): oversized_module ≥ 30 symbols OR ≥ 1000 lines; monolithic_export_surface ≥ 20 exports; barrel_file_reexport ≥ 5 re-exports; hub_module_bidirectional ≥ 5 intra-project imports; deep_import_chain ≥ 4 path depth; excessive_public_api ≥ 10 symbols AND >80% exported.

**Call graph**
Name-based resolution via `symbols_by_name` lookup — heuristic only, no type info. BFS with configurable depth (max 5). Replaces old `call_graph.rs`.

**S3 workspace**
- S3 workspace root is a synthetic `s3://bucket/prefix` path. `execute_read()` disk fallback is guarded by `root.exists()` to prevent filesystem access on S3 workspaces.
- `--s3` flag conflicts with positional `name`/`dir` args via `#[arg(conflicts_with)]`
- Server mode (`serve`) is S3-only — no `--path` flag. Used by Virgil Live (cloud service).

**Audit pipeline model (JSON-first)**
All audit logic is JSON-driven. `src/pipeline/` owns the DSL, executor, and builtin file loading.
`AuditEngine` in `src/audit/engine.rs` discovers JSON files and calls `run_pipeline`.
No Rust pipeline code exists — `audit/pipeline.rs`, `audit/pipelines/`, and the legacy trait
hierarchy (`Pipeline`, `NodePipeline`, `GraphPipeline`) have been deleted.

**DSL composability**
`WhereClause` uses a generic `metrics: HashMap<String, NumericPredicate>` field — any metric
computed by a `compute_metric` stage is filterable without changing the Rust schema:
```json
{"when": {"metrics": {"cyclomatic_complexity": {"gte": 15}}}}
```
The `taint` stage is decomposed into `taint_sources` + `taint_sanitizers` + `taint_sinks`
stages that accumulate into a shared context. The old combined `taint` form continues to work
(desugared by the executor) for backward compatibility with external pipeline files.
Use `AuditEngine::categories(vec!["security".to_string()])` to filter by category.

**Audit CLI**
`virgil audit [--dir|--s3] [--language] [--category] [--pipeline] [--format] [--per-page] [--page]`
No nested subcommands. Category values match the `category` field in JSON pipeline files:
`security`, `architecture`, `code_style`, `tech_debt`, `complexity`, `scalability`.

<!-- GSD:skills-start source:skills/ -->
## Project Skills

| Skill | Description | Path |
|-------|-------------|------|
| grill-me | Interview the user relentlessly about a plan or design until reaching shared understanding, resolving each branch of the decision tree. Use when user wants to stress-test a plan, get grilled on their design, or mentions "grill me". | `.agents/skills/grill-me/SKILL.md` |
| skill-creator | Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Claude's capabilities with specialized knowledge, workflows, or tool integrations. | `.agents/skills/skill-creator/SKILL.md` |
| virgil | > Explore and query codebases using virgil-cli. Covers project registration, JSON query language for symbol search, call graph traversal, file reading, and static audit. Use when asked to analyze a codebase, understand architecture, find symbols, trace callers/callees, onboard to a project, investigate bugs, or map the API surface of any TypeScript/JavaScript/C/C++/C#/Rust/Python/Go/Java/PHP codebase. | `.agents/skills/virgil/SKILL.md` |
<!-- GSD:skills-end -->

<!-- GSD:workflow-start source:GSD defaults -->
## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd-quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd-debug` for investigation and bug fixing
- `/gsd-execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

<!-- GSD:profile-start -->
## Developer Profile

> Profile not yet configured. Run `/gsd-profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
