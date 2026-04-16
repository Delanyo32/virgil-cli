---
phase: 01-engine-fixes-architecture-json-expansion
reviewed: 2026-04-16T00:00:00Z
depth: standard
files_reviewed: 22
files_reviewed_list:
  - Cargo.toml
  - src/audit/engine.rs
  - src/audit/json_audit.rs
  - src/audit/pipeline.rs
  - src/audit/pipelines/c/mod.rs
  - src/audit/pipelines/cpp/mod.rs
  - src/audit/pipelines/csharp/mod.rs
  - src/audit/pipelines/go/mod.rs
  - src/audit/pipelines/java/mod.rs
  - src/audit/pipelines/javascript/mod.rs
  - src/audit/pipelines/php/mod.rs
  - src/audit/pipelines/python/mod.rs
  - src/audit/pipelines/rust/mod.rs
  - src/audit/pipelines/typescript/mod.rs
  - src/main.rs
  - src/server.rs
  - tests/audit_json_integration.rs
  - src/audit/builtin/api_surface_area_rust.json
  - src/audit/builtin/circular_dependencies_javascript.json
  - src/audit/builtin/dependency_graph_depth_rust.json
  - src/audit/builtin/dependency_graph_depth_go.json
  - src/audit/builtin/api_surface_area_php.json
  - src/audit/builtin/module_size_distribution_javascript.json
findings:
  critical: 0
  warning: 4
  info: 4
  total: 8
status: issues_found
---

# Phase 01: Code Review Report

**Reviewed:** 2026-04-16
**Depth:** standard
**Files Reviewed:** 22
**Status:** issues_found

## Summary

This phase introduces a JSON-driven audit pipeline engine and migrates the architecture audit category away from Rust `ProjectAnalyzer` implementations. The core machinery in `json_audit.rs` and `engine.rs` is sound: discovery priority (project-local beats user-global beats built-ins), deduplication, language filtering, and the JSON-pipeline-overrides-Rust-pipeline suppression all work correctly. The 36 built-in JSON audit files are well-formed and consistent.

Four logic bugs were found that could produce incorrect results in specific use cases. No security issues were identified. Four informational items cover dead code, inconsistency, and test design.

---

## Warnings

### WR-01: JSON audits run regardless of `pipeline_selector` — architecture pipelines fire on every category

**File:** `src/audit/engine.rs:261-299`
**Issue:** The JSON audit loop (lines 261–299) runs for all discovered JSON audit files whenever `graph` is `Some`. It does **not** consult `self.pipeline_selector`. All 36 built-in JSON files are tagged with `"category": "architecture"`, but when a caller invokes the engine with `PipelineSelector::Security` and passes a graph, all 36 architecture pipelines fire alongside the security Rust pipelines. In `main.rs`, `run_tech_debt_ws` / `run_security_ws` etc. pass `Some(&index)` to the engine (lines 444, 568), so this fires on every CLI audit command. In `server.rs`, only `Architecture` and `CodeStyle` pass `index_ref = Some(&state.code_graph)` in `handle_audit_summary`, but the `handle_audit_category` call for `architecture` also passes `Some` (line 403) while security/scalability pass `None` — so the server is partially protected by accident rather than by design.

**Fix:** Filter JSON audits by category before running them. The `JsonAuditFile` struct already has a `category` field. Map `PipelineSelector` variants to their expected category strings and skip JSON audits whose `category` does not match:
```rust
fn selector_category(selector: PipelineSelector) -> &'static str {
    match selector {
        PipelineSelector::TechDebt  => "code-quality",
        PipelineSelector::Complexity => "code-quality",
        PipelineSelector::CodeStyle  => "code-quality",
        PipelineSelector::Security   => "security",
        PipelineSelector::Scalability => "scalability",
        PipelineSelector::Architecture => "architecture",
    }
}

// In the JSON audit loop:
let expected_category = selector_category(self.pipeline_selector);
for json_audit in &json_audits {
    if json_audit.category != expected_category {
        continue;
    }
    // ... existing language + pipeline filter checks
}
```

---

### WR-02: `project_dir` never set in `main.rs` or `server.rs` — project-local audit overrides are silently disabled

**File:** `src/audit/engine.rs:83-85`, `src/main.rs` (all `run_*_ws` functions), `src/server.rs` (all audit handlers)
**Issue:** `AuditEngine::project_dir()` is the only way to enable project-local JSON audit discovery (the `.virgil/audits/` directory). Every call to `AuditEngine::new()` in `main.rs` and `server.rs` omits `.project_dir(...)`, so `discover_json_audits` is always called with `None` (line 84: `self.project_dir.as_deref()`). The project-local override feature is completely unreachable from the CLI and server — only the built-in 36 audits ever run. This contradicts the documented behavior ("project-local beats built-in").

For the CLI, the workspace root directory is already available in every `run_*_ws` helper; for the server, the S3 URI cannot provide a local directory, so the feature is intentionally irrelevant there. The CLI gap is the actionable bug.

**Fix:** Pass the workspace root directory to the engine in the CLI helpers:
```rust
// In run_architecture_ws (and the other run_*_ws helpers) in main.rs:
let mut engine = audit::engine::AuditEngine::new()
    .languages(languages)
    .pipeline_selector(audit::engine::PipelineSelector::Architecture)
    .project_dir(workspace.root().to_path_buf()); // <- add this
```
This assumes `Workspace::root()` exists or can be derived from the dir argument already passed to `resolve_audit_workspace`. If `Workspace` does not expose the root, thread the `dir: &Path` argument into each `run_*_ws` helper.

---

