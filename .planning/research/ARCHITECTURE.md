# Architecture: JSON Audit Pipeline Migration

**Domain:** Static analysis pipeline migration — Rust-to-JSON declarative rules
**Researched:** 2026-04-16

---

## System Overview

virgil-cli has two coexisting pipeline execution paths. Both are live simultaneously:

**Path A — Rust pipelines:** `src/audit/pipelines/{language}/{pipeline}.rs` implements `Pipeline`,
`NodePipeline`, or `GraphPipeline` traits. Loaded via `pipeline.rs` dispatch functions
(`tech_debt_pipelines_for_language`, etc.). Registered at compile time.

**Path B — JSON pipelines:** `src/audit/builtin/*.json` files embedded via `include_str!` in
`json_audit.rs`. Loaded at runtime via `discover_json_audits()`. Executed via
`graph::executor::run_pipeline()` operating on the `CodeGraph`.

These paths are NOT symmetric replacements. Understanding the asymmetry is critical for the
migration plan.

---

## Component Boundaries

### What JSON Pipelines Are Today

JSON pipelines in `src/audit/builtin/` execute as `GraphStage` sequences on the `CodeGraph`.
The four existing files show the current vocabulary:

- `select`: choose node type (`file`, `symbol`) with `exclude` predicates
- `find_cycles`: Tarjan SCC on edge type
- `group_by`: aggregate nodes by attribute (`file`)
- `count`: threshold filter on group size
- `max_depth`: longest-path DAG traversal on edge type
- `ratio`: count numerator/denominator with combined threshold
- `flag`: emit `AuditFinding` with pattern, message, severity / severity_map

The engine runs JSON pipelines **after** all Rust per-file pipelines and **after**
`ProjectAnalyzers`. JSON findings are appended to the same `findings` vec and appear identical
in output.

### What JSON Pipelines Are NOT Today

JSON pipelines only run when `graph` is `Some(&CodeGraph)`. The engine's per-file rayon loop
(which handles `NodePipeline`, `GraphPipeline`, and `Legacy Pipeline`) is a separate code path
that JSON pipelines never enter. JSON pipelines have no access to the per-file tree-sitter AST.
They operate exclusively on graph-level nodes and edges.

### The Override Mechanism — Scope Is Narrower Than It Appears

`json_pipeline_names` is built from loaded JSON audits. This set is used only at line 245 of
`engine.rs` to suppress `ProjectAnalyzers` (the three cross-file analyzers: `circular_deps`,
`dependency_depth`, `coupling`). The same suppression does **not** apply to the per-file Rust
pipeline vectors. A JSON file named `cyclomatic_complexity.json` would NOT suppress the Rust
`cyclomatic.rs` pipeline — both would run, producing duplicate findings.

**Consequence for migration:** The safe migration sequence for any per-file pipeline is:
1. Write JSON file
2. Validate JSON findings match expected behavior
3. Delete the Rust `.rs` file from `src/audit/pipelines/{language}/`
4. Remove its `Box::new(...)` line from the language `mod.rs`
5. Remove its `pub mod` declaration from the language `mod.rs`

Step 3 is what prevents duplication, not the JSON override mechanism.

### Architecture Pipelines: The Proven Template

All 10 language `architecture_pipelines()` functions return `vec![]`. The 4 JSON architecture
pipelines run via Path B (graph executor) only. This is the correct end state for all pipelines:
Rust files deleted, JSON files driving execution. The architecture category proves the pattern
is operational.

---

## Pipeline Inventory

### Counts

| Language | Rust `.rs` files | Unique pipeline names |
|---|---|---|
| TypeScript | 24 | 24 |
| JavaScript | 32 | 32 |
| Rust | 29 | 29 |
| Python | 31 | 31 |
| Go | 29 | 29 |
| Java | 31 | 31 |
| C | 32 | 32 |
| C++ | 32 | 32 |
| C# | 32 | 32 |
| PHP | 26 | 26 |
| **Total** | **298** | **163 unique names** |

