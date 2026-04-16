# Project Research Summary

**Project:** virgil-cli — JSON Audit Pipeline Migration
**Domain:** Declarative static-analysis pipeline DSL migration (Rust to JSON)
**Researched:** 2026-04-16
**Confidence:** HIGH

## Executive Summary

virgil-cli already has a dual-path audit engine: Rust pipelines in `src/audit/pipelines/` run per-file via rayon, and JSON pipelines in `src/audit/builtin/` run graph-only via the `CodeGraph` executor. Four architecture pipelines (module_size_distribution, api_surface_area, circular_dependencies, dependency_graph_depth) are fully migrated and working. The research goal was to determine how many of the remaining ~298 Rust pipeline files can be converted to JSON, in what order, and what engine changes are prerequisites. The answer is more constrained than naive file counts suggest.

The central tension in the research is this: ARCHITECTURE.md identifies 10 "shared" pipeline names (cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio, dead_code, duplicate_code, coupling, n_plus_one_queries, sync_blocking_in_async, memory_leak_indicators) and claims they are high-ROI JSON targets. FEATURES.md and PITFALLS.md both confirm this is wrong -- the JSON executor is graph-only and has no access to the per-file tree-sitter AST. These 10 pipelines are all per-symbol AST computations, not graph traversals. They cannot be expressed in the current JSON format without first adding a `match_pattern` stage and a `compute_metric` stage to the executor. Attempting to migrate them now would produce pipelines that silently emit zero findings (Pitfall 4: JSON pipelines only execute when `graph: Some`), which is worse than the status quo.

The immediately safe migration scope is narrower: the architecture category has 4 pipeline patterns per language (9 supported languages = 36 JSON files) and those pipelines ARE graph-based, matching the engine's current capabilities. In parallel, four prerequisite engine fixes are needed before anything else can migrate: (1) add per-file Rust pipeline suppression to `engine.rs` to prevent doubled findings, (2) fix the hardcoded `include_str!` list in `json_audit.rs` so new files do not need manual registration, (3) implement the `match_pattern` stage in the executor, and (4) implement the `compute_metric` stage. Once those two stages exist, the 10 shared-name pipelines become viable JSON targets and the total migration scope expands to roughly 220 of 298 remaining pipelines.

---

## Key Findings

### Recommended Stack

The Rust + JSON format combination is already chosen and operational. Research confirmed the format is well-designed relative to industry alternatives: it is more powerful than Semgrep or ast-grep for cross-file aggregate analysis (group_by, find_cycles, max_depth, ratio stages have no equivalents in YAML-based tools), more portable than CodeQL (no compilation step, ships inside the binary), and more extensible than SonarQube (no Java plugin needed). The `GraphStage` untagged serde enum design is sound and should be preserved. The `severity_map` with `when` clauses is better-designed threshold escalation than Semgrep's flat severity fields.

**Core components:**
- `src/graph/pipeline.rs`: GraphStage enum -- the DSL vocabulary. Needs `match_pattern` and `compute_metric` variants added.
- `src/audit/json_audit.rs`: built-in discovery via `include_str!`. Needs dynamic discovery (include_dir! or fs::read_dir) to eliminate manual registration per new file.
- `src/audit/engine.rs`: dual-path executor. Needs one-line `retain` call to extend JSON suppression from ProjectAnalyzers to all per-file pipelines.
- `src/graph/executor.rs`: stage execution. Has five stub stages (`traverse`, `filter`, `match_name`, `count_edges`, `pair`) that silently pass nodes through -- these must not be used until implemented.

### Expected Features (Migration Scope)

**Immediately migratable (engine already supports):**
- All per-language architecture pipeline variants -- `select -> group_by -> count/ratio/find_cycles/max_depth -> flag` patterns, language-scoped via `languages` field. Approximately 36 JSON files covering 9 language-specific variants of the 4 existing architecture patterns.
- `coupling` ProjectAnalyzer (the one remaining cross-file analyzer not yet migrated) -- JSON override mechanism already suppresses ProjectAnalyzers by name.

**Migratable after engine extensions (Phase 2 prerequisite):**
- `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio` once `compute_metric` stage is added.
- `n_plus_one_queries`, `sync_blocking_in_async`, `dead_code`, `coupling` (per-file), and most tech-debt pipelines once `match_pattern` stage is added.
- Estimated 220 of 298 remaining pipeline files become migratable with these two stage additions.

