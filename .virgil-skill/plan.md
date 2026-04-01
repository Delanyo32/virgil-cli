# Plan: Graph-Primary Audit Pipeline Architecture

## Config
Auto-commit: yes
Auto-proceed phases: no

## Desired State
Python audit pipelines use the CodeGraph as their primary analysis engine, not tree-sitter queries. The codebase has two explicit pipeline traits — `NodePipeline` for inherently per-node metrics (complexity, line counts) and `GraphPipeline` for everything else. Graph pipelines require a `&CodeGraph` (not `Option`), making graph availability a compile-time guarantee. The old `Pipeline` trait remains only for non-Python languages until they migrate.

### Criteria
- [ ] `NodePipeline` trait defined with `check(&self, tree, source, file_path) -> Vec<AuditFinding>`
- [ ] `GraphPipeline` trait defined with `check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding>` where `ctx.graph: &CodeGraph` (not Option)
- [ ] `AnyPipeline` enum wraps `Node`, `Graph`, and `Legacy` variants with shared `name()` accessor
- [ ] `AuditEngine::run()` dispatches correctly to all three variants
- [ ] 4 Python complexity pipelines implement `NodePipeline`: cyclomatic, function_length, cognitive, comment_ratio
- [ ] 9 already graph-filtered Python pipelines implement `GraphPipeline`: dead_code, coupling, missing_type_hints, memory_leak_indicators, n_plus_one_queries, sql_injection, path_traversal, api_surface_area, module_size_distribution
- [ ] 15 pure tree-sitter Python pipelines implement `GraphPipeline`: bare_except, mutable_default_args, magic_numbers, god_functions, stringly_typed, deep_nesting, duplicate_logic, duplicate_code, command_injection, code_injection, insecure_deserialization, ssrf, resource_exhaustion, xxe_format_string, sync_blocking_in_async
- [ ] Old `Pipeline` trait removed from all Python pipeline files
- [ ] Python dispatch functions (`tech_debt_pipelines()`, etc.) return `Vec<AnyPipeline>`
- [ ] Non-Python languages continue to work via `Legacy(Box<dyn Pipeline>)` with zero changes
- [ ] All existing tests pass (`cargo test`)
- [ ] No clippy warnings (`cargo clippy`)

## Phase 1: Introduce New Trait System + Engine Dispatch
Status: not-started
Goal: New traits exist, engine dispatches all three variants, all existing behavior unchanged

### Task 1.1: Define NodePipeline, GraphPipeline traits and AnyPipeline enum
Status: not-started
Change: In `src/audit/pipeline.rs`:
- Add `GraphPipelineContext<'a>` struct — same fields as `PipelineContext` but `graph: &'a CodeGraph` (required, not Option)
- Add `NodePipeline` trait: `name()`, `description()`, `check(tree, source, file_path) -> Vec<AuditFinding>`
- Add `GraphPipeline` trait: `name()`, `description()`, `check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding>`
- Add `AnyPipeline` enum with `Node(Box<dyn NodePipeline>)`, `Graph(Box<dyn GraphPipeline>)`, `Legacy(Box<dyn Pipeline>)` variants
- Impl `AnyPipeline::name() -> &str` that delegates to the inner trait's `name()`
- Keep existing `Pipeline` trait + `PipelineContext` untouched
Research:
- [code] `PipelineContext<'a>` has fields: tree, source, file_path, id_counts, graph (Option) (source: pipeline.rs:13-19)
- [code] `Pipeline` trait has: name(), description(), check(), check_with_ids(), check_with_context() (source: pipeline.rs:21-45)
- [code] Engine uses `Vec<Arc<dyn Pipeline>>` and calls `pipeline.check_with_context(&ctx)` (source: engine.rs:96-97, 173)
Test: `cargo build` — new types compile, no existing code broken

### Task 1.2: Update engine to dispatch AnyPipeline variants
Status: not-started
Change: In `src/audit/engine.rs`:
- Change `pipeline_map` value type from `Vec<Arc<dyn Pipeline>>` to `Vec<Arc<AnyPipeline>>`
- Update the per-file pipeline loop to match on `AnyPipeline` variants:
  - `Node(p)` → call `p.check(tree, source, file_path)` (no graph needed)
  - `Graph(p)` → if graph available, build `GraphPipelineContext` and call `p.check(&ctx)`, else skip
  - `Legacy(p)` → call `p.check_with_context(&ctx)` as before (existing PipelineContext with Option graph)