The gap (298 files vs 163 names) exists because many pipeline names are shared across languages.
The same JSON pipeline name applies to all languages, but the Rust implementation is per-language.

### Shared Pipeline Names (All 10 Languages — One JSON File Covers All)

```
cyclomatic_complexity    function_length    cognitive_complexity    comment_to_code_ratio
dead_code                duplicate_code     coupling
n_plus_one_queries       sync_blocking_in_async    memory_leak_indicators
```

These 10 names appear in every language. One JSON file with no `languages` filter replaces
10 Rust `.rs` files each. Highest ROI targets.

### Category Breakdown Per Language

Each language mod.rs exposes 6 category functions. The pipeline count in each:

| Category | Approx pipelines/lang | JSON coverage today |
|---|---|---|
| tech_debt | 7-12 | 0 |
| complexity | 4 | 0 (all 4 shared names) |
| code_style | 3 | 0 (all 3 shared names: dead_code, duplicate_code, coupling) |
| security | 7-9 | 0 |
| scalability | 3 | 0 (all 3 shared names) |
| architecture | 0 (all empty `vec![]`) | 4 |

### Trait Type Breakdown

| Trait | Count | JSON compatibility |
|---|---|---|
| `impl Pipeline for` (legacy) | 172 | Replace with graph query stages |
| `impl GraphPipeline for` | 91 | Replace with graph query stages |
| `impl NodePipeline for` | 25 | Replace with graph query stages |

All three trait types are equivalent from the migration perspective: remove the Rust file,
add JSON with the same pipeline name.

### Cross-File Analyzers (ProjectAnalyzers)

Three `ProjectAnalyzer` implementations in `src/audit/analyzers/`:
- `circular_deps` (file: `circular_deps.rs`) — already replaced by `circular_dependencies.json`
- `dependency_depth` (file: `dependency_depth.rs`) — already replaced by `dependency_depth.json`
- `coupling` (file: `coupling.rs`) — not yet replaced

These are different from per-file pipelines. They implement `ProjectAnalyzer` trait, run after
the rayon per-file loop, and operate on the full `CodeGraph`. The JSON override mechanism
suppresses them by name. Migration: write JSON, verify, then the engine already suppresses the
Rust `ProjectAnalyzer` automatically via `json_pipeline_names`.

---

## JSON Engine Integration — What Must Change

### Adding a New JSON Builtin

`json_audit.rs` `builtin_audits()` uses a hardcoded `include_str!` array. Every new JSON
builtin file requires two edits:

1. Add `src/audit/builtin/{pipeline_name}.json`
2. Add `include_str!("builtin/{pipeline_name}.json")` to the `sources` array in `builtin_audits()`

This is the only engine change needed. No other modification to `json_audit.rs`, `engine.rs`,
`pipeline.rs`, or any language `mod.rs` is needed for adding JSON pipelines.

### Adding Override Suppression for Per-File Rust Pipelines (Optional Engine Change)

The current override mechanism only suppresses `ProjectAnalyzers`. If we want JSON to override
per-file Rust pipelines without deleting Rust files, the engine would need:

```rust
// In the per-file pipeline selection loop (engine.rs ~line 96):
lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()));
```

This would let JSON files suppress corresponding Rust pipelines without deleting them —
useful for a soft migration where we validate JSON output before hard-deleting Rust.
This is an optional but low-risk one-line engine change that makes the migration safer.

### Language Filtering in JSON Files

JSON pipelines support a `languages` field:
```json
{ "languages": ["typescript", "tsx"] }
```

Language-specific JSON files must include this field. Cross-language shared pipelines
(the 10 shared names above) should omit `languages` to apply to all.

---

## Build Order and Phase Structure

### Guiding Principles

1. **Category-first, not language-first.** The 10 shared pipeline names (complexity, scalability,
   code_style) should each become a single JSON file that covers all 10 languages. Doing this
   category-wide in one phase eliminates the most Rust files per JSON file authored.