### WR-03: `handle_audit_summary` in `server.rs` passes `None` for graph to TechDebt/Security/Scalability — JSON architecture audits silently skipped

**File:** `src/server.rs:274-279`
**Issue:** In `handle_audit_summary`, `index_ref` is `Some(&state.code_graph)` only for `Architecture` and `CodeStyle` (line 275). For `TechDebt`, `Security`, and `Scalability`, `None` is passed. The JSON audit loop in `engine.rs` is gated on `if let Some(g) = graph` (line 261), so JSON audits are completely suppressed for those three categories in the summary endpoint. This means the audit summary response silently underreports findings from any JSON audits categorized under those selectors. As a secondary effect, `GraphPipeline` Rust pipelines inside those categories also receive an empty fallback graph (`effective_graph` from line 151) rather than the real one — but the current code passes `graph_ref = None` (line 152), so `Legacy` pipelines that use `PipelineContext::graph` also see `None`.

The intent appears to be that only Architecture and CodeStyle need the graph, but this conflicts with JSON audits and GraphPipeline variants present in tech_debt/security/scalability pipelines for multiple languages.

**Fix:** Pass the pre-built `code_graph` to all engine invocations in the server — it's already built once and stored in `AppState`. Cost is zero (it's `Arc`-equivalent access via a shared reference):
```rust
// Replace the index_ref match in handle_audit_summary:
let index_ref = Some(&state.code_graph);
```
The engine already handles the case gracefully when a graph is provided but no graph-dependent pipelines are present.

---

### WR-04: `api_surface_area_php.json` uses a higher threshold than the other language variants — silent inconsistency

**File:** `src/audit/builtin/api_surface_area_php.json:22`
**Issue:** `api_surface_area_php.json` requires `count >= 15` before triggering, while every other language variant (`api_surface_area_rust.json`, and by reference `api_surface_area_javascript.json` etc.) requires `count >= 10`. The audit plans in `audit_plans/` (per CLAUDE.md) define `excessive_public_api` with threshold `>= 10 symbols AND > 80% exported`. PHP will silently miss files with 10–14 symbols that would be flagged in other languages. This is not documented as intentional.

**Fix:** Align PHP to the cross-language baseline:
```json
"threshold": {
  "and": [
    {"count": {"gte": 10}},
    {"ratio": {"gte": 0.8}}
  ]
}
```

---

## Info

### IN-01: `JsonAuditFile.category` field is stored but never used for filtering — dead field

**File:** `src/audit/json_audit.rs:13-14`
**Issue:** The `category` field is parsed and stored in `JsonAuditFile` but is never read anywhere in `engine.rs`, `main.rs`, or `server.rs`. Until WR-01 is fixed, it is completely dead. This is a code smell that may mislead future contributors into thinking category-based filtering is already implemented.

**Fix:** Document it explicitly as the intended filter field (add a comment), or suppress the lint if not yet used:
```rust
/// Audit category — used to filter pipelines by PipelineSelector.
/// Must match one of: "architecture", "code-quality", "security", "scalability".
pub category: String,
```

---

### IN-02: `test_builtin_audits_returns_four` — stale test name, passes for wrong reason

**File:** `src/audit/json_audit.rs:129`
**Issue:** The test function is named `test_builtin_audits_returns_four` but asserts `audits.len() >= 36`. The name is a leftover from an earlier version when only 4 built-ins existed. This is misleading but harmless.

**Fix:** Rename the test to reflect the current expectation:
```rust
#[test]
fn test_builtin_audits_returns_all_36() {
```

---

### IN-03: `dependency_graph_depth_go.json` threshold is `gte: 5` while the Rust variant uses `gte: 4` — undocumented asymmetry

**File:** `src/audit/builtin/dependency_graph_depth_go.json:18`, `src/audit/builtin/dependency_graph_depth_rust.json:18`
**Issue:** The Rust `dependency_graph_depth` pipeline triggers at depth >= 4; the Go variant triggers at depth >= 5. The integration test comment in `tests/audit_json_integration.rs` line 185 states the TypeScript threshold is `gte:6`. These three languages have three different depth thresholds with no documentation explaining the rationale. If these are intentional per-language calibrations, a comment in each JSON file would make this clear. If they are errors, they should be unified.

**Fix:** Add a comment field (JSON does not support comments, so add a `"notes"` key or document in the audit_plans file) or verify alignment with `audit_plans/` specs.

---

### IN-04: `circular_dependencies_javascript.json` language list includes `"tsx"` and `"jsx"` but those are separate `Language` enum variants — language filter may not match

**File:** `src/audit/builtin/circular_dependencies_javascript.json:5`
**Issue:** The `languages` array is `["typescript", "javascript", "tsx", "jsx"]`. The engine's language filter (engine.rs line 274) compares with `l.as_str().eq_ignore_ascii_case(lang_str)`. This is correct only if `Language::Tsx.as_str()` returns `"tsx"` and `Language::JavaScript.as_str()` returns `"javascript"`. If the `as_str()` method returns different casing or spelling (e.g. `"TypeScript"`, `"js"`, `"JSX"`) then TSX/JSX files would be silently excluded from this pipeline. The `eq_ignore_ascii_case` comparison handles casing, but not spelling differences. This is a latent risk that should be verified against `Language::as_str()` implementation.

**Fix:** Verify `Language::as_str()` values for all languages and ensure the JSON `languages` arrays use the exact strings returned (case insensitive). Add a unit test that constructs all `Language` variants, calls `as_str()`, and asserts the returned strings match what the JSON files use.

---

_Reviewed: 2026-04-16_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: standard_
