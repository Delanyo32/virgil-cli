# Phase 5: Final Cleanup + Test Health - Research

**Researched:** 2026-04-16
**Domain:** Rust pipeline deletion, JSON migration (TECH-02), dead-code cleanup, test health
**Confidence:** HIGH

## Summary

Phase 5 is the largest migration phase in this milestone. The scope confirmed in CONTEXT.md expands beyond "cleanup" to include TECH-02: migrating all 139 non-taint Rust pipeline files to JSON definitions in `src/audit/builtin/`. This encompasses every per-language tech-debt pipeline (the "Graph" pipelines like `magic_numbers`, `god_object_detection`, etc.) and every code-style pipeline (the "Legacy" trio: `dead_code`, `duplicate_code`, `coupling`). After migration and deletion of those 139 files, a dead-code pass on `helpers.rs` removes all helper functions whose only callers are gone.

The current codebase state (as of research date) is clean: `cargo test` passes with 162 integration tests + 8 legacy integration tests + 0 failures. There is exactly 1 compiler warning (unused variable `source` in `typescript/any_escape_hatch.rs` — a non-audit-pipeline file that is itself a migration target). The JSON engine, match_pattern stage, and compute_metric stage are all implemented and working.

The 14 taint-based Rust exception files are already identified and will NOT be migrated — they receive only a `// PERMANENT RUST EXCEPTION: ...` comment block at the top. The `src/audit/analyzers/` directory is entirely out of scope (all 3 files are active ProjectAnalyzers that stay as-is, per D-11).

**Primary recommendation:** Organize migration as 10 per-language plans (one per language group) + 1 final cleanup plan. For each plan: read the `audit_plans/{lang}_tech_debt.md` spec → write JSON → `cargo test` → delete Rust file → `cargo test` → add integration tests matching the deleted file's `#[test]` count.

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