2. **High-shared-count first.** The 10 universally-shared names should be Phase 1. Each JSON
   file removes 10 Rust `.rs` files (one per language).

3. **Architecture is already done.** 4 architecture JSON files already exist and all
   `architecture_pipelines()` functions return `vec![]`. No architecture work remains in the
   pipeline layer. The 3 `ProjectAnalyzers` (circular_deps already done, dependency_depth
   already done, coupling remaining) complete this category.

4. **Security and tech-debt last.** These are language-specific (patterns differ significantly
   across languages) and have the most variation in detection logic. They benefit from having
   the shared infrastructure patterns (complexity, code_style) established first so contributors
   understand the JSON format.

5. **Keep `cargo test` green throughout.** Because the migration is: write JSON → delete Rust →
   remove registration, the test suite loses 2,172 unit tests as files are deleted. This is
   expected. New integration tests in `tests/audit_json_integration.rs` replace them.

### Recommended Phase Order

#### Phase A: Shared Cross-Language Pipelines (10 JSON files → 100 Rust files removed)

Target: Complexity category (4 pipelines × 10 languages) + Scalability category
(3 pipelines × 10 languages) + Code Style category (3 pipelines × 10 languages).

These 10 pipeline names appear identically in all 10 languages:
- `cyclomatic_complexity` — 10 Rust files
- `function_length` — 10 Rust files
- `cognitive_complexity` — 10 Rust files
- `comment_to_code_ratio` — 10 Rust files
- `n_plus_one_queries` — 10 Rust files
- `sync_blocking_in_async` — 10 Rust files
- `memory_leak_indicators` — 10 Rust files
- `dead_code` — 10 Rust files
- `duplicate_code` — 10 Rust files
- `coupling` — 10 Rust files (per-file coupling, distinct from ProjectAnalyzer coupling)

**Engine change needed here:** Add the one-line per-file suppression to `engine.rs` so JSON
files shadow Rust files during the validation window before deletion. Without this, running
both the JSON and Rust `cyclomatic_complexity` pipeline doubles every finding.

**Delete order:** Add JSON file → validate output matches → remove Rust files from all 10
language directories in one commit → remove registrations from all 10 `mod.rs` files.

#### Phase B: Remaining ProjectAnalyzer — Coupling Cross-File

Target: `CouplingAnalyzer` in `src/audit/analyzers/coupling.rs`.

This is a `ProjectAnalyzer` (cross-file graph), not a per-file pipeline. The JSON override
mechanism already suppresses it when a JSON file with `pipeline: "cross_file_coupling"` exists.
(Note: the `CouplingAnalyzer` name may differ from the per-file `coupling` pipeline — verify
the `name()` return value before writing the JSON.)

**No engine change needed.** Write JSON, delete `src/audit/analyzers/coupling.rs`, remove from
`analyzers::architecture_analyzers()` in `src/audit/analyzers/mod.rs`.

#### Phase C: Language-Specific Tech Debt Pipelines — Language by Language

Target: The 7-12 language-specific tech_debt pipelines per language. These are not shared across
languages. Recommended order: start with languages where the pipelines are simplest (PHP has 7,
TypeScript has 11) and work toward the most complex (Python, Go, Rust which use graph-dependent
pipelines).

Suggested sub-order within Phase C:
1. PHP tech_debt (7 pipelines, mostly `NodePipeline` — simpler detection logic)
2. JavaScript tech_debt (12 pipelines)
3. TypeScript tech_debt (11 pipelines)
4. C tech_debt (12 pipelines, graph-dependent, C-idiom patterns)
5. C++ tech_debt (12 pipelines, graph-dependent)
6. Java tech_debt (11 pipelines, graph-dependent)
7. C# tech_debt (12 pipelines, graph-dependent)
8. Go tech_debt (10 pipelines, graph-dependent)
9. Python tech_debt (12 pipelines, all `GraphPipeline`)
10. Rust tech_debt (10 pipelines, all `GraphPipeline`, most complex to migrate)

