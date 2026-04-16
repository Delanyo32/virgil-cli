# Phase 3: Tech Debt + Scalability JSON Migration - Context

**Gathered:** 2026-04-16
**Status:** Ready for planning

<domain>
## Phase Boundary

Migrate the 4 shared cross-language complexity pipelines (`cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`) and 2 shared scalability pipelines (`n_plus_one_queries`, `sync_blocking_in_async`) from Rust to declarative JSON definitions. Delete replaced Rust pipeline files. Add integration tests.

**Total JSON files to produce:** ~11 files (4 cross-language complexity + 1 cross-language n_plus_one_queries + ~6 per-language sync_blocking_in_async files for Rust, TypeScript, Python, Go, Java, C/C++/C#/PHP as applicable).

**TECH-02 is explicitly deferred:** Per-language tech-debt pipeline migrations (any_escape_hatch, type_assertions, unchecked_index_access, etc.) are NOT in this phase. They move to Phase 4 or a dedicated phase. Rationale: most TECH-02 pipelines are GraphPipeline candidates that cannot be fully expressed in the current JSON DSL.

**Requirements covered:** TECH-01, TECH-03, SCAL-01, TEST-01, TEST-02

</domain>

<decisions>
## Implementation Decisions

### TECH-02 Scope

- **D-01:** TECH-02 (per-language tech-debt pipeline migrations) is deferred out of Phase 3. Phase 3 covers only the 6 shared pipelines. TECH-02 will be addressed in Phase 4+ once the match_pattern and GraphPipeline JSON expressibility picture is clearer.

### compute_metric Pipeline Structure (TECH-01)

- **D-02:** The 4 complexity pipelines ship as **4 cross-language JSON files** — one per pipeline, no `"languages"` filter. `compute_metric` operates on graph symbol nodes that already carry language information; the stage itself is language-aware. No per-language splitting needed (unlike architecture pipelines which required language-specific graph stage logic).
  - `cyclomatic_complexity.json`
  - `function_length.json`
  - `cognitive_complexity.json`
  - `comment_to_code_ratio.json`

- **D-03:** Each complexity JSON pipeline must **explicitly filter by symbol kind** before calling `compute_metric`. Use `{"select": "symbol", "kind": ["function", "method", "arrow_function"]}` as the first stage. Do NOT rely on compute_metric to skip non-function nodes internally — explicit filtering is the established pattern and keeps executor logic clean.

- **D-04:** Thresholds follow `audit_plans/` specifications (authoritative). The current Rust defaults (CC > 10, function length, etc.) serve as the fallback only if audit_plans/ does not specify a threshold for a given metric.

### n_plus_one_queries Migration (SCAL-01)

- **D-05:** Migrate `n_plus_one_queries` to a **single cross-language JSON file** using `match_pattern` to detect the AST structure (loop + awaited call / loop + method call). Accept that the Rust version's ~30 hardcoded DB method name filters (findOne, axios.get, etc.) cannot be replicated by `match_pattern` (which is purely syntactic). **Document the precision delta inside the JSON file** in the `"description"` or a `"notes"` field so future maintainers know what was intentionally dropped.

- **D-06:** The match_pattern query for n_plus_one_queries should target the broadest structural pattern: a call expression (method call or function call) inside a loop body (for, forEach, map, while). The loss of DB-name specificity means a higher false positive rate vs. the Rust version — this is accepted and documented.

### sync_blocking_in_async Migration (SCAL-01)

- **D-07:** Migrate `sync_blocking_in_async` as **per-language JSON files** — one per language group. Each file uses a language-specific `match_pattern` query because the "blocking in async" pattern looks different per language:
  - Rust: `std::fs::*` / `std::net::*` calls inside `async fn` body
  - TypeScript/JavaScript: synchronous patterns inside `async function` / `async` arrow functions
  - Python: `time.sleep()` / synchronous calls inside `async def`
  - Go, Java, C#, PHP: per-language async construct detection

- **D-08:** Naming follows Phase 1 convention: `sync_blocking_in_async_{lang}.json` for each language group that has async support.

### Deletion Strategy

- **D-09:** Delete Rust pipeline files for each pipeline **in the same batch as the JSON replacement** — not in a separate cleanup step. Each plan unit: write JSON → verify cargo test passes → delete Rust → verify cargo test passes again.

### Test Strategy

- **D-10:** Same minimum as Phase 1: **1 positive + 1 negative case per pipeline** in `tests/audit_json_integration.rs`. Tests exercise the full `AuditEngine` path end-to-end. No additional boundary tests required (the threshold-boundary testing was considered and deferred — keep velocity consistent with prior phases).

### Claude's Discretion

- Which language groups get `sync_blocking_in_async` JSON files: start with languages that have clear async constructs (TypeScript, Python, Rust, Go). C, C++, PHP, Java files can be minimal/omitted if async detection isn't meaningful in that language — planner's judgment.
- Exact `match_pattern` S-expression queries for n_plus_one_queries and sync_blocking_in_async: derive from existing Rust queries in those pipeline files as starting templates.
- Whether `comment_to_code_ratio` needs a `select` kind filter or operates at file level: check the current Rust helper to determine if it's per-function or per-file, then structure the JSON accordingly.

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Tech Debt Audit Plans (per language)
These specs define what per-language complexity patterns should detect. For Phase 3, primarily useful for threshold values in the 4 shared complexity pipelines.
- `audit_plans/typescript_tech_debt.md` — TS complexity thresholds and pipeline analysis
- `audit_plans/python_tech_debt.md` — Python complexity thresholds
- `audit_plans/rust_tech_debt.md` — Rust complexity thresholds
- `audit_plans/go_tech_debt.md` — Go complexity thresholds
- `audit_plans/java_tech_debt.md` — Java complexity thresholds
- `audit_plans/c_tech_debt.md` — C complexity thresholds
- `audit_plans/cpp_tech_debt.md` — C++ complexity thresholds
- `audit_plans/csharp_tech_debt.md` — C# complexity thresholds
- `audit_plans/php_tech_debt.md` — PHP complexity thresholds

### Existing Rust Pipelines to Replace (read to understand current logic)
- `src/audit/pipelines/typescript/cyclomatic.rs` — Current CC implementation (function query + ControlFlowConfig)
- `src/audit/pipelines/typescript/function_length.rs` — Current function length logic
- `src/audit/pipelines/typescript/cognitive.rs` — Current cognitive complexity
- `src/audit/pipelines/typescript/comment_ratio.rs` — Current comment ratio
- `src/audit/pipelines/typescript/n_plus_one_queries.rs` — Rust n_plus_one with hardcoded DB names (read before writing match_pattern replacement)
- `src/audit/pipelines/typescript/sync_blocking_in_async.rs` — TypeScript sync_blocking
- `src/audit/pipelines/rust/sync_blocking_in_async.rs` — Rust-language sync_blocking (std::fs patterns)
- `src/audit/pipelines/python/sync_blocking_in_async.rs` — Python sync_blocking (if exists)

### Phase 2 Context (executor stage decisions)
- `.planning/phases/02-executor-stage-implementation/02-CONTEXT.md` — D-01 through D-09: exact DSL shapes for match_pattern and compute_metric stages, workspace parameter decision

### Existing JSON Templates
- `src/audit/builtin/api_surface_area_typescript.json` — Phase 1 template for graph-stage DSL
- Any existing `src/audit/builtin/*.json` with `match_pattern` usage (check if Phase 2 added any during testing)

### Engine and Helpers
- `src/graph/executor.rs` — `run_pipeline()` with workspace parameter; `execute_stage()` dispatch for MatchPattern and ComputeMetric
- `src/graph/metrics.rs` — `compute_cyclomatic`, `compute_cognitive`, `count_function_lines`, `compute_comment_ratio` (moved from helpers.rs in Phase 2)
- `src/audit/pipelines/helpers.rs` — Re-exports from graph::metrics for backward compat; check what's still referenced

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `src/graph/metrics.rs`: All 4 compute_metric helper functions now live here (moved from helpers.rs in Phase 2). The JSON executor calls these directly.
- `src/audit/builtin/*.json` (36 files from Phase 1): Graph-stage DSL templates — use as structural reference for the new complexity JSON files.
- `tests/audit_json_integration.rs`: Existing integration test file; add Phase 3 tests here following same pattern as Phase 1 tests.
- `src/audit/pipelines/*/cyclomatic.rs` (9 language files): Each has a FUNCTION_QUERY constant with the tree-sitter S-expression for finding functions in that language — valuable as starting templates for match_pattern queries.

### Established Patterns
- JSON pipeline format: `pipeline` (name), `category`, `description`, `languages` (optional filter), `graph` (stages array)
- compute_metric DSL (Phase 2 decision D-07): `{"compute_metric": "cyclomatic_complexity"}`
- match_pattern DSL (Phase 2 decision D-04): `{"match_pattern": "(tree_sitter_query_string)"}`
- Preceding select before compute_metric: `{"select": "symbol", "kind": ["function", "method", "arrow_function"]}`
- include_dir! auto-discovery: new JSON files in `src/audit/builtin/` are picked up automatically at compile time — no changes to json_audit.rs required

### Integration Points
- New JSON files land in `src/audit/builtin/` — automatically discovered
- Rust pipeline files to delete are in `src/audit/pipelines/{lang}/` subdirectories
- Rust `mod.rs` files in each language directory may need updating after deletion (remove module declarations)
- `src/audit/pipelines/helpers.rs` re-exports from graph::metrics — after all Rust pipeline files using helpers.rs are deleted, check if helpers.rs itself becomes dead code (Phase 5 cleanup, but note it here)

</code_context>

<specifics>
## Specific Ideas

- For `comment_to_code_ratio`: the current Rust implementation may be per-file (not per-function). If `compute_metric` operates on symbol nodes, this pipeline may need a different structure — possibly `match_pattern` at file level or a `file` select stage. **Check the current Rust helpers.rs `compute_comment_ratio` signature** before designing the JSON.
- The `n_plus_one_queries.rs` TypeScript file has the most complete list of DB method patterns — use it as the reference for what the JSON match_pattern query is "approximating". Include a comment in the JSON description listing the dropped patterns (findOne, findUnique, findMany, axios.get/post/put/delete, etc.) so the delta is visible.
- For `sync_blocking_in_async`, the Rust-language pipeline (`src/audit/pipelines/rust/sync_blocking_in_async.rs`) uses scoped path matching (`std::fs::`, `std::net::`) which match_pattern can approximate with `(scoped_identifier) @id` + text matching — but match_pattern doesn't support text predicates like `(#match? @id "^std::fs")`. May need to emit findings for ALL scoped calls inside async fns and accept false positives, or document as a known limitation.

</specifics>

<deferred>
## Deferred Ideas

- **TECH-02: Per-language tech-debt migrations** — all 11 TypeScript pipelines (any_escape_hatch, type_assertions, unchecked_index_access, etc.) and equivalent pipelines for 8 other language groups. Deferred because most require GraphPipeline (cross-file graph data) that the current JSON DSL cannot express. Target: Phase 4 or a dedicated phase after GraphPipeline JSON support is assessed.
- **SCAL-02: Per-language scalability pipelines** — already in Phase 4 per roadmap.
- **Threshold boundary tests** — adding a third "exactly at threshold" test case per compute_metric pipeline was considered and deferred to keep test velocity consistent with Phase 1.
- **match_pattern text predicates** (e.g., `#match?`, `#eq?` for filtering by identifier name) — would enable more faithful n_plus_one_queries detection (filter by DB method names). Currently unclear if the match_pattern executor supports predicates. If Phase 2 added predicate support, revisit n_plus_one_queries fidelity. If not, this is a future engine enhancement.

</deferred>

---

*Phase: 03-tech-debt-scalability-json-migration*
*Context gathered: 2026-04-16*
