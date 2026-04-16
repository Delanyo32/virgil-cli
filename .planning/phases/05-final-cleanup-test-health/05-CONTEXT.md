# Phase 5: Final Cleanup + Test Health - Context

**Gathered:** 2026-04-16
**Status:** Ready for planning

<domain>
## Phase Boundary

Phase 5 is a **migration + cleanup phase**, not just a cleanup phase. The scope expands to include TECH-02 (per-language tech debt + code-style pipeline migration), which was deferred from Phase 3. Together with structural cleanup (helpers.rs pruning, permanent exception documentation, test replacement), this achieves the true "JSON-first" goal.

**In scope:**
- Migrate all 139 non-taint Rust pipeline files to JSON definitions in `src/audit/builtin/`
- Per language: all tech-debt + code-style pipelines (dead_code, coupling, duplicate_code, magic_numbers, god_object, language-specific patterns)
- Prune dead functions from `helpers.rs` after pipeline deletions
- Add PERMANENT RUST EXCEPTION comments to the 14 taint-based security files
- Add integration tests matching Rust test depth per pipeline
- `cargo test` passes with zero failures as the verified final state

**Permanent Rust exceptions (not migrated — 14 files, already identified in Phase 4):**
- `sql_injection` (Go, Java, Python, PHP, C#)
- `xss_dom_injection` (JavaScript)
- `ssrf` / `ssrf_open_redirect` (JavaScript, PHP, Python, Go)
- `xxe` (Java, C#)

**Out of scope:**
- `src/audit/analyzers/` (coupling, dead_exports, duplicate_symbols — all active ProjectAnalyzers, keep as-is)
- `helpers.rs` structural relocation (stays at `src/audit/pipelines/helpers.rs`)
- JSON engine enhancements (no new executor stages)
- New audit categories or patterns beyond what audit_plans/ specifies

**Requirements covered:** TECH-02, TECH-03, CLEAN-01, CLEAN-02, CLEAN-03, TEST-01, TEST-02

</domain>

<decisions>
## Implementation Decisions

### TECH-02 Migration Scope

- **D-01:** Phase 5 **migrates all 139 non-taint Rust pipeline files** to JSON — this is the TECH-02 work deferred from Phase 3. The pipelines/ directory gets fully emptied of migrated content (only taint-based exceptions remain as Rust). No pipeline category is abandoned.

- **D-02:** For pipelines whose logic cannot be faithfully expressed in `match_pattern` JSON (e.g., `duplicate_code` rolling-hash similarity, per-language `coupling` import fan-out analysis): write a **simplified match_pattern** that catches the most common instances of the anti-pattern. Document the precision delta in the JSON `"description"` field (e.g., "simplified from Rust: detects structural indicators only, not semantic duplication"). Never skip a pipeline — every non-taint pipeline gets a JSON version.

- **D-03:** **`audit_plans/` specs are authoritative** — same approach as Phase 1 architecture. Planner reads `audit_plans/{lang}_tech_debt.md` for each language before writing JSON pipelines. Only falls back to reading the Rust file when the spec lacks specific pattern details. Obvious bugs in Rust implementations should be fixed in the JSON version; non-trivial logic changes deferred.

### Plan Organization

- **D-04:** Plans are organized **per language group** — same structure as Phase 4. Each plan covers all tech-debt + code-style pipelines for one language group plus integration tests. Approximately 9-10 plans total (one per language group) + 1 cleanup plan for helpers.rs pruning and exception documentation.

- **D-05:** Language ordering follows Phase 4 precedent: simpler language groups first (Rust, Go), more complex near end (JavaScript/TypeScript, C++, C#). Planner chooses exact ordering.

### Test Strategy

- **D-06:** **Match Rust test depth per pipeline** — for each Rust pipeline file being deleted, count its existing `#[test]` functions and create the equivalent number of integration tests in `tests/audit_json_integration.rs`. Some pipelines have 5-10 tests. This is more work per plan but preserves coverage depth.

- **D-07:** Tests are integration-style (pass a code snippet, assert findings via full `AuditEngine` path) — same pattern as Phases 1-4. Committed in the same batch as each pipeline migration, not in a separate plan.

- **D-08:** Zero-failure target. No fixed total test count target — the goal is meaningful coverage per pipeline, not matching the baseline number of 2,142. The final count will be lower (Rust unit tests are gone) but each pipeline has proportional coverage.

### helpers.rs Disposition

- **D-09:** `helpers.rs` **stays at `src/audit/pipelines/helpers.rs`** — no structural relocation. The functions used by `graph/executor.rs`, `engine.rs`, and `analyzers/coupling.rs` are still needed; the module remains.

- **D-10:** After pipeline files are deleted, do a **dead-code pass on helpers.rs**: any `pub fn` with no remaining callers outside `src/audit/pipelines/` gets deleted. This satisfies CLEAN-03 and eliminates unused exports. The pass happens in the final cleanup plan after all language migration plans complete.

### analyzers/ Cleanup

- **D-11:** All three files in `src/audit/analyzers/` are **kept as-is**:
  - `coupling.rs` — `CouplingAnalyzer` (cross_file_coupling) is actively called by `engine.rs` for architecture analysis
  - `dead_exports.rs` — `DeadExportsAnalyzer` registered in `code_style_analyzers()`
  - `duplicate_symbols.rs` — `DuplicateSymbolsAnalyzer` registered in `code_style_analyzers()`
  These are `ProjectAnalyzer` implementations operating on the full `CodeGraph` — not dead code. CLEAN-01 does not apply to them.

### Permanent Exception Documentation

- **D-12:** Each of the 14 taint-based Rust pipeline files gets a **comment block at the top of the file**:
  ```rust
  // PERMANENT RUST EXCEPTION: This pipeline requires FlowsTo/SanitizedBy graph
  // predicates for taint propagation analysis. These are not expressible in the
  // match_pattern JSON DSL. Do not migrate — this file stays as Rust intentionally.
  ```
  Added in the final cleanup plan.

### Claude's Discretion

- Exact ordering of language groups across the 9-10 migration plans
- Whether JavaScript and TypeScript pipelines are combined in one plan or split (they share a base in `javascript/mod.rs`)
- Which specific functions in `helpers.rs` lose all callers after pipeline deletion — planner does a dead-code audit at the end
- For `duplicate_code` specifically (rolling-hash logic): exact simplified match_pattern approach (structural size/line indicator vs. AST hash approximation) — planner's judgment based on what the audit_plans spec says

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Tech Debt Audit Plans (authoritative specs for TECH-02)
These files define what each per-language tech-debt pipeline should detect. Read BEFORE writing JSON pipelines for each language.
- `audit_plans/typescript_tech_debt.md` — TypeScript tech debt patterns and thresholds
- `audit_plans/javascript_tech_debt.md` — JavaScript tech debt patterns
- `audit_plans/rust_tech_debt.md` — Rust tech debt patterns
- `audit_plans/go_tech_debt.md` — Go tech debt patterns
- `audit_plans/python_tech_debt.md` — Python tech debt patterns
- `audit_plans/java_tech_debt.md` — Java tech debt patterns
- `audit_plans/c_tech_debt.md` — C tech debt patterns
- `audit_plans/cpp_tech_debt.md` — C++ tech debt patterns
- `audit_plans/csharp_tech_debt.md` — C# tech debt patterns
- `audit_plans/php_tech_debt.md` — PHP tech debt patterns

### Cross-File Analyzers Reference
- `audit_plans/cross_file_analyzers.md` — Analyzes coupling, dead_exports, duplicate_symbols ProjectAnalyzers (do NOT migrate these)

### Prior Phase Migration Precedents
- `.planning/phases/04-security-per-language-scalability-migration/04-CONTEXT.md` — Phase 4 decisions on match_pattern translation approach, simplification strategy, and plan structure
- `.planning/phases/03-tech-debt-scalability-json-migration/03-CONTEXT.md` — Phase 3 decisions on complexity pipeline structure and deletion strategy

### JSON Pipeline Structure
- `src/audit/builtin/api_surface_area_rust.json` — Canonical template for JSON pipeline format (read this before writing new pipelines)
- `src/audit/json_audit.rs` — JSON audit engine implementation (understand stage dispatch before writing pipelines)

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `src/audit/builtin/*.json` — 75+ existing JSON pipeline files from Phases 1-4 as structural templates
- `tests/audit_json_integration.rs` — Existing integration test file; all new tests append here
- `src/audit/pipelines/helpers.rs` — Shared helpers; only non-pipeline callers are `executor.rs`, `engine.rs`, `coupling.rs`

### Established Patterns
- Phase 4 per-language migration pattern: read Rust file → identify match_pattern queries → write JSON → cargo test → delete Rust file → cargo test → add integration tests
- JSON pipeline naming: `{pipeline_name}_{lang}.json` for per-language, `{pipeline_name}.json` for cross-language
- Integration test pattern: create temp dir, write fixture file, run `AuditEngine`, assert findings contain/don't contain expected pattern

### Integration Points
- `src/audit/engine.rs` — Loads JSON pipelines via `json_audit.rs`; dispatches `code_style_analyzers()` and `architecture_analyzers()` for ProjectAnalyzer layer
- `src/graph/executor.rs` — Uses metric helpers from `helpers.rs` (stays after pruning)
- `src/audit/pipelines/mod.rs` — Will shrink as language subdirs are removed; eventually may only need `pub mod helpers;`

</code_context>

<specifics>
## Specific Ideas

- Match Rust test depth: before deleting each Rust pipeline file, count its `#[test]` functions and write that many integration tests for the JSON replacement
- Permanent exception comment format decided: `// PERMANENT RUST EXCEPTION: ...` at top of each taint-based file
- Simplified duplicate_code: if rolling-hash isn't expressible, document in JSON `"description"` — acceptable precision loss with explicit documentation

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 05-final-cleanup-test-health*
*Context gathered: 2026-04-16*