- Update pipeline dispatch functions to return `Vec<AnyPipeline>`:
  - For Python: will eventually return Node/Graph variants (Phase 2+)
  - For all other languages: wrap existing `Box<dyn Pipeline>` as `AnyPipeline::Legacy`
- Update `pipeline_filter` to work with `AnyPipeline::name()`
Research:
- [code] `pipelines_for_language()` returns `Vec<Box<dyn Pipeline>>` — needs to return `Vec<AnyPipeline>` (source: pipeline.rs:47-63)
- [code] 6 dispatch functions: pipelines_for_language, complexity_pipelines_for_language, code_style_pipelines_for_language, security_pipelines_for_language, scalability_pipelines_for_language, architecture_pipelines_for_language (source: pipeline.rs:47-247)
- [code] Engine filter: `lang_pipelines.retain(|p| self.pipeline_filter.contains(&p.name().to_string()))` (source: engine.rs:92)
- [code] Engine Arc wrapping: `lang_pipelines.into_iter().map(Arc::from).collect()` (source: engine.rs:96-97)
Test: `cargo test` — all 1971+ existing tests pass, behavior identical

### Task 1.3: Update pipeline.rs dispatch functions for all languages
Status: not-started
Change: In `src/audit/pipeline.rs`:
- Change all 6 `*_pipelines_for_language()` return types from `Vec<Box<dyn Pipeline>>` to `Vec<AnyPipeline>`
- For non-Python languages: wrap each `Box::new(pipeline)` as `AnyPipeline::Legacy(Box::new(...))`
- For Python: initially wrap as `AnyPipeline::Legacy` too (migration happens in Phase 2-4)
- This is a mechanical change — every existing pipeline gets wrapped in `Legacy()`
Test: `cargo test` + `cargo clippy` — all pass, no warnings

## Phase 2: Migrate Complexity Pipelines to NodePipeline + Already Graph-Filtered to GraphPipeline
Status: not-started
Goal: 4 complexity pipelines use NodePipeline, 9 graph-filtered pipelines use GraphPipeline, all return correct AnyPipeline variants

### Task 2.1: Migrate 4 complexity pipelines to NodePipeline
Status: not-started
Change: In `src/audit/pipelines/python/`:
- `cyclomatic.rs`: Replace `impl Pipeline for CyclomaticComplexityPipeline` with `impl NodePipeline for ...`; keep `check()` signature as-is (it already matches NodePipeline::check)
- `function_length.rs`: Same — `impl NodePipeline for FunctionLengthPipeline`
- `cognitive.rs`: Same — `impl NodePipeline for CognitiveComplexityPipeline`
- `comment_ratio.rs`: Same — `impl NodePipeline for CommentToCodeRatioPipeline`
- Update `complexity_pipelines()` in `python/mod.rs` to return `Vec<AnyPipeline>` with `AnyPipeline::Node(Box::new(...))`
Research:
- [code] CyclomaticComplexityPipeline::check() — only uses tree + source + file_path (source: cyclomatic.rs:67-116)
- [code] FunctionLengthPipeline::check() — only uses tree + source + file_path (source: function_length.rs:42-106)
- [code] CognitiveComplexityPipeline::check() — only uses tree + source + file_path (source: cognitive.rs:67-116)
- [code] CommentToCodeRatioPipeline::check() — only uses tree + source + file_path (source: comment_ratio.rs:50-97)
Test: `cargo test` — complexity pipeline tests pass, audit output identical

### Task 2.2: Migrate 9 already graph-filtered pipelines to GraphPipeline
Status: not-started
Change: In `src/audit/pipelines/python/`:
- For each of: dead_code, coupling, missing_type_hints, memory_leak_indicators, n_plus_one_queries, sql_injection, path_traversal, api_surface_area, module_size_distribution:
  1. Replace `impl Pipeline for XxxPipeline` with `impl GraphPipeline for XxxPipeline`
  2. Move `check_with_context()` logic into `GraphPipeline::check()`, changing `ctx.graph` from `Option<&CodeGraph>` to `&CodeGraph` (remove Option unwrapping/fallback)
  3. Remove old `check()` and `check_with_ids()` implementations (graph-only, no fallback)
  4. Remove any `if let Some(graph) = ctx.graph` guards — graph is always present
