# Phase 1: Engine Fixes + Architecture JSON Expansion - Context

**Gathered:** 2026-04-16
**Status:** Ready for planning

<domain>
## Phase Boundary

Fix two engine bugs (ENG-01: doubled-findings suppression gap, ENG-02: hardcoded `include_str!` array) and deliver per-language JSON architecture pipelines for all 9 language groups (TypeScript/JS, Python, Rust, Go, Java, C, C++, C#, PHP). Delete the replaced 4 language-agnostic JSON files and all empty Rust architecture stub functions. Add integration test scaffolding.

This phase does NOT implement `match_pattern` or `compute_metric` executor stages — those are Phase 2. Architecture improvements are limited to what the existing graph-stage DSL can express.

</domain>

<decisions>
## Implementation Decisions

### Per-Language JSON Architecture File Structure

- **D-01:** Create 36 new JSON files (4 pipelines × 9 language groups) in `src/audit/builtin/`. Each file includes a `"languages"` filter field (e.g., `"languages": ["typescript", "javascript"]`). Naming: `{pipeline_name}_{lang}.json` (e.g., `module_size_distribution_typescript.json`).
- **D-02:** Delete the existing 4 language-agnostic JSON files (`api_surface_area.json`, `circular_dependencies.json`, `dependency_depth.json`, `module_size_distribution.json`) once all per-language replacements are in place. They are superseded and keeping them would cause double-running.

### Architecture Pipeline Spec Depth

- **D-03:** Implement graph-stage improvements from `audit_plans/` that the existing DSL supports — language-specific exclusions (`is_test_file`, `is_generated`), barrel file handling, language idiom filters. Skip improvements that require `match_pattern` (Phase 2 executor work). The constraint: "specs first, audit_plans/ is authoritative" applies to what the current engine CAN express.
- **D-04:** Each language's JSON files use language-calibrated thresholds — not shared uniform values. Examples: Go packages have different idiomatic size norms than Java classes; Python `__init__.py` files are barrel files by convention; Rust `lib.rs`/`mod.rs` are expected to have large API surfaces; PHP exports everything top-level by default.

### ENG-02: Auto-Discovery Mechanism

- **D-05:** Replace the hardcoded `include_str!` array in `builtin_audits()` with the `include_dir!` macro (add `include_dir` crate to `Cargo.toml`). The entire `src/audit/builtin/` directory gets embedded at compile time. Adding a new `.json` file is automatically picked up at next `cargo build` — no source change needed.

### Integration Test Strategy

- **D-06:** Create `tests/audit_json_integration.rs` as a separate integration test file. One representative language per pipeline: 4 pipelines × 1 representative language = 4 positive + 4 negative cases (8 tests total). Tests exercise the full `AuditEngine` path end-to-end, not just JSON parsing.

### Claude's Discretion

- **ENG-01 fix:** In `engine.rs`, apply the existing `json_pipeline_names` suppression to per-language pipelines (same pattern as the existing project-analyzer retain at line 245). Add `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))` immediately after building `lang_pipelines`.
- **ARCH-10 scope:** All per-language `architecture_pipelines()` functions already return empty vecs — no Rust architecture implementation files exist. ARCH-10 means removing the empty stub functions from all language `mod.rs` files and removing `architecture_pipelines_for_language()` from `src/audit/pipeline.rs` once the JSON pipelines replace the need for that dispatch.

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

No external specs beyond what's in the codebase — requirements are fully captured in decisions above and the files below.

### Architecture Pipeline Specs (per language)
- `audit_plans/architecture_rubrics.md` — Reference rubric IDs (Arch-1 through Arch-15) used throughout all architecture audit plans
- `audit_plans/javascript_architecture.md` — JavaScript/TypeScript architecture pipeline bugs and improvement specs
- `audit_plans/typescript_architecture.md` — TypeScript-specific architecture improvements (if separate from javascript_architecture.md)
- `audit_plans/python_architecture.md` — Python architecture pipeline specs
- `audit_plans/rust_architecture.md` — Rust architecture pipeline specs
- `audit_plans/go_architecture.md` — Go architecture pipeline specs
- `audit_plans/java_architecture.md` — Java architecture pipeline specs
- `audit_plans/c_architecture.md` — C architecture pipeline specs
- `audit_plans/cpp_architecture.md` — C++ architecture pipeline specs
- `audit_plans/csharp_architecture.md` — C# architecture pipeline specs
- `audit_plans/php_architecture.md` — PHP architecture pipeline specs

### Existing JSON Pipeline Templates
- `src/audit/builtin/api_surface_area.json` — Template for the api_surface_area graph-stage DSL pattern
- `src/audit/builtin/circular_dependencies.json` — Template for circular_dependencies pattern
- `src/audit/builtin/dependency_depth.json` — Template for dependency_depth pattern
- `src/audit/builtin/module_size_distribution.json` — Template for module_size_distribution pattern

### Engine Files (understand before modifying)
- `src/audit/json_audit.rs` — `builtin_audits()` hardcoded array to replace; `discover_json_audits()` layering logic to preserve
- `src/audit/engine.rs` — ENG-01 fix location (line ~245 area); how JSON pipeline names suppress Rust pipelines
- `src/audit/pipeline.rs` — `architecture_pipelines_for_language()` dispatch to remove after ARCH-10

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `src/audit/builtin/*.json` (4 files): Graph-stage DSL patterns that work today — use as templates for the 36 new language-specific files
- `src/audit/json_audit.rs::discover_json_audits()`: Already handles project-local → user-global → built-in layering; ENG-02 only changes how built-ins are embedded
- `engine.rs` line 245 `project_analyzers.retain(...)`: The exact suppression pattern to replicate for per-language pipelines (ENG-01)

### Established Patterns
- JSON pipeline format: `pipeline` (name), `category`, `description`, `languages` (optional filter), `graph` (stages array)
- Existing `graph` stages: `select`, `exclude`, `group_by`, `ratio`, `flag` — these are what Phase 1 can use
- `include_str!` pattern (to be replaced): hardcoded array in `builtin_audits()` — the whole function gets replaced by `include_dir!` iteration
- Per-language mod.rs files return empty `architecture_pipelines()` vecs — safe to remove entirely

### Integration Points
- New `include_dir` crate dependency: add to `Cargo.toml` `[dependencies]`
- `builtin_audits()` in `src/audit/json_audit.rs`: Replace array with `include_dir!` iteration
- `engine.rs` lang_pipeline construction block: Add `retain` call for ENG-01
- `src/audit/pipeline.rs::architecture_pipelines_for_language()`: Remove or empty out after ARCH-10

</code_context>

<specifics>
## Specific Ideas

- Language-calibrated threshold examples from audit_plans/:
  - Rust: `lib.rs` and `mod.rs` treated as barrel files (not flagged for api_surface_area)
  - Python: `__init__.py` files are barrel files by convention
  - Go: smaller package size norms vs Java
  - PHP: top-level declarations are always exported by default — api_surface_area threshold needs adjustment
  - TypeScript/JS: distinguish `.min.js`, `dist/`, `vendor/` as generated/excluded
- The 36 new JSON files can be generated methodically: start with the existing 4 templates, apply language-specific `exclude` clauses and calibrated thresholds from audit_plans/

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 01-engine-fixes-architecture-json-expansion*
*Context gathered: 2026-04-16*