**Stay in Rust permanently:**
- `duplicate_code` -- hash-based cross-subtree similarity; no declarative equivalent.
- `memory_leak_indicators` -- resource lifecycle path sensitivity via CFG.
- Security pipelines using taint propagation (FlowsTo/SanitizedBy path traversal).
- `callback_hell` and recursive AST nesting depth accumulation.
- Pipelines requiring struct field visibility inspection (graph does not store struct fields as Symbol nodes).
- Return type signature inspection (function return type is a string in the graph, not a typed AST node).

**Defer indefinitely:**
- Project-relative threshold computation (mean + N*stddev requires two-pass execution model).
- Full CFG-level data flow for security analysis.

### Architecture Approach

The migration is architecturally additive-then-subtractive: add JSON file, then delete Rust file. The engine already supports dual-path execution, so no structural changes to the pipeline runner are needed beyond the suppression fix. The key insight from the research is that the JSON override mechanism (which suppresses ProjectAnalyzers by pipeline name) does NOT extend to per-file pipelines registered via `pipelines_for_language()`. This is the single most important engine gap. Without the suppression fix, every pipeline migration is unsafe unless the Rust file is deleted atomically in the same commit.

**Major components in scope:**
1. `engine.rs` suppression fix -- one `retain` call: `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))`. Prevents doubled findings during any overlap window.
2. `json_audit.rs` include_str gap -- every new JSON builtin currently requires a manual Rust code edit. Switch to `include_dir!` macro or per-file `include_str!` additions in the same commit as the JSON file.
3. `executor.rs` stage completions -- `match_pattern` and `compute_metric` are new; `traverse`, `filter`, `match_name`, `count_edges` are stubs that must either be implemented or changed to return loud errors.
4. Per-language architecture JSON files -- 36 files, safe to write now against the current engine.

### Critical Pitfalls

1. **Doubled findings from unsuppressed Rust pipeline** (Pitfall 1) -- The JSON override only covers ProjectAnalyzers. Any JSON file added without atomically deleting the corresponding Rust file produces duplicate findings with no engine error and no test failure. Fix: add the one-line `retain` suppression to `engine.rs` in Phase 1, then atomic PR is still required but the engine provides a safety net during PR review.

2. **JSON pipeline silently produces zero findings because graph is None** (Pitfall 4) -- The JSON executor only runs when `graph: Some`. Tech-debt, complexity, and code-style CLI paths historically ran without constructing a CodeGraph. A migrated pipeline for those categories will appear to succeed but emit nothing. This is the reason the 10 shared-name pipelines CANNOT be migrated until Phase 3 (after verifying the CLI constructs a CodeGraph for those audit paths).

3. **Stub executor stages produce silent wrong output** (Pitfall 3) -- Five `GraphStage` variants are stubs that silently pass nodes through unchanged. A pipeline that uses `traverse` as a filter will flag every node; one that uses it for expansion will produce zero findings. No error is emitted. Mitigation: change stubs to `Err(...)` in Phase 1 so failures are loud, and document which stages are safe to use.

4. **`include_str!` registration gap** (Pitfall 7) -- A JSON file added to `src/audit/builtin/` without updating `builtin_audits()` is silently never loaded. The test `test_builtin_audits_returns_four` will fail only if someone remembers to update the count. Mitigation: switch to `include_dir!` in Phase 1.

5. **Test coverage black hole from Rust deletion** (Pitfall 5) -- 2,205 `#[test]` functions disappear atomically when Rust files are deleted. `cargo test` passes with no signal that coverage collapsed. Mitigation: write integration tests in `tests/audit_json_integration.rs` in the same PR as each Rust deletion, minimum one positive-case and one negative-case test per pipeline name.

---

## Implications for Roadmap

### Phase 1: Engine Fixes and Architecture Language Expansion

**Rationale:** The architecture category is proven at the format level. The immediate expansion is per-language architecture JSON files (9 languages x 4 patterns). This is safe now with zero engine changes. In the same phase, fix the two engine bugs that make all future migration unsafe: the doubled-findings suppression gap (engine.rs retain) and the include_str! manual registration requirement (json_audit.rs). Make the five stub stages loudly fail. Add the integration test scaffolding.

**Delivers:** 36 new JSON architecture pipeline files; suppression fix in `engine.rs`; dynamic loading fix in `json_audit.rs`; stub stages changed to Err; integration test harness in `tests/audit_json_integration.rs`.