#### Phase D: Security Pipelines — Language by Language

Target: 7-9 security pipelines per language. Security patterns are the most language-specific
and the most likely to require careful detection logic review (injection, deserialization, etc.).
Same language ordering as Phase C.

#### Phase E: Cleanup

Target: Delete `src/audit/primitives/` helpers that are no longer referenced, consolidate or
delete `src/audit/analyzers/` if all three `ProjectAnalyzers` are migrated, remove dead imports
from `src/audit/mod.rs` and `src/audit/engine.rs`.

Run `cargo test` and verify integration test suite covers critical pipeline behaviors.

---

## Critical Engine Change: Per-File Override Suppression

The current engine does not suppress per-file Rust pipelines when a JSON file with the same
name exists. This is the single most important architectural decision for the migration:

**Option 1 — Delete-first (no engine change):** Write JSON, immediately delete Rust file in
same commit. No duplication risk. Higher risk of regressions if JSON is wrong.

**Option 2 — Override-then-delete (one engine change):** Add suppression logic to the
per-file rayon loop. Deploy JSON shadowing Rust. Validate. Then delete Rust in a follow-up
commit. Lower regression risk, requires one targeted engine.rs change.

Recommendation: Option 2 for the high-volume shared pipelines (Phase A). Option 1 is
acceptable for language-specific pipelines (Phases C–D) where each file can be reviewed
individually before deletion.

The engine change is a single `retain` call:
```rust
// In engine.rs, after building lang_pipelines vector, before pipeline_filter:
lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()));
```

This makes the `json_pipeline_names` HashSet suppress per-file pipelines, matching the
behavior already implemented for `ProjectAnalyzers`.

---

## Dependency Order — What Requires What

```
Phase A (shared) ──────────────────────────────────────────────────────────┐
  requires: engine.rs one-line change, 10 JSON files, 100 Rust deletions   │
  unblocks: everything else (establishes JSON format understanding)         │
                                                                            │
Phase B (coupling ProjectAnalyzer) ────────────────────────────────────────┤
  requires: nothing (JSON override already works for ProjectAnalyzers)      │
  unblocks: cleaner analyzer module                                         │
                                                                            │
Phase C (tech_debt, per language) ─────────────────────────────────────────┤
  requires: Phase A complete (JSON format established)                      │
  blocked by: nothing else; each language is independent                    │
                                                                            │
Phase D (security, per language) ──────────────────────────────────────────┤
  requires: Phase A complete                                                │
  blocked by: nothing else; each language is independent                    │
                                                                            │
Phase E (cleanup) ──────────────────────────────────────────────────────────┘
  requires: Phases A–D complete
  unblocks: final cargo test pass, removal of dead code
```

Phases C and D are fully parallelizable at the language level. If two contributors work on
different languages in the same phase, there are no merge conflicts (each language lives in
its own subdirectory).

---

## What Cannot Be Migrated to JSON (Out of Scope)

Pipelines marked as "GraphPipeline" in audit_plans that require:
- CFG-level analysis (e.g., Rust `panic_detection` needing CFG Guard statements for
  `is_some()` guard detection)
- Taint propagation (`TaintEngine` FlowsTo paths)
- Resource lifecycle tracking (`ResourceAnalyzer` Acquires/ReleasedBy edges)

The current `GraphStage` vocabulary in `graph/pipeline.rs` supports only: `select`, `filter`,
`group_by`, `count`, `max_depth`, `find_cycles`, `ratio`, `flag`, and `traverse`. There are
no CFG traversal, taint path, or resource lifecycle stages.

Affected pipelines (examples from audit_plans): `panic_detection` (CFG-based guard awareness),
`async_blocking` (transitive blocking via CFG), Python `command_injection` (taint propagation).