- Update corresponding dispatch functions in `python/mod.rs` to wrap as `AnyPipeline::Graph(Box::new(...))`
Research:
- [code] All 9 pipelines follow pattern: `check_with_context()` checks `ctx.graph.is_some()`, uses graph if available, falls back to `self.check()` otherwise (source: various python/*.rs)
- [code] Graph is now built in all 6 audit subcommands (Phase 1 of previous plan) — fallback path is dead code
- [code] `PipelineContext.graph: Option<&CodeGraph>` → `GraphPipelineContext.graph: &CodeGraph` — removes all `if let Some(graph)` boilerplate
Test: `cargo test` — all graph-aware pipeline tests pass, audit output identical

### Task 2.3: Update Python dispatch functions to return AnyPipeline
Status: not-started
Change: In `src/audit/pipelines/python/mod.rs`:
- `tech_debt_pipelines()` → return `Vec<AnyPipeline>`: all 8 as `Graph` (they'll be migrated in Phase 3, but for now keep as `Legacy` until individually migrated)
- `complexity_pipelines()` → return `Vec<AnyPipeline>`: all 4 as `Node`
- `code_style_pipelines()` → return `Vec<AnyPipeline>`: dead_code/coupling as `Graph`, duplicate_code as `Legacy` (migrated in Phase 4)
- `security_pipelines()` → return `Vec<AnyPipeline>`: sql_injection/path_traversal as `Graph`, rest as `Legacy` (migrated in Phase 4)
- `scalability_pipelines()` → return `Vec<AnyPipeline>`: n_plus_one/memory_leak as `Graph`, sync_blocking as `Legacy`
- `architecture_pipelines()` → return `Vec<AnyPipeline>`: both as `Graph`
- Update return types in function signatures
Test: `cargo test` + `cargo clippy`

## Phase 3: Migrate Tech-Debt Pure Tree-Sitter Pipelines to GraphPipeline
Status: not-started
Goal: 7 tech-debt pipelines (bare_except, mutable_default_args, magic_numbers, god_functions, stringly_typed, deep_nesting, duplicate_logic) implement GraphPipeline with graph-enhanced detection

Outline: For each pipeline, replace `impl Pipeline` with `impl GraphPipeline`. Rewrite `check()` to accept `GraphPipelineContext` and use graph for enhanced analysis:
- `bare_except`: use graph to check if caught exceptions flow from external/untrusted sources (higher severity)
- `mutable_default_args`: use graph to check cross-module callers (higher severity for widely-called functions)
- `magic_numbers`: use graph to check if the literal appears in multiple functions (should be a named constant)
- `god_functions`: use graph call edges to count outgoing calls (god function = many callees + many lines)
- `stringly_typed`: use graph to check if string-dispatched variables flow across function boundaries
- `deep_nesting`: use graph to check if deeply nested code is in a hot call path
- `duplicate_logic`: use graph call edges to detect functions with same signature AND same callees (true duplicates)

## Phase 4: Migrate Security, Style, and Scalability Pure Tree-Sitter Pipelines to GraphPipeline
Status: not-started
Goal: Remaining 8 pipelines implement GraphPipeline with graph-enhanced detection

Outline: Migrate the remaining pure tree-sitter pipelines:
- Security (6): command_injection, code_injection, insecure_deserialization, ssrf, resource_exhaustion, xxe_format_string — use graph taint analysis (FlowsTo/SanitizedBy edges) to suppress findings when inputs are proven safe
- Style (1): duplicate_code — use graph to detect cross-file duplicated logic (not just same-file)
- Scalability (1): sync_blocking_in_async — use graph to trace if blocking call is wrapped in executor or thread pool

## Phase 5: Remove Legacy Pipeline from Python + Cleanup
Status: not-started
Goal: No Python pipeline implements the old `Pipeline` trait. Old trait only used by non-Python languages.

Outline:
- Verify no Python pipeline files import or implement `Pipeline`
- Remove `Pipeline` import from all Python pipeline modules
- Clean up any dead helper functions that only served the old `check()` path
- Update Python `mod.rs` functions to assert no `Legacy` variants remain
- Run full test suite + clippy
- Consider: should `PipelineContext` (with Option graph) be removed if only `GraphPipelineContext` is used?