**Addresses:** Architecture coverage across all 9 languages; the two critical engine bugs blocking safe migration.

**Avoids:** Pitfall 1 (doubled findings), Pitfall 3 (silent stub pass-through), Pitfall 7 (include_str gap), Pitfall 5 (test black hole).

**Scope note on architecture patterns:** The `leaky_abstraction_boundary` pattern (exported types with public struct fields) cannot be expressed in JSON because the graph does not store struct fields as Symbol nodes. Omit it from Phase 1 JSON files and document as known regression.

**Research flag:** Standard patterns -- no deeper research needed. Language variants are mechanical language-filter additions to proven pipeline structures.

### Phase 2: Engine Stage Extensions (match_pattern + compute_metric)

**Rationale:** The 10 shared-name pipelines (cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio, dead_code, duplicate_code, coupling, n_plus_one_queries, sync_blocking_in_async, memory_leak_indicators) and virtually all tech-debt pipelines are blocked on two missing executor stages. These stages are the prerequisite for roughly 220 of the remaining 298 Rust files. Building them before writing any of those pipelines avoids the risk of writing JSON pipelines that silently produce wrong output. This phase is pure engine work with no JSON pipeline authoring.

**Delivers:** `match_pattern` GraphStage variant + executor implementation (tree-sitter Query against file's AST, one PipelineNode per match); `compute_metric` stage for cyclomatic/cognitive/function_length/comment_ratio (calls existing helpers in `src/audit/pipelines/helpers.rs`); `kind` predicate added to WhereClause; verification that CLI audit paths for tech-debt/complexity/code-style construct a CodeGraph.

**Addresses:** The single biggest format gap (per-symbol AST pattern matching has no JSON equivalent today).

**Avoids:** Pitfall 4 (graph: None produces zero findings for AST-based pipelines -- CLI construction must be verified).

**Research flag:** Needs focused planning. The `match_pattern` stage must access the parsed `Tree` and raw source bytes from `PipelineContext` -- verify this context is available in the executor before writing the implementation. The `compute_metric` helpers in `src/audit/pipelines/helpers.rs` must be accessible from `src/graph/executor.rs` (cross-module reference -- verify no circular dependency).

### Phase 3: Shared Cross-Language Pipeline Migration (10 names, ~100 Rust files removed)

**Rationale:** With `match_pattern` and `compute_metric` stages implemented, the 10 shared pipeline names become expressible in JSON. Each JSON file covers all 10 languages via no `languages` filter (or explicit list). This is the highest ROI phase: 10 JSON files remove ~100 Rust files. The suppression fix from Phase 1 provides a safety net during the validation window before deletion.

**Delivers:** 10 JSON pipeline files; deletion of ~100 Rust pipeline files; deletion of ~2,000 Rust unit tests replaced by integration tests.

**Addresses:** Complexity category (4 pipelines), code-style category (dead_code and coupling per-file; duplicate_code stays in Rust), scalability category (n_plus_one_queries, sync_blocking_in_async, memory_leak_indicators).

**Exception:** `duplicate_code` stays as Rust. Do not create a JSON stub for it.
**Exception:** `memory_leak_indicators` requires resource lifecycle path sensitivity -- stays as Rust.

**Avoids:** Pitfall 1 (suppression fix as safety net), Pitfall 5 (integration tests written before Rust deletion), Pitfall 6 (behavior regression -- compare against audit_plan specs), Pitfall 8 (language filter -- set `languages` explicitly or omit for true cross-language pipelines).

**Research flag:** Standard patterns after Phase 2. The per-file `coupling` pipeline is distinct from the `CouplingAnalyzer` ProjectAnalyzer -- verify the Rust pipeline name matches before writing the JSON.

### Phase 4: Language-Specific Tech-Debt and Security Migration

**Rationale:** With the engine fully capable and shared infrastructure proven, language-specific tech-debt and security pipelines can migrate language by language, fully parallelizable across contributors. Security pipelines using taint propagation remain in Rust permanently.

**Delivers:** Per-language tech-debt JSON files (7-12 per language, 9 languages); per-language security JSON files for non-taint patterns; deletion of corresponding Rust files.

**Order within phase:** PHP and JavaScript first (simpler detection logic), then TypeScript, C, C++, Java, C#, Go, and Rust/Python last (most complex, most graph-dependent).

**Deferred permanently:** Taint-propagation security pipelines, duplicate_code, memory_leak_indicators, callback_hell, return-type-inspection pipelines.

**Research flag:** Each language's tech-debt spec is in `audit_plans/{language}_architecture.md`. The Rust tech-debt pipelines (panic_detection, async_blocking) are described as requiring CFG-level guard awareness -- verify whether a simplified `match_pattern` approximation achieves acceptable parity before committing to full migration. Python and Go pipelines are `GraphPipeline` implementations -- verify graph prerequisites.

### Phase Ordering Rationale

- Phase 1 before everything: fixes the two engine bugs that make any migration unsafe, and delivers immediately visible value (complete architecture coverage across all languages).
- Phase 2 before Phase 3: writing JSON for AST-based pipelines without the execution stages produces silent failures. Silent failures are worse than the current Rust implementations.
- Phase 3 before Phase 4: shared-name pipelines establish format conventions and test patterns. Contributors authoring Phase 4 need working references.
- Phase 4 is internally parallelizable at the language level -- no cross-language merge conflicts.

### Research Flags

Phases needing deeper research during planning:
- **Phase 2:** Verify `PipelineContext` carries the parsed `Tree` and raw source bytes needed by `match_pattern`. Verify CLI entry points for tech-debt/complexity/code-style audit paths construct a `CodeGraph` before relying on JSON pipelines for those categories.
- **Phase 4 (Rust/Python tech-debt):** Verify whether simplified `match_pattern` versions of CFG-dependent pipelines achieve parity, or whether those pipelines must stay in Rust.

Phases with standard patterns:
- **Phase 1:** Architecture pipeline language variants are mechanical; format is proven.
- **Phase 3:** After Phase 2 stage additions, shared-name pipelines follow the template from Phase 1.

---

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Directly read from source: pipeline.rs, executor.rs, json_audit.rs, engine.rs. No inference. |
| Features | HIGH | All 4 builtin/*.json files read; all executor stage variants inventoried; audit_plans read for detection logic. |
| Architecture | HIGH | Dual-path engine structure confirmed from source. Suppression asymmetry confirmed from engine.rs line numbers. |
| Pitfalls | HIGH | All pitfalls derived from direct source inspection. Stub stages confirmed from executor.rs TODO comments. |

**Overall confidence:** HIGH

### Gaps to Address

- **compute_metric helper accessibility:** STACK.md asserts that `compute_cyclomatic`, `compute_cognitive`, etc. exist in `src/audit/pipelines/helpers.rs`. Verify these are accessible from `src/graph/executor.rs` without circular module dependency before Phase 2 planning.
- **CLI CodeGraph construction for non-architecture audit paths:** Verify in `main.rs` that tech-debt/complexity/code-style CLI paths construct a CodeGraph. If they do not, JSON pipelines silently produce zero findings for those categories and the CLI must be updated before Phase 3.
- **coupling ProjectAnalyzer name mismatch risk:** The `CouplingAnalyzer`'s `name()` return value must be verified against the intended JSON pipeline name before Phase 1 includes it. A mismatch means the ProjectAnalyzer continues running alongside the JSON pipeline.
- **leaky_abstraction_boundary omission:** This architecture pattern requires struct field visibility inspection, which the graph does not support. Omit from Phase 1 JSON files. Document as known regression in PR.

---

## Sources

### Primary (HIGH confidence -- direct source inspection)
- `src/audit/engine.rs` -- dual-path execution, suppression mechanism scope
- `src/audit/json_audit.rs` -- include_str! builtin loading, JsonAuditFile schema
- `src/graph/pipeline.rs` -- GraphStage enum, WhereClause predicates, all stage config structs
- `src/graph/executor.rs` -- stage implementations and stub TODO comments
- `src/audit/builtin/*.json` -- all 4 existing pipelines read
- `src/audit/pipeline.rs` -- Pipeline/NodePipeline/GraphPipeline trait hierarchy
- `audit_plans/*.md` -- per-language pipeline analysis and detection logic specs
- `audit_plans/cross_file_analyzers.md` -- ProjectAnalyzer migration analysis

### Secondary (MEDIUM confidence -- official documentation)
- Semgrep rule syntax docs -- format comparison
- ast-grep YAML reference -- format comparison
- CodeQL query language docs -- format comparison

---
*Research completed: 2026-04-16*
*Ready for roadmap: yes*