For these, the JSON file can implement the simpler version (tree-sitter pattern matching via
graph node attributes) and skip the CFG/taint enhancement described in audit_plans. The
audit_plans' "Replacement Pipeline Design" sections describe ideal future state; the migration
target is correctness parity with the existing Rust implementation, not the full enhancement.

---

## Test Migration Strategy

**Current state:** 2,172 `#[test]` functions in `src/audit/pipelines/`. These are unit tests
for Rust implementations. When Rust files are deleted, tests vanish.

**Target state:** Integration tests in `tests/audit_json_integration.rs` that:
1. Write a minimal code snippet to a `tempdir`
2. Build a `Workspace` from it
3. Build a `CodeGraph`
4. Run `AuditEngine` with the relevant `PipelineSelector` and `pipeline_filter`
5. Assert the expected finding pattern appears (or does not appear for clean code)

One integration test per pipeline name is sufficient. The test validates the pipeline fires
and produces a finding with the correct `pattern` field value. It does not need to test every
edge case from the Rust unit tests — the audit_plans document the edge cases; implementation
correctness is validated by running against real codebases.

**Minimum viable test set per JSON pipeline:**
- `test_{pipeline_name}_fires_on_positive_case`: input with violation, assert finding with
  correct `pattern` and `pipeline` name
- `test_{pipeline_name}_clean_code_no_finding`: input without violation, assert empty findings

These 2 tests × 163 pipeline names = ~326 integration tests total, replacing 2,172 unit tests.

---

## Architectural Constraints and Invariants

**Must not change:**
- Pipeline names. They appear in CLI output, API responses, `--pipeline` filter flags, and
  user-facing documentation. Pipeline name stability is a compatibility requirement.
- `AuditFinding` structure. `pipeline`, `pattern`, `severity`, `file_path`, `line`, `message`,
  `remediation` fields must all be present with same semantics.
- `PipelineSelector` routing. The category → selector mapping (TechDebt, Complexity, etc.)
  determines which pipelines run for each `audit` subcommand. JSON files declare their category
  via the `category` field — this must match what the engine expects for the selector.

**Can change:**
- Finding messages (improve wording vs current Rust implementations)
- Detection thresholds (improve per audit_plans analysis)
- Severity levels (improve per audit_plans graduation recommendations)
- Internal Rust helpers in `primitives/` and `analyzers/` after their last consumer is removed

**Category field values in JSON must match engine routing:**

| `PipelineSelector` | Expected `category` string in JSON |
|---|---|
| TechDebt | `"tech-debt"` (or check engine routing — verify against `discover_json_audits` category handling) |
| Complexity | `"complexity"` |
| CodeStyle | `"code-style"` |
| Security | `"security"` |
| Scalability | `"scalability"` |
| Architecture | `"architecture"` |

Note: the engine's `discover_json_audits` returns ALL JSON audits and runs them after Rust
pipelines with no category filtering — category is used only in the JSON file as metadata, not
for engine routing. The pipeline MUST still appear in the correct language's `{category}_pipelines()`
function (which returns it as a registered Rust pipeline) for per-file execution to be possible.
Once Rust files are deleted and the category function returns `vec![]`, JSON takes over.

---

## Summary

The migration is architecturally straightforward because the engine already supports dual-path
execution. The main structural requirement is ensuring duplication cannot occur when both a
Rust and JSON pipeline with the same name exist simultaneously. This requires one targeted
change to `engine.rs`. After that change, the migration is purely additive (write JSON) followed
by subtractive (delete Rust) work with no cross-cutting dependencies between languages.

The 10 shared pipeline names represent the fastest return: 10 JSON files remove 100 Rust files
and 100 category function registrations. This phase alone handles ~34% of the total Rust
pipeline files. The remaining 198 Rust files are language-specific and can be migrated in
parallel by category (security, tech_debt) or language.

---

*Research date: 2026-04-16*