- **D-01:** Phase 5 migrates all 139 non-taint Rust pipeline files to JSON. The pipelines/ directory gets fully emptied of migrated content — only taint-based exceptions remain as Rust.
- **D-02:** For pipelines whose logic cannot be faithfully expressed in `match_pattern` JSON (e.g., `duplicate_code` rolling-hash similarity, per-language `coupling` import fan-out analysis): write a simplified `match_pattern` that catches the most common instances. Document the precision delta in the JSON `"description"` field. Never skip a pipeline — every non-taint pipeline gets a JSON version.
- **D-03:** `audit_plans/` specs are authoritative — same approach as Phase 1 architecture. Read `audit_plans/{lang}_tech_debt.md` for each language before writing JSON pipelines. Only fall back to reading the Rust file when the spec lacks specific pattern details. Obvious bugs in Rust implementations should be fixed in the JSON version; non-trivial logic changes deferred.
- **D-04:** Plans organized per language group — same structure as Phase 4. Each plan covers all tech-debt + code-style pipelines for one language group plus integration tests. Approximately 9-10 plans total + 1 cleanup plan.
- **D-05:** Language ordering follows Phase 4 precedent: simpler language groups first (Rust, Go), more complex near end (JavaScript/TypeScript, C++, C#). Planner chooses exact ordering.
- **D-06:** Match Rust test depth per pipeline — for each Rust pipeline file being deleted, count its `#[test]` functions and create the equivalent number of integration tests in `tests/audit_json_integration.rs`.
- **D-07:** Tests are integration-style (pass a code snippet, assert findings via full `AuditEngine` path). Committed in the same batch as each pipeline migration.
- **D-08:** Zero-failure target. No fixed total test count target — goal is meaningful coverage per pipeline.
- **D-09:** `helpers.rs` stays at `src/audit/pipelines/helpers.rs` — no structural relocation.
- **D-10:** After pipeline files deleted, do a dead-code pass on `helpers.rs`: any `pub fn` with no remaining callers outside `src/audit/pipelines/` gets deleted. Happens in the final cleanup plan.
- **D-11:** All three files in `src/audit/analyzers/` are kept as-is. CLEAN-01 does not apply to them.
- **D-12:** Each of the 14 taint-based Rust pipeline files gets a comment block at the top: `// PERMANENT RUST EXCEPTION: This pipeline requires FlowsTo/SanitizedBy graph predicates for taint propagation analysis. These are not expressible in the match_pattern JSON DSL. Do not migrate — this file stays as Rust intentionally.`

### Claude's Discretion

- Exact ordering of language groups across the 9-10 migration plans
- Whether JavaScript and TypeScript pipelines are combined in one plan or split (they share a base in `javascript/mod.rs`)
- Which specific functions in `helpers.rs` lose all callers after pipeline deletion — planner does a dead-code audit at the end
- For `duplicate_code` specifically (rolling-hash logic): exact simplified match_pattern approach (structural size/line indicator vs. AST hash approximation) — planner's judgment based on what the audit_plans spec says

### Deferred Ideas (OUT OF SCOPE)

None — discussion stayed within phase scope.
</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| CLEAN-01 | `src/audit/analyzers/` helpers removed if no longer referenced by any remaining pipeline | VERIFIED: analyzers/ has 3 files, all are active ProjectAnalyzers (coupling, dead_exports, duplicate_symbols) — keep as-is per D-11 |
| CLEAN-02 | `src/audit/pipelines/` directory empty or removed after full migration | VERIFIED: 139 files must be deleted; 14 taint exceptions + helpers.rs + mod.rs files remain — directory NOT empty, but fully pruned |
| CLEAN-03 | Dead imports and unused helper functions in `src/audit/` cleaned up after all deletions | VERIFIED: dead-code pass on helpers.rs per D-10; primitive.rs files per language become dead and can be deleted |
| TEST-02 | `cargo test` passes with zero failures at every phase boundary | VERIFIED: currently passing (162 + 8 tests, 0 failures); 1 compiler warning (unused variable in any_escape_hatch.rs) |
</phase_requirements>

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| Tech-debt pattern detection (AST-level) | JSON pipeline + match_pattern stage | — | match_pattern wraps tree-sitter queries; executor runs per workspace file |
| Tech-debt pattern detection (graph-level) | JSON pipeline + select/where/flag stages | — | Graph nodes (symbols, files) drive detection; no AST needed |
| Code-style detection (dead_code, coupling, duplicate_code) | JSON pipeline with simplified match_pattern | helpers.rs (dead-code pass) | Simplification documented in JSON description field |
| Taint-based security (permanent exceptions) | Rust pipelines (kept as-is) | — | FlowsTo/SanitizedBy predicates not expressible in JSON DSL |
| ProjectAnalyzer layer (coupling, dead_exports, duplicate_symbols) | src/audit/analyzers/ (kept as-is) | — | CodeGraph-level cross-file analysis; unchanged by this phase |
| Integration test coverage | tests/audit_json_integration.rs | — | Same AuditEngine path used by Phase 1-4 tests |

---

## Standard Stack

### Core (all pre-existing, no new dependencies)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tree-sitter 0.25 | 0.25.x | AST parsing for match_pattern queries | Already in use; MUST NOT downgrade |
| serde_json 1 | 1.x | JSON pipeline deserialization | Already in use |
| include_dir | current | Embeds `src/audit/builtin/*.json` at compile time | Already in use via json_audit.rs |

**No new dependencies needed for this phase.** All machinery (match_pattern executor, compute_metric executor, JSON pipeline format, integration test harness) was built in Phases 1-4.

---

## Architecture Patterns

### System Architecture Diagram

```
audit_plans/{lang}_tech_debt.md  (authoritative spec)
    |
    v
[Write JSON pipeline]  -->  src/audit/builtin/{pipeline_name}.json
    |                           |
    v                           v
[cargo test green]          json_audit.rs (embed via include_dir!)
    |                           |
    v                           v
[Delete Rust pipeline .rs]  engine.rs (discovers JSON, suppresses same-name Rust pipeline)
    |                           |
    v                           v
[cargo test green again]    executor.rs (runs match_pattern / select+flag stages)
    |
    v
[Add integration tests]  -->  tests/audit_json_integration.rs
    |
    v
[cargo test green + new tests pass]
```

### Recommended Project Structure After Phase 5

```
src/audit/
├── builtin/          # ~239 JSON pipelines total (75 existing + 139 new + more)
│   ├── ...           # existing Phase 1-4 JSON files (keep as-is)
│   └── {new}.json    # 139+ new JSON pipelines from this phase
├── pipelines/
│   ├── helpers.rs    # PRUNED: only functions with non-pipeline callers remain
│   ├── mod.rs        # SHRUNK: only pub mod helpers + taint language dirs
│   ├── csharp/
│   │   ├── mod.rs         # SHRUNK: only pub mod helpers; security taint pipeline declarations
│   │   ├── csharp_ssrf.rs # PERMANENT RUST EXCEPTION (taint)
│   │   ├── sql_injection.rs # PERMANENT RUST EXCEPTION (taint)
│   │   └── xxe.rs         # PERMANENT RUST EXCEPTION (taint)
│   ├── go/            # Similar: mod.rs + sql_injection.rs + ssrf_open_redirect.rs
│   ├── java/          # Similar: mod.rs + sql_injection.rs + xxe.rs + java_ssrf.rs
│   ├── javascript/    # Similar: mod.rs + xss_dom_injection.rs + ssrf.rs
│   ├── php/           # Similar: mod.rs + sql_injection.rs + ssrf.rs
│   └── python/        # Similar: mod.rs + sql_injection.rs + ssrf.rs
│   # c/, cpp/, rust/, typescript/ DELETED (no taint exceptions)
├── analyzers/        # UNCHANGED: coupling.rs, dead_exports.rs, duplicate_symbols.rs
└── ...               # engine.rs, pipeline.rs, json_audit.rs unchanged
```

### Pattern 1: Tech-Debt Pipeline Migration (match_pattern approach)

**What:** Convert a Rust GraphPipeline or NodePipeline to a JSON file using `match_pattern` with a tree-sitter S-expression query.
**When to use:** When the pipeline detects a specific AST pattern (e.g., `magic_numbers`, `var_usage`, `loose_equality`).

```json
// Source: src/audit/builtin/magic_numbers_rust.json (example pattern)
{
  "pipeline": "magic_numbers",
  "category": "code-quality",
  "description": "Detects numeric literals used outside const/static/enum contexts. Simplified from Rust: does not check for #[allow] attributes or NOLINT. False positives in index expressions possible.",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(integer_literal) @lit",
      "exclude": {
        "is_test_file": true
      }
    },
    {
      "flag": {
        "pattern": "magic_number",
        "message": "Magic number literal detected — consider extracting to a named constant",
        "severity": "info"
      }
    }
  ]
}
```

**Key constraint:** The `"pipeline"` field in JSON must exactly match the `fn name()` return value in the Rust file it replaces. The engine uses this name for suppression (HashSet lookup). [VERIFIED: engine.rs line 87-91]

### Pattern 2: Code-Style Pipeline Migration (simplified select+flag)

**What:** Convert `dead_code`, `coupling`, `duplicate_code` (the Legacy trio) to JSON using simplified structural detection.
**When to use:** For the code-style category pipelines that use complex helpers (hash_block_normalized, count_all_identifier_occurrences).

```json
// Simplified dead_code example — document precision delta in description
{
  "pipeline": "dead_code",
  "category": "code-quality",
  "description": "Detects exported symbols with zero references via identifier occurrence counting. Simplified from Rust: no cross-file reference counting via CodeGraph. Detects in-file indicators only.",
  "languages": ["rust"],
  "graph": [
    {
      "select": "symbol",
      "where": {"exported": true},
      "exclude": {"is_test_file": true}
    },
    {
      "flag": {
        "pattern": "potentially_dead_export",
        "message": "Exported symbol '{{name}}' — verify it has external callers",
        "severity": "info"
      }
    }
  ]
}
```

### Pattern 3: Per-Language JSON Naming Strategy

**What:** Tech-debt pipelines that share a `name()` string across languages (e.g., `magic_numbers` in Rust, Go, C, C++, Python, JavaScript) require careful JSON naming to control suppression scope correctly.

**Decision from Phase 4 precedent:** Use same `"pipeline"` name with different `"languages"` filters. Multiple JSON files can share the same pipeline name — the engine deduplicates by name for suppression, but each JSON file applies its own language filter when running. [VERIFIED: engine.rs lines 87-91, 271-279]

**Per-language file naming convention:** `{pipeline_name}_{language_code}.json` for disambiguation at the filesystem level, BUT set `"pipeline": "magic_numbers"` (the shared name) for correct Rust pipeline suppression.

Example: `magic_numbers_rust.json` → `{"pipeline": "magic_numbers", "languages": ["rust"], ...}`

**Exception:** Pipelines with truly unique names per-language (e.g., `god_object_detection` for Rust, `god_struct` for Go, `god_class` for Java/C#) can use the exact pipeline name as the filename.

### Pattern 4: helpers.rs Dead-Code Pass

**What:** After all 139 non-taint pipeline files are deleted, identify which `pub fn` declarations in `helpers.rs` have zero callers outside `src/audit/pipelines/` and delete them.

**Callers that survive (MUST KEEP these helpers):**

| Helper Function | Non-Pipeline Caller | Location |
|-----------------|--------------------|----|
| `count_all_identifier_occurrences` | `AuditEngine::run()` | `src/audit/engine.rs:15` |
| `is_barrel_file` | `execute_match_pattern()`, `CouplingAnalyzer` | `src/graph/executor.rs:15`, `src/audit/analyzers/coupling.rs:8` |
| `is_excluded_for_arch_analysis` | `execute_match_pattern()`, `CouplingAnalyzer` | `src/graph/executor.rs:15`, `src/audit/analyzers/coupling.rs:8` |
| `is_test_file` | `execute_match_pattern()` | `src/graph/executor.rs:15` |
| `pub use crate::graph::metrics::*` (re-exports) | Implicitly through helpers.rs module | Stays as re-export |

[VERIFIED: via grep of non-pipeline callers]

**All other `pub fn` declarations become dead after pipeline deletion** and should be removed in the cleanup plan. This includes the ~49 other public functions (language-specific helpers, hash helpers, AST traversal helpers, etc.).

### Pattern 5: Permanent Rust Exception Annotation

**What:** Add a comment block at the top of each of the 14 taint-based security files.

```rust
// PERMANENT RUST EXCEPTION: This pipeline requires FlowsTo/SanitizedBy graph
// predicates for taint propagation analysis. These are not expressible in the
// match_pattern JSON DSL. Do not migrate — this file stays as Rust intentionally.
```

The 14 files: [VERIFIED: filesystem scan]
- `csharp/csharp_ssrf.rs`, `csharp/sql_injection.rs`, `csharp/xxe.rs`
- `go/sql_injection.rs`, `go/ssrf_open_redirect.rs`
- `java/java_ssrf.rs`, `java/sql_injection.rs`, `java/xxe.rs`
- `javascript/xss_dom_injection.rs`, `javascript/ssrf.rs`
- `php/sql_injection.rs`, `php/ssrf.rs`
- `python/sql_injection.rs`, `python/ssrf.rs`

### Anti-Patterns to Avoid

- **Skipping any non-taint pipeline:** Every one of the 139 must get a JSON file, even if simplified. "We can't express it in JSON" is not an acceptable outcome — D-02 requires a simplified version.
- **Using incorrect pipeline name in JSON:** The `"pipeline"` field must exactly match the Rust pipeline's `fn name()` return value. Wrong name = no suppression = doubled findings.
- **Deleting before JSON exists:** Always: write JSON → `cargo test` → delete → `cargo test`. Never delete first.
- **Using `pipeline_name_rust.json` as the `"pipeline"` value:** File can be named `magic_numbers_rust.json` but `"pipeline"` must be `"magic_numbers"`.
- **Forgetting to update mod.rs:** Removing `pub mod foo;` from mod.rs must happen when `foo.rs` is deleted, otherwise `cargo build` fails.
- **Leaving primitives.rs in deleted language dirs:** After migrating all files for c, cpp, rust, typescript, the `primitives.rs` in those dirs also has no callers and should be deleted along with the language subdirectory.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| AST pattern matching | Custom traversal in JSON | `match_pattern` stage with tree-sitter S-expression | Already implemented in executor.rs; just write the query string |
| Metric thresholds | New Rust metric functions | `compute_metric` stage (cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio) | Already wired in executor.rs |
| Symbol-level analysis | New graph traversal code | `select: "symbol"` + `where:` clauses | WhereClause supports kind, exported, is_test_file, cyclomatic_complexity, etc. |
| Integration test patterns | New test harness | Append to existing `tests/audit_json_integration.rs` | Pattern established across 162 tests in Phases 1-4 |

---

## Runtime State Inventory

> This is a migration/deletion phase — inventory required.

| Category | Items Found | Action Required |
|----------|-------------|-----------------|
| Stored data | None — no database; pipeline names stored in source code only | Code edit only |
| Live service config | None — no external services store pipeline names | None |
| OS-registered state | None | None |
| Secrets/env vars | None — pipeline names not in env vars or secrets | None |
| Build artifacts | `target/` contains compiled pipeline code; auto-rebuilt by `cargo build` after deletion | None — `cargo build` handles it |

**Nothing found in categories 1-5 that requires data migration.** All changes are code edits (write JSON, delete Rust, update mod.rs). The `cargo test` cycle at each step is the verification gate.

---

## Common Pitfalls

### Pitfall 1: Shared Pipeline Names Causing Double-Suppression
**What goes wrong:** Two JSON files with the same `"pipeline"` name (e.g., `magic_numbers`) for different languages — the engine sees the name once in the HashSet and suppresses ALL Rust `magic_numbers` pipelines, but that is actually correct behavior (they're all being replaced).
**Why it happens:** The suppression is name-based, not name+language based.
**How to avoid:** This is the INTENDED design. Verify that all JSON files for a shared-name pipeline are committed before deleting the Rust file from any language. [VERIFIED: engine.rs suppression logic]
**Warning signs:** If a JSON file for language A exists but the Rust file for language B is not yet deleted, language B findings come from JSON (correct, since JSON language filter applies).

### Pitfall 2: Forgetting mod.rs Update After Deletion
**What goes wrong:** Deleting `magic_numbers.rs` but leaving `pub mod magic_numbers;` in `mod.rs` causes compile error.
**Why it happens:** Rust module system requires explicit `pub mod` declarations; removing the file without updating the module file breaks compilation.
**How to avoid:** Always update mod.rs in the same commit as the file deletion. The canonical pattern: delete file + remove mod declaration + run `cargo test`.
**Warning signs:** `error[E0583]: file not found for module` on `cargo build`.

### Pitfall 3: Leaving Language Subdirs with Dead primitives.rs
**What goes wrong:** For languages with no taint exceptions (rust, c, cpp, typescript), ALL pipeline files including `primitives.rs` become dead. Leaving `primitives.rs` means `mod.rs` still declares `pub mod primitives;` and the file has no callers.
**Why it happens:** `primitives.rs` is used only by sibling pipeline files, not by any external caller. Once all siblings are deleted, it has zero callers.
**How to avoid:** Delete `primitives.rs` and the entire language subdirectory for rust, c, cpp, typescript as part of the cleanup plan. [VERIFIED: no external callers of lang-specific primitives.rs]
**Warning signs:** `warning: unused code` on `primitives.rs` symbols after all other files deleted.

### Pitfall 4: JSON Pipeline Category Mismatch
**What goes wrong:** Writing `"category": "security"` for a tech-debt pipeline causes it to not appear in `virgil audit code-quality tech-debt` output.
**Why it happens:** The engine applies pipeline_selector filtering; JSON audit language filter is checked but category routing in the CLI uses the category field.
**How to avoid:** Tech-debt pipelines: `"category": "code-quality"`. Code-style pipelines: `"category": "code-quality"`. Match Phase 1-4 category conventions.
**Warning signs:** `virgil audit` produces zero findings for a category that should have results.

### Pitfall 5: Integration Test Count Target Confusion
**What goes wrong:** Writing 1-2 integration tests per pipeline instead of matching the Rust unit test count (D-06).
**Why it happens:** Some Rust pipelines have 5-15 `#[test]` functions. The default instinct is "one positive, one negative."
**How to avoid:** Before deleting each `.rs` file, count `#[test]` occurrences with `grep -c "#\[test\]" file.rs`. Write that many integration tests. Review what each Rust test covers (edge cases, pattern variations).
**Warning signs:** Committed plan says "2 tests per pipeline" but the Rust file has 8 tests.

### Pitfall 6: helpers.rs Re-exports Getting Deleted
**What goes wrong:** Deleting the `pub use crate::graph::metrics::...` re-export lines at the top of helpers.rs breaks any future caller that imports them via helpers.
**Why it happens:** These re-exports look like "unnecessary" use statements during dead-code cleanup.
**How to avoid:** The re-exports are needed for backward compatibility if any non-pipeline code uses `helpers::compute_cyclomatic`. Check `grep -r "helpers::compute_\|helpers::ControlFlow"` before removing. In practice, the graph::metrics module is imported directly by executor.rs, so the re-exports in helpers.rs may actually be dead — verify before deciding.

---

## Code Examples

Verified patterns from the existing codebase:

### Integration Test Pattern (established in Phases 1-4)
```rust
// Source: tests/audit_json_integration.rs — established pattern
#[test]
fn magic_numbers_rust_finds_literal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), r#"
fn compute() -> i32 {
    let result = 42 * 100;  // magic numbers
    result
}
"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::TechDebt)
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(findings.iter().any(|f| f.pipeline == "magic_numbers"),
        "expected magic_numbers finding, got: {:?}", findings.iter().map(|f| &f.pipeline).collect::<Vec<_>>());
}

#[test]
fn magic_numbers_rust_clean() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), r#"
const MAX_RETRIES: i32 = 5;
fn compute() -> i32 { MAX_RETRIES }
"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(1_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::TechDebt)
        .pipelines(vec!["magic_numbers".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(findings.is_empty(), "expected no findings for clean code");
}
```

### JSON Pipeline with match_pattern (from Phase 4)
```json
// Source: src/audit/builtin/panic_dos_rust.json
{
  "pipeline": "panic_dos",
  "category": "security",
  "description": "Detects ...",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (field_expression field: (field_identifier) @method) @call_fn) @call"
    },
    {
      "flag": {
        "pattern": "unwrap_untrusted",
        "message": "unwrap/expect call detected",
        "severity": "warning"
      }
    }
  ]
}
```

### JSON Pipeline with symbol selection (from Phase 1)
```json
// Source: src/audit/builtin/api_surface_area_rust.json
{
  "pipeline": "api_surface_area_rust",
  "category": "architecture",
  "description": "...",
  "languages": ["rust"],
  "graph": [
    {
      "select": "symbol",
      "exclude": {
        "or": [
          {"is_test_file": true},
          {"is_generated": true}
        ]
      }
    },
    {"group_by": "file"},
    {"ratio": {"numerator": {"where": {"exported": true}}, "denominator": {}, "threshold": {"and": [{"count": {"gte": 10}}, {"ratio": {"gte": 0.8}}]}}},
    {"flag": {"pattern": "excessive_public_api", "message": "...", "severity": "info"}}
  ]
}
```

---

## Current State Inventory (Verified)

### Files in `src/audit/pipelines/`

**Total Rust files (all):** 175 [VERIFIED: `find ... -name "*.rs" | wc -l`]

**Breakdown:**
- `helpers.rs`: 1 file (stays, pruned)
- `mod.rs` × 11 (root + 10 language subdirs): stay, shrink
- `primitives.rs` × 10 (one per language): dead after all sibling deletions for rust/c/cpp/typescript; stay for csharp/go/java/javascript/php/python (if taint files use them — NONE do, so all primitives.rs are dead after their language's migration)
- Language tech-debt + code-style pipeline files: **139 files to migrate/delete**
- Taint exception files: **14 files to annotate and keep**

**Pipeline files to migrate by language:**

| Language | Tech-Debt Files | Code-Style Trio | Total | Rust Test Count |
|----------|-----------------|-----------------|-------|-----------------|
| Rust | 10 | 3 | 13 | ~127 |
| Go | 10 | 3 | 13 | ~112 |
| Python | 12 | 3 | 15 | ~188 |
| PHP | 7 | 3 | 10 | ~83 |
| Java | 11 | 3 | 14 | ~129 |
| JavaScript | 12 | 3 | 15 | ~142 |
| TypeScript | 11 | 3 | 14 | ~141 |
| C | 12 | 3 | 15 | ~129 |
| C++ | 12 | 3 | 15 | ~127 |
| C# | 12 | 3 | 15 | ~131 |
| **Total** | **109** | **30** | **139** | **~1309** |

[VERIFIED: filesystem scan]

### Files in `src/audit/analyzers/`

| File | Status | Reason |
|------|--------|--------|
| `coupling.rs` | KEEP (unchanged) | `CouplingAnalyzer` called by `engine.rs::architecture_analyzers()` |
| `dead_exports.rs` | KEEP (unchanged) | `DeadExportsAnalyzer` in `code_style_analyzers()` |
| `duplicate_symbols.rs` | KEEP (unchanged) | `DuplicateSymbolsAnalyzer` in `code_style_analyzers()` |

[VERIFIED: src/audit/analyzers/mod.rs]

### `cargo test` Current State

- **Result:** PASSING — 162 integration tests + 8 legacy integration tests + 0 doc tests [VERIFIED: `cargo test` output]
- **Warnings:** 1 warning — `unused variable: source` in `typescript/any_escape_hatch.rs:99` [VERIFIED: `cargo build`]
- **Failures:** 0

### `helpers.rs` Functions: Survival After Deletion

| Function | Non-Pipeline Caller | Keep After Cleanup |
|----------|--------------------|--------------------|
| `count_all_identifier_occurrences` | `engine.rs:15` | YES |
| `is_barrel_file` | `executor.rs:16`, `analyzers/coupling.rs:8` | YES |
| `is_excluded_for_arch_analysis` | `executor.rs:16`, `analyzers/coupling.rs:8` | YES |
| `is_test_file` | `executor.rs:16` | YES |
| `pub use crate::graph::metrics::*` (re-exports) | Verify before deleting | LIKELY YES |
| All ~49 other pub functions | Pipeline files only | DELETE in cleanup plan |

[VERIFIED: grep of non-pipeline callers across src/]

---

## Plan Organization Recommendation

Based on CONTEXT.md D-04 and D-05, the following plan structure satisfies the phase requirements:

### Language Migration Plans (10 plans)

| Plan | Language(s) | Files | Est. Test Count |
|------|------------|-------|-----------------|
| 05-01 | Rust | 13 | ~127 |
| 05-02 | Go | 13 | ~112 |
| 05-03 | Python | 15 | ~188 |
| 05-04 | PHP | 10 | ~83 |
| 05-05 | Java | 14 | ~129 |
| 05-06 | C | 15 | ~129 |
| 05-07 | C++ | 15 | ~127 |
| 05-08 | JavaScript | 15 | ~142 |
| 05-09 | TypeScript | 14 | ~141 |
| 05-10 | C# | 15 | ~131 |

### Cleanup Plan (1 plan)

| Plan | Tasks |
|------|-------|
| 05-11 | helpers.rs dead-code pass, delete dead primitives.rs files, delete dead language subdirs (rust/, c/, cpp/, typescript/), add PERMANENT RUST EXCEPTION comments to 14 taint files, update top-level mod.rs, final `cargo test` green |

**Total: ~11 plans**

Note: JavaScript and TypeScript are split into separate plans (05-08, 05-09) because TypeScript's `tech_debt_pipelines()` function calls a different impl (`typescript::tech_debt_pipelines(language)`) and shares security pipeline routing with JavaScript. This reduces risk of cross-pollination errors.

---

## Validation Architecture

### Test Framework

| Property | Value |
|----------|-------|
| Framework | Rust's built-in `#[test]` via `cargo test` |
| Config file | `Cargo.toml` (workspace) |
| Quick run command | `cargo test -- --test-thread=1 2>&1 \| tail -20` |
| Full suite command | `cargo test` |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|--------------|
| CLEAN-02 | `src/audit/pipelines/` contains no migrated Rust files | Structural (filesystem verify) | `find src/audit/pipelines/ -name "*.rs" \| grep -v "helpers\|mod.rs\|primitives\|sql_injection\|ssrf\|xxe\|xss_dom\|java_ssrf\|csharp_ssrf"` | N/A (manual) |
| CLEAN-03 | No dead-code warnings in `src/audit/` | Compiler | `cargo build 2>&1 \| grep "warning:.*unused"` | N/A |
| TEST-02 | `cargo test` passes with zero failures | Integration | `cargo test` | ✅ `tests/audit_json_integration.rs` |
| TECH-02 | Each migrated pipeline produces non-empty findings | Integration | Per-pipeline tests in audit_json_integration.rs | ❌ (Wave 0 gap — ~1309 new tests) |

### Sampling Rate
- **Per task commit:** `cargo test` (full suite — under 30 seconds as shown by current run)
- **Per wave merge:** `cargo test` + `cargo build 2>&1 | grep warning:`
- **Phase gate:** Full suite green + zero unused-import warnings in `src/audit/`

### Wave 0 Gaps
- [ ] ~1309 new tests in `tests/audit_json_integration.rs` — added incrementally in each language migration plan (D-06, D-07)

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `primitives.rs` in each language subdir has no callers outside that subdir's sibling files | Dead-code analysis | If wrong, deleting primitives.rs breaks compilation — caught immediately by `cargo test` |
| A2 | The `pub use crate::graph::metrics::*` re-exports in helpers.rs are not needed by non-pipeline callers | helpers.rs cleanup | If executor.rs or other callers import via helpers::, removing re-exports breaks compilation |
| A3 | All code-style trio pipelines (dead_code, coupling, duplicate_code) for all 10 languages have zero taint-analysis dependencies and can be safely simplified in JSON | Cleanup scope | If wrong, JSON simplified version has higher false-positive rate — acceptable per D-02 |

---

## Open Questions

1. **JavaScript/TypeScript plan split vs. combined**
   - What we know: TypeScript's `tech_debt_pipelines()` delegates to `typescript::tech_debt_pipelines(language)` which is language-parameterized; security pipeline routing delegates to `javascript::security_pipelines(language)`. The TypeScript pipeline files are entirely separate from JavaScript pipeline files.
   - What's unclear: Whether combining them in one plan (05-08+09) would create confusion during mod.rs updates.
   - Recommendation: Split into two plans (05-08 for JS, 05-09 for TS) to reduce per-plan scope and risk.

2. **helpers.rs re-export lines survival**
   - What we know: `helpers.rs` lines 7-14 are `pub use crate::graph::metrics::...`. These are used by callers who import `helpers::compute_cyclomatic` etc. Currently, executor.rs imports directly from `graph::metrics` — not from helpers.
   - What's unclear: Whether any of the 139 pipeline files being deleted are the ONLY callers of `helpers::compute_cyclomatic` etc., or if there are zero callers of these re-exports.
   - Recommendation: Planner should grep `helpers::compute_\|helpers::count_function\|helpers::compute_comment\|helpers::ControlFlowConfig` across non-pipeline files during cleanup plan. If zero results, re-exports are also dead.

---

## Environment Availability

Step 2.6: SKIPPED (no external dependencies — phase is pure code edits within the Rust workspace).

---

## Security Domain

The phase does not introduce new security attack surfaces. Taint-based security pipelines are explicitly kept as Rust with no modification. JSON pipelines for non-taint security patterns (command injection, buffer overflow, etc.) were already migrated in Phase 4. No new ASVS categories apply.

---

## Sources

### Primary (HIGH confidence)
- VERIFIED: filesystem scan of `src/audit/pipelines/` — all 175 files catalogued
- VERIFIED: `cargo test` execution — 170 passing tests, 0 failures
- VERIFIED: `cargo build` — 1 warning (unused variable in any_escape_hatch.rs)
- VERIFIED: `src/audit/engine.rs` — pipeline name suppression mechanism (HashSet<String>)
- VERIFIED: `src/graph/executor.rs` — helpers::is_barrel_file, is_excluded_for_arch_analysis, is_test_file imports
- VERIFIED: `src/audit/analyzers/mod.rs` — 3 active ProjectAnalyzers (keep as-is)
- VERIFIED: `src/audit/pipelines/helpers.rs` — 53 public functions enumerated; 4 confirmed non-pipeline callers

### Secondary (MEDIUM confidence)
- VERIFIED: CONTEXT.md D-01 through D-12 — all decisions loaded and reproduced verbatim above
- VERIFIED: audit_plans/ directory listing — all 10 tech-debt spec files present

---

## Metadata

**Confidence breakdown:**
- File inventory: HIGH — directly verified via filesystem
- helpers.rs dead-code analysis: HIGH — verified via grep of all non-pipeline callers
- JSON migration pattern: HIGH — 75+ existing JSON pipelines demonstrate the pattern
- Test count estimates: MEDIUM — counted via grep; exact numbers require file-by-file audit during planning
- Plan count (11 plans): MEDIUM — split vs. combined JS/TS is Claude's discretion

**Research date:** 2026-04-16
**Valid until:** 2026-05-16 (stable domain — no external dependencies)
