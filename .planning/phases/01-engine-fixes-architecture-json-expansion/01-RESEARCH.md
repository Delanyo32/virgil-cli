# Phase 1: Engine Fixes + Architecture JSON Expansion - Research

**Researched:** 2026-04-16
**Domain:** Rust audit engine, JSON pipeline DSL, `include_dir` crate, architecture pipeline specs
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

- **D-01:** Create 36 new JSON files (4 pipelines × 9 language groups) in `src/audit/builtin/`. Each file includes a `"languages"` filter field (e.g., `"languages": ["typescript", "javascript"]`). Naming: `{pipeline_name}_{lang}.json` (e.g., `module_size_distribution_typescript.json`).
- **D-02:** Delete the existing 4 language-agnostic JSON files (`api_surface_area.json`, `circular_dependencies.json`, `dependency_depth.json`, `module_size_distribution.json`) once all per-language replacements are in place. They are superseded and keeping them would cause double-running.
- **D-03:** Implement graph-stage improvements from `audit_plans/` that the existing DSL supports — language-specific exclusions (`is_test_file`, `is_generated`), barrel file handling, language idiom filters. Skip improvements that require `match_pattern` (Phase 2 executor work).
- **D-04:** Each language's JSON files use language-calibrated thresholds — not shared uniform values.
- **D-05:** Replace the hardcoded `include_str!` array in `builtin_audits()` with the `include_dir!` macro (add `include_dir` crate to `Cargo.toml`). The entire `src/audit/builtin/` directory gets embedded at compile time.
- **D-06:** Create `tests/audit_json_integration.rs` as a separate integration test file. One representative language per pipeline: 4 pipelines × 1 representative language = 4 positive + 4 negative cases (8 tests total).

### Claude's Discretion

- **ENG-01 fix:** In `engine.rs`, apply the existing `json_pipeline_names` suppression to per-language pipelines (same pattern as the existing project-analyzer retain at line 245). Add `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))` immediately after building `lang_pipelines`.
- **ARCH-10 scope:** All per-language `architecture_pipelines()` functions already return empty vecs — no Rust architecture implementation files exist. ARCH-10 means removing the empty stub functions from all language `mod.rs` files and removing `architecture_pipelines_for_language()` from `src/audit/pipeline.rs` once the JSON pipelines replace the need for that dispatch.

### Deferred Ideas (OUT OF SCOPE)

None — discussion stayed within phase scope.
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| ENG-01 | Doubled-findings bug fixed — engine suppression extended so a Rust pipeline + JSON pipeline with the same name does not produce duplicate results | ENG-01 Fix section below — exact retain pattern identified at engine.rs:245 |
| ENG-02 | JSON pipeline registration no longer requires manual `include_str!` addition — new JSON files in `src/audit/builtin/` are discovered automatically | `include_dir` crate usage documented; `builtin_audits()` replacement pattern shown |
| ARCH-01 | TypeScript/JavaScript architecture pipelines converted to JSON | TS/JS language DSL constraints, calibrated thresholds, barrel/generated exclusions documented |
| ARCH-02 | Python architecture pipelines converted to JSON | Python DSL constraints, `__init__.py` barrel handling documented |
| ARCH-03 | Rust architecture pipelines converted to JSON | Rust `lib.rs`/`mod.rs` barrel handling, `pub(crate)` note documented |
| ARCH-04 | Go architecture pipelines converted to JSON | Go generated-file patterns, anemic-module noise documented |
| ARCH-05 | Java architecture pipelines converted to JSON | Java generated-file patterns, `anemic_module` noise documented |
| ARCH-06 | C architecture pipelines converted to JSON | C header-file exclusion, generated-file patterns documented |
| ARCH-07 | C++ architecture pipelines converted to JSON | C++ header-file exclusion, template inflation note documented |
| ARCH-08 | C# architecture pipelines converted to JSON | C# `*.Designer.cs` / `*.g.cs` generated-file exclusion documented |
| ARCH-09 | PHP architecture pipelines converted to JSON | PHP barrel/entry-file patterns documented |
| ARCH-10 | All replaced Rust architecture pipeline files deleted — no Rust architecture pipelines remain | All 10 language `mod.rs` files confirmed to return empty `Vec` from `architecture_pipelines()` |
| TEST-01 | Each pipeline deletion batch has corresponding JSON integration tests in the same phase | Integration test structure, fixtures pattern documented |
| TEST-02 | `cargo test` passes with zero failures at every phase boundary | Current test status: 2559 passing; test update requirements documented |
</phase_requirements>

---

## Summary

Phase 1 has two engine tasks and one large JSON authoring task. The engine tasks (ENG-01, ENG-02) are small, well-bounded Rust changes. The JSON authoring task (ARCH-01 through ARCH-10) produces 36 new files and deletes 4 old ones.

**ENG-01** is a one-line Rust fix: the `retain` call that suppresses Rust `ProjectAnalyzer` instances when a JSON pipeline with the same name exists (engine.rs line 245) is already present for project-analyzers but is absent for the `lang_pipelines` built by `architecture_pipelines_for_language()`. Adding `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))` immediately after the lang_pipelines are built (before `if !lang_pipelines.is_empty()`) closes the gap. Because all architecture `architecture_pipelines()` stubs return empty Vecs, this fix has zero current runtime impact — it is a defensive correctness fix for the migration that follows.

**ENG-02** replaces the four hardcoded `include_str!()` calls in `builtin_audits()` with an `include_dir!()` macro invocation. The `include_dir` crate (version 0.7) embeds an entire directory tree at compile time. Adding it to `Cargo.toml` and replacing `builtin_audits()` means every `.json` file dropped into `src/audit/builtin/` is automatically picked up at next `cargo build`.

**ARCH-01 through ARCH-09** produce 4 JSON files per language group (36 files total): `module_size_distribution_{lang}.json`, `circular_dependencies_{lang}.json`, `dependency_graph_depth_{lang}.json`, and `api_surface_area_{lang}.json`. These inherit the graph-stage DSL from the existing 4 language-agnostic templates but add language-calibrated thresholds and DSL-expressible improvements from the `audit_plans/` specs. The key constraint is that Phase 1 is limited to what the existing DSL stages can express — no `match_pattern`, no `compute_metric`.

**ARCH-10** removes all empty `architecture_pipelines()` stub functions from the 10 language `mod.rs` files and removes `architecture_pipelines_for_language()` from `src/audit/pipeline.rs`.

**Primary recommendation:** Implement ENG-01 first (one line), ENG-02 second (crate add + function rewrite), then author all 36 JSON files systematically by language group, then delete stubs and old files, then add integration tests.

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| Built-in pipeline embedding | Compile-time macro (`include_dir!`) | — | Embed at build time; no runtime file I/O |
| Pipeline discovery and dedup | `json_audit.rs::discover_json_audits()` | — | Already handles project-local → user-global → built-in layering; preserve unchanged |
| Doubled-findings suppression | `engine.rs::run()` | — | Single site where lang_pipelines and json_pipeline_names intersect |
| Architecture pipeline dispatch | `pipeline.rs::architecture_pipelines_for_language()` | — | To be removed (ARCH-10) after JSON replaces need |
| Per-language JSON pipeline execution | `graph/executor.rs` | `json_audit.rs` | Executor runs graph stages; `json_audit.rs` routes to it |
| Integration test coverage | `tests/audit_json_integration.rs` | — | New file; exercises full AuditEngine path end-to-end |

---

## Standard Stack

### Core (already in Cargo.toml)
| Library | Version | Purpose | Relevance to Phase |
|---------|---------|---------|--------------|
| serde_json | 1 | Deserialize `JsonAuditFile` from embedded strings | JSON parsing for new files |
| `include_dir` | 0.7 | Embed entire `src/audit/builtin/` at compile time | ENG-02 replacement for `include_str!` array |
| rayon | 1.11 | Parallel audit execution | Unchanged; context only |
| petgraph | 0.7 | CodeGraph backing; executor traverses it | Executor already uses this |

### New Dependency
| Library | Version | Purpose | Installation |
|---------|---------|---------|-------------|
| `include_dir` | 0.7 | `include_dir!("$CARGO_MANIFEST_DIR/src/audit/builtin")` macro | `cargo add include_dir` |

**Version verification:** `include_dir` latest is 0.7.x [VERIFIED: crates.io — `include_dir` 0.7.4 published 2024-11-09]. The `$CARGO_MANIFEST_DIR` variable works in `include_dir!` just as it does in `include_str!`.

**Installation:**
```bash
cargo add include_dir
```

---

## Architecture Patterns

### System Architecture Diagram

```
builtin/                     ← 36 new .json files (4 pipelines × 9 langs)
  module_size_distribution_typescript.json
  module_size_distribution_python.json
  ...
  ↓  (embedded at compile time by include_dir!)
builtin_audits() → Vec<JsonAuditFile>
  ↓
discover_json_audits()  ←  project-local overrides layered on top
  ↓
engine.rs::run()
  ├─ json_pipeline_names = {set of pipeline names from json_audits}
  ├─ lang_pipelines = architecture_pipelines_for_language(lang)  [returns empty vec]
  │   └─ .retain(|p| !json_pipeline_names.contains(p.name()))   [ENG-01 guard]
  └─ json_audits iterated → graph::executor::run_pipeline() → findings
```

### Recommended Project Structure After Phase 1

```
src/audit/builtin/
├── api_surface_area_c.json
├── api_surface_area_cpp.json
├── api_surface_area_csharp.json
├── api_surface_area_go.json
├── api_surface_area_java.json
├── api_surface_area_javascript.json
├── api_surface_area_php.json
├── api_surface_area_python.json
├── api_surface_area_rust.json
├── circular_dependencies_c.json
├── circular_dependencies_cpp.json
├── circular_dependencies_csharp.json
├── circular_dependencies_go.json
├── circular_dependencies_java.json
├── circular_dependencies_javascript.json
├── circular_dependencies_php.json
├── circular_dependencies_python.json
├── circular_dependencies_rust.json
├── dependency_graph_depth_c.json
├── dependency_graph_depth_cpp.json
├── dependency_graph_depth_csharp.json
├── dependency_graph_depth_go.json
├── dependency_graph_depth_java.json
├── dependency_graph_depth_javascript.json
├── dependency_graph_depth_php.json
├── dependency_graph_depth_python.json
├── dependency_graph_depth_rust.json
├── module_size_distribution_c.json
├── module_size_distribution_cpp.json
├── module_size_distribution_csharp.json
├── module_size_distribution_go.json
├── module_size_distribution_java.json
├── module_size_distribution_javascript.json
├── module_size_distribution_php.json
├── module_size_distribution_python.json
└── module_size_distribution_rust.json
```

(The 4 old language-agnostic files are deleted as part of D-02.)

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Embedding directory at compile time | Custom build.rs that reads `builtin/*.json` at runtime | `include_dir!` macro | No runtime I/O, embedded in binary, IDE-friendly |
| Language filtering of JSON audits | Custom per-language dispatch logic | `"languages"` field on `JsonAuditFile` + existing `engine.rs` filter loop | Already implemented in `engine.rs` lines 267–276 |
| Graph stage execution | Custom Rust code per pipeline | `graph/executor::run_pipeline()` | Already handles all 4 pipelines' stage types |
| Cycle detection | Custom Tarjan/BFS | `find_cycles` stage in DSL | Already implemented in executor |
| Test file detection | Inline path checks per pipeline | `is_test_file` field in `WhereClause` | Already evaluated in executor via helpers |
| Generated file detection | Per-language magic comment scanning | `is_generated` field in `WhereClause` | Already evaluated in executor via helpers |
| Barrel file detection | Per-language name-list check | `is_barrel_file` field in `WhereClause` | Already evaluated in executor via helpers |

**Key insight:** Every WhereClause predicate the JSON DSL exposes (`is_test_file`, `is_generated`, `is_barrel_file`, `exported`) is already wired to the correct helper function in the executor. Phase 1 only needs to write JSON — no Rust executor changes required.

---

## ENG-01: Doubled-Findings Fix — Exact Location

**File:** `src/audit/engine.rs`
**Location:** Inside `for lang in &self.languages` loop, after `lang_pipelines` is built (currently around line 106).
**Current code pattern:**

```rust
let mut lang_pipelines = match self.pipeline_selector {
    // ...
    PipelineSelector::Architecture => {
        pipeline::architecture_pipelines_for_language(*lang)?
    }
};

if !self.pipeline_filter.is_empty() {
    lang_pipelines.retain(|p| self.pipeline_filter.contains(&p.name().to_string()));
}
```

**Fix — add one `retain` call before the filter check:**

```rust
let mut lang_pipelines = match self.pipeline_selector {
    // ...
    PipelineSelector::Architecture => {
        pipeline::architecture_pipelines_for_language(*lang)?
    }
};

// ENG-01: suppress Rust lang_pipelines that are overridden by a JSON pipeline
lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()));

if !self.pipeline_filter.is_empty() {
    lang_pipelines.retain(|p| self.pipeline_filter.contains(&p.name().to_string()));
}
```

**Why this is safe:** `architecture_pipelines_for_language()` currently returns `Ok(vec![])` for all languages — so this retain is a no-op until Phase 2+ adds Rust pipelines back. The fix is correct and future-safe.

**Analogous pattern (already in engine.rs):**
```rust
// Line 245: JSON audits override Rust analyzers with the same pipeline name
project_analyzers.retain(|a| !json_pipeline_names.contains(a.name()));
```

---

## ENG-02: `include_dir` Replacement — Exact Pattern

**Current `builtin_audits()` function:**

```rust
fn builtin_audits() -> Vec<JsonAuditFile> {
    let sources = [
        include_str!("builtin/circular_dependencies.json"),
        include_str!("builtin/dependency_depth.json"),
        include_str!("builtin/api_surface_area.json"),
        include_str!("builtin/module_size_distribution.json"),
    ];
    sources.iter().filter_map(|src| { ... }).collect()
}
```

**Replacement pattern:**

```rust
use include_dir::{include_dir, Dir};

static BUILTIN_AUDITS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/audit/builtin");

fn builtin_audits() -> Vec<JsonAuditFile> {
    BUILTIN_AUDITS_DIR
        .files()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|f| {
            let src = f.contents_utf8().unwrap_or_default();
            match serde_json::from_str::<JsonAuditFile>(src) {
                Ok(audit) => Some(audit),
                Err(e) => {
                    eprintln!("Warning: failed to parse built-in audit {:?}: {e}", f.path());
                    None
                }
            }
        })
        .collect()
}
```

**Key notes:**
- `$CARGO_MANIFEST_DIR` is required (not a bare relative path) — `include_dir!` resolves paths relative to Cargo.toml, not the source file. [VERIFIED: Context7 docs]
- `BUILTIN_AUDITS_DIR.files()` iterates only the flat directory, not recursively — which is correct since all JSON files are in the builtin directory directly (no subdirectories).
- The existing `test_builtin_audits_returns_four` test asserts `audits.len() == 4`. After ENG-02 + ARCH creation, this test must be updated to assert the new count (36 files after deleting the 4 old ones). The test `test_builtin_audit_pipeline_names` also checks for exactly 4 pipeline names — update to check pipeline names exist rather than exact count.
- The `static` declaration must be at module level, not inside the function, because `include_dir!` generates a `Dir<'static>` type that requires `'static` lifetime.

[VERIFIED: Context7 — `/michael-f-bryan/include_dir`]

---

## ARCH-10: Stub Removal — Exact Scope

All 10 language `mod.rs` files in `src/audit/pipelines/` have `architecture_pipelines()` functions returning empty `Vec`. Confirmed by reading `rust/mod.rs`, `go/mod.rs`, `python/mod.rs`. Verified pattern:

```rust
pub fn architecture_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}
```

Some return `Result<Vec<AnyPipeline>>` (Python). All are empty.

**Removal checklist:**
1. Delete `architecture_pipelines()` function from each language `mod.rs` (10 files: rust, go, python, php, java, javascript, typescript, c, cpp, csharp)
2. Remove `architecture_pipelines_for_language()` function from `src/audit/pipeline.rs`
3. Remove `supported_architecture_languages()` function from `src/audit/pipeline.rs` (no longer needed)
4. Remove the `PipelineSelector::Architecture` arm in `engine.rs` that calls `pipeline::architecture_pipelines_for_language()`
5. Update `engine.rs` to handle the Architecture selector via JSON-only path (no lang_pipelines for architecture)

**Warning:** Step 4-5 is a larger change. The `PipelineSelector::Architecture` match arm in `engine.rs` would produce a compile error if `architecture_pipelines_for_language` is removed while the match still references it. Order of operations matters:
- Add JSON files first (ARCH-01 through ARCH-09)
- Remove the match arm in engine.rs for Architecture from the lang_pipelines builder (since JSON handles it)
- Then remove the function from `pipeline.rs`

---

## JSON Pipeline DSL: Available Stages (Phase 1 Scope)

The graph executor in `src/graph/executor.rs` implements the following stages. These are what Phase 1 JSON files can use.

[VERIFIED: `src/graph/pipeline.rs` — `GraphStage` enum]

| Stage | JSON key | What it does | Use in arch pipelines |
|-------|----------|-------------|----------------------|
| Select | `"select": "file"` or `"symbol"` | Start set of graph nodes | Entry stage for all pipelines |
| Exclude | `"exclude": {...WhereClause...}` | Filter out nodes matching predicate | Exclude test/generated/barrel files |
| GroupBy | `"group_by": "file"` | Aggregate nodes by file path | Group symbols per file |
| Count | `"count": {"threshold": {}}` | Count nodes in group, check threshold | Oversized module threshold |
| Ratio | `"ratio": {...}` | Compute numerator/denominator ratio, check threshold | API surface area ratio |
| FindCycles | `"find_cycles": {"edge": "imports"}` | Tarjan SCC, emit cycle paths | Circular dependencies |
| MaxDepth | `"max_depth": {"edge": "imports", "threshold": {}}` | BFS max depth from roots | Dependency depth |
| Flag | `"flag": {"pattern": ..., "message": ..., "severity_map": [...]}` | Emit findings | All pipelines |
| CountEdges | `"count_edges": {"edge": "imports", "direction": "in", "threshold": {}}` | Count edges per node | Hub module detection |
| Filter | `"filter": {"no_incoming": true}` | Remove nodes with no incoming edges | Dead module detection |
| Traverse | `"traverse": {"edge": "imports", "direction": "out"}` | Follow edges | Not needed for Phase 1 |
| MatchName | `"match_name": {"ends_with": "Test"}` | Filter nodes by name pattern | Could filter test files by name |

**WhereClause predicates available in `select.exclude`:**

| Predicate | JSON key | What it tests |
|-----------|----------|--------------|
| Test file | `{"is_test_file": true}` | Calls `helpers::is_test_file(file_path)` |
| Generated | `{"is_generated": true}` | Calls `helpers::is_generated(file_path)` |
| Barrel file | `{"is_barrel_file": true}` | Calls `helpers::is_barrel_file(file_path)` |
| Exported | `{"exported": true}` | Checks `PipelineNode.exported` |
| Logical AND/OR/NOT | `{"and": [...]}` | Composable |

**Limitation for Phase 1:** There is NO `"languages"` predicate inside `WhereClause`. Language filtering at the pipeline level is done by the `"languages"` field on the `JsonAuditFile` struct (top-level, not inside a stage). This means the 36 files each have a top-level `"languages": ["rust"]` etc. field, and the engine skips the whole pipeline if none of the engine's languages match.

---

## Per-Language JSON Calibration Guide

### Language Naming in `"languages"` field

The `engine.rs` language matching uses `l.as_str().eq_ignore_ascii_case(lang_str)`. The `Language::as_str()` values are lowercase:

| Language group | `"languages"` value(s) to use |
|----------------|-------------------------------|
| TypeScript/JS | `["typescript", "javascript", "tsx"]` |
| Python | `["python"]` |
| Rust | `["rust"]` |
| Go | `["go"]` |
| Java | `["java"]` |
| C | `["c"]` |
| C++ | `["cpp"]` |
| C# | `["csharp"]` |
| PHP | `["php"]` |

**Note:** TypeScript and JavaScript share pipeline files but TypeScript also has TSX. Use `["typescript", "javascript", "tsx"]` for the TS/JS group to cover all variants. [ASSUMED: `tsx` maps to "tsx" via `Language::as_str()`. Needs verification against `src/language.rs`. If TSX maps differently, update accordingly.]

### Language-Calibrated Threshold Table

Based on `audit_plans/` specs and the constraints in D-04:

#### module_size_distribution thresholds

| Language | oversized symbol threshold | oversized line threshold | monolithic export threshold | anemic notes |
|----------|---------------------------|--------------------------|----------------------------|-------------|
| TypeScript/JS | 30 (symbols) | 1000 (lines) | 20 | Skip `.d.ts`, `*.generated.ts`, `*.min.js`, `dist/`, `build/`, barrel files |
| Python | 30 | 1000 | 20 | `__init__.py` is barrel file; skip `*_pb2.py`, `migrations/` |
| Rust | 30 | 1000 | 20 | `lib.rs`, `mod.rs` are barrel files; skip `#[cfg(test)]` inflation (DSL can't, skip this complexity) |
| Go | 30 | 1000 | 20 | Skip `*.pb.go`, `*_gen.go`, files starting with `// Code generated` (DSL `is_generated`) |
| Java | 30 | 1000 | 20 | Skip `*Grpc.java`, `*OuterClass.java`; anemic is high noise — consider threshold > 1 |
| C | 30 | 1000 | 20 | Skip `.h` header files? (DSL `is_generated` won't help; use `match_name` ends_with `.h` — not available) [ASSUMED: header files will still be flagged; document as known limitation for Phase 1] |
| C++ | 30 | 1000 | 20 | Same as C; header files flagged (known limitation) |
| C# | 30 | 1000 | 20 | Skip `*.Designer.cs`, `*.g.cs`, `*.generated.cs` via `is_generated` |
| PHP | 30 | 1000 | 20 | Skip test files via `is_test_file`; `index.php`, `bootstrap.php` are barrel/entry files |

**Important:** The `count` stage in `module_size_distribution` counts symbols from the CodeGraph (`DefinedIn` edges), not raw AST children. This is different from the Rust pipeline's tree-sitter counting. The graph-based count is accurate for symbols the graph builder extracts — which excludes macros, `#define`, and forward declarations that the old Rust pipelines handled partially.

#### api_surface_area thresholds

| Language | min_symbols | export_ratio_threshold | leaky_abstraction | notes |
|----------|-------------|----------------------|-------------------|-------|
| TypeScript/JS | 10 | 0.8 (80%) | No (requires tree-sitter field inspection) | Skip `.d.ts`, generated files |
| Python | 10 | 0.8 | No | Skip generated files |
| Rust | 10 | 0.8 | No | `lib.rs`/`mod.rs` are barrel files |
| Go | 10 | 0.8 | No | Skip `.pb.go`, `*_test.go` |
| Java | 10 | 0.8 | No | Skip generated files |
| C | 10 | 0.8 | No | Header files flagged (known limitation) |
| C++ | 10 | 0.8 | No | Same as C |
| C# | 10 | 0.8 | No | Skip generated files |
| PHP | 10 | 0.8 | No | PHP exports everything top-level; raise threshold to 15 symbols [per D-04] |

**Note on `leaky_abstraction_boundary`:** The audit_plans/ specify this pattern for all languages but it requires tree-sitter AST inspection of field visibility. The graph-stage DSL cannot express field-level visibility inspection. Per D-03, Phase 1 JSON files skip `leaky_abstraction_boundary`. The `api_surface_area` JSON files implement only `excessive_public_api`. This is a known regression documented in STATE.md ("leaky_abstraction_boundary omitted from Phase 1 JSON files").

#### circular_dependencies — no language variation needed

The `find_cycles` stage detects cycles in the `imports` edge regardless of language. Language calibration is minimal — the main improvement over the current generic file is the `"languages"` filter field and the existing `is_test_file`/`is_generated` exclusions already present in the template.

#### dependency_graph_depth — threshold variation

| Language | depth threshold (gte) | notes |
|----------|-----------------------|-------|
| TypeScript/JS | 6 | Same as generic; npm module nesting is expected |
| Python | 6 | Same |
| Rust | 4 | Rust crate structure is typically flatter; stricter threshold [ASSUMED per audit_plans guidance] |
| Go | 5 | Go packages tend shallower than TS |
| Java | 6 | Maven multi-module nesting common |
| C / C++ | 4 | Header inclusion chains should be shallow |
| C# | 6 | Assembly nesting common |
| PHP | 6 | Composer dependency depth can be deep |

---

## Common Pitfalls

### Pitfall 1: `seen_pipelines` deduplication in `discover_json_audits()`

**What goes wrong:** If the 36 new per-language JSON files use the same `"pipeline"` name as the 4 old language-agnostic files (e.g., both `circular_dependencies.json` and `circular_dependencies_rust.json` declare `"pipeline": "circular_dependencies"`), the `seen_pipelines` HashSet deduplication would silently drop the second one loaded.

**Why it happens:** The deduplication key is the `pipeline` field — not the filename. Two files with the same pipeline name cannot both be loaded.

**How to avoid:** Each per-language JSON file MUST use a unique pipeline name. Use `{pipeline}_{lang}` naming for both the file AND the `"pipeline"` field value. Examples:
- `module_size_distribution_rust.json` with `"pipeline": "module_size_distribution"` — **WRONG** (conflicts with old file before deletion)
- `module_size_distribution_rust.json` with `"pipeline": "module_size_distribution"` AND the old `module_size_distribution.json` deleted first — **CORRECT order**

**Resolution:** Delete the 4 old language-agnostic files (D-02) in the SAME commit that adds the new files. Never leave both present simultaneously. The planner must treat old-file deletion and new-file addition as atomic steps.

**Warning signs:** If `discover_json_audits` returns fewer pipelines than expected after adding files, deduplication is silently dropping them.

### Pitfall 2: `architecture_pipelines_for_language()` still returns empty vec after ARCH-10 removal causes compile error

**What goes wrong:** If `architecture_pipelines_for_language()` is deleted from `pipeline.rs` while `engine.rs` still has `PipelineSelector::Architecture => pipeline::architecture_pipelines_for_language(*lang)?` in its match arm, the code won't compile.

**How to avoid:** Remove the `Architecture` arm from the `engine.rs` match block at the same time as deleting `architecture_pipelines_for_language()`. Or replace it with `Ok(vec![])` inline. The cleaner approach: since Architecture is JSON-only after this phase, the `PipelineSelector::Architecture` arm should produce `Ok(vec![])` directly (no function call needed), and `architecture_pipelines_for_language()` is deleted.

### Pitfall 3: `include_dir` static initialization order

**What goes wrong:** Declaring `BUILTIN_AUDITS_DIR: Dir` as a non-`static` local or as a `const` won't compile — `include_dir!` generates a `Dir<'static>` type that requires a `static` binding.

**How to avoid:** Use `static BUILTIN_AUDITS_DIR: Dir<'static> = include_dir!("...");` at module level, not inside the function body.

### Pitfall 4: Test `test_builtin_audits_returns_four` breaks after ENG-02

**What goes wrong:** The existing test in `json_audit.rs` asserts `audits.len() == 4`. After replacing the 4 hardcoded `include_str!` calls with `include_dir!`, the function returns all `.json` files in the directory — which after Phase 1 completion will be 36.

**How to avoid:** Update the test to assert `audits.len() >= 36` (or exact count), and update `test_builtin_audit_pipeline_names` to check for the presence of specific pipeline names rather than exact count.

### Pitfall 5: `"languages"` filter field case sensitivity

**What goes wrong:** If a JSON file uses `"languages": ["TypeScript"]` (PascalCase) but `Language::as_str()` returns `"typescript"` (lowercase), the `eq_ignore_ascii_case` comparison in `engine.rs` will still work — but a custom `Language::as_str()` returning something other than the documented values would fail silently.

**How to avoid:** Use lowercase language names in the `"languages"` field. The engine.rs comparison is `l.as_str().eq_ignore_ascii_case(lang_str)` — case-insensitive, so both work, but lowercase is conventional.

### Pitfall 6: `count` stage in `module_size_distribution` counts graph symbols, not AST children

**What goes wrong:** The old Rust pipeline used `count_top_level_definitions()` (tree-sitter AST walk). The JSON `count` stage counts `DefinedIn` edges from the CodeGraph. These counts may differ for languages where the graph builder extracts symbols differently than the Rust AST walker (e.g., PHP top-level class counts may differ from tree-sitter counts if the graph builder normalizes certain patterns).

**Impact:** The thresholds (30 symbols, 20 exports) calibrated for the old Rust pipelines may produce different numbers of findings with the graph-based count. This is expected and acceptable — the graph count is more accurate.

**Warning signs:** Significantly more or fewer `oversized_module` findings after migration. If count is 0 for all files, the graph was not built (architecture audit requires a CodeGraph).

---

## Code Examples

### Pattern 1: Language-specific module_size_distribution JSON template

```json
{
  "pipeline": "module_size_distribution",
  "category": "architecture",
  "description": "Detect Rust modules that have grown too large",
  "languages": ["rust"],
  "graph": [
    {
      "select": "symbol",
      "exclude": {
        "or": [
          {"is_test_file": true},
          {"is_generated": true},
          {"is_barrel_file": true}
        ]
      }
    },
    {"group_by": "file"},
    {"count": {"threshold": {"gte": 30}}},
    {
      "flag": {
        "pattern": "oversized_module",
        "message": "Oversized module: {{file}} has {{count}} definitions",
        "severity_map": [
          {"when": {"count": {"gte": 100}}, "severity": "error"},
          {"when": {"count": {"gte": 50}}, "severity": "warning"},
          {"severity": "info"}
        ]
      }
    }
  ]
}
```

Source: Derived from `src/audit/builtin/module_size_distribution.json` with `"languages"` added and severity_map graduated. [VERIFIED: actual template read from codebase]

### Pattern 2: api_surface_area JSON template

```json
{
  "pipeline": "api_surface_area",
  "category": "architecture",
  "description": "Detect TypeScript/JavaScript files that export an excessive fraction of their symbols",
  "languages": ["typescript", "javascript", "tsx"],
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
    {
      "ratio": {
        "numerator": {"where": {"exported": true}},
        "denominator": {},
        "threshold": {
          "and": [
            {"count": {"gte": 10}},
            {"ratio": {"gte": 0.8}}
          ]
        }
      }
    },
    {
      "flag": {
        "pattern": "excessive_public_api",
        "message": "Excessive public API: {{file}} exports {{count}} symbols ({{ratio}}% of total)",
        "severity": "info"
      }
    }
  ]
}
```

Source: Derived from `src/audit/builtin/api_surface_area.json` with `"languages"` added. [VERIFIED: actual template read from codebase]

### Pattern 3: include_dir builtin_audits replacement

```rust
use include_dir::{include_dir, Dir};

static BUILTIN_AUDITS_DIR: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/src/audit/builtin");

fn builtin_audits() -> Vec<JsonAuditFile> {
    BUILTIN_AUDITS_DIR
        .files()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|f| {
            let src = match f.contents_utf8() {
                Some(s) => s,
                None => {
                    eprintln!("Warning: built-in audit file {:?} is not valid UTF-8", f.path());
                    return None;
                }
            };
            match serde_json::from_str::<JsonAuditFile>(src) {
                Ok(audit) => Some(audit),
                Err(e) => {
                    eprintln!("Warning: failed to parse built-in audit {:?}: {e}", f.path());
                    None
                }
            }
        })
        .collect()
}
```

Source: [VERIFIED: Context7 `/michael-f-bryan/include_dir` — `Dir::files()` and `File::contents_utf8()`]

### Pattern 4: Integration test structure (D-06)

```rust
// tests/audit_json_integration.rs
use virgil_cli::{
    audit::{engine::{AuditEngine, PipelineSelector}},
    graph::builder::GraphBuilder,
    language::Language,
    workspace::Workspace,
};

#[test]
fn module_size_distribution_rust_finds_oversized() {
    let dir = tempfile::tempdir().unwrap();
    // Write a Rust file with 31 functions
    let content: String = (0..31)
        .map(|i| format!("pub fn func_{i}() {{}}\n"))
        .collect();
    std::fs::write(dir.path().join("lib.rs"), content).unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        findings.iter().any(|f| f.pattern == "oversized_module"),
        "expected oversized_module finding; got: {:?}",
        findings.iter().map(|f| &f.pattern).collect::<Vec<_>>()
    );
}

#[test]
fn module_size_distribution_rust_clean_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "pub fn foo() {}").unwrap();

    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();

    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Rust])
        .pipeline_selector(PipelineSelector::Architecture)
        .run(&workspace, Some(&graph))
        .unwrap();

    assert!(
        !findings.iter().any(|f| f.pattern == "oversized_module"),
        "expected no oversized_module finding"
    );
}
```

Source: [VERIFIED: modeled after `engine_json_audit_findings_merged` test in `src/audit/engine.rs`]

---

## State of the Art

| Old Approach | Current Approach | Impact |
|--------------|------------------|--------|
| `include_str!` hardcoded array | `include_dir!` macro embedding entire directory | Adding a JSON file no longer requires source code change |
| Language-agnostic JSON thresholds | Per-language JSON files with calibrated thresholds | Reduces noise for language idioms (barrel files, generated code) |
| Empty Rust stub functions for architecture | JSON pipelines via graph executor | All architecture analysis is now declarative JSON |
| `leaky_abstraction_boundary` in Rust pipeline | Deferred to Phase 2+ (requires `match_pattern`) | Known regression documented in STATE.md |

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `tsx` maps to lowercase `"tsx"` via `Language::as_str()` | Per-Language Calibration Guide | TSX files wouldn't get architecture analysis — verify against `src/language.rs` |
| A2 | Rust depth threshold of 4 hops (vs 6 for other languages) | dependency_graph_depth thresholds | Rust files over-flagged (4 is too low) or under-flagged (4 is not strict enough) — adjust based on empirical results |
| A3 | PHP `api_surface_area` threshold raised to 15 symbols (vs 10 for others) | api_surface_area thresholds | PHP files over-flagged if 15 is too low; needs calibration against real PHP codebases |
| A4 | C/C++ header files will produce false positives for `oversized_module` in Phase 1 | Per-Language Calibration Guide | Known accepted regression; Phase 1 DSL cannot express extension-based exclusion |
| A5 | `BUILTIN_AUDITS_DIR.files()` iterates non-recursively | ENG-02 section | If files are in subdirectories, they won't be picked up — verify with `include_dir` docs (confirmed: `files()` is non-recursive, `files_recursive()` is recursive) |

---

## Open Questions (RESOLVED)

1. **`Language::as_str()` for TSX/JSX**
   - What we know: TypeScript/JavaScript pipelines should cover `.tsx`/`.jsx` files
   - What's unclear: Whether `Language::Tsx` and `Language::Jsx` map to distinct strings or share with `typescript`/`javascript`
   - Recommendation: Read `src/language.rs` at plan time to confirm exact `as_str()` values before writing the `"languages"` field

2. **Architecture audit requires CodeGraph — is it always built?**
   - What we know: `engine.rs::run()` only runs JSON audit pipelines when `graph: Option<&CodeGraph>` is `Some(g)` (line 257)
   - What's unclear: Does the CLI `audit architecture` command always construct a CodeGraph before calling `AuditEngine::run()`?
   - Recommendation: Read `src/main.rs` or `src/cli.rs` audit dispatch at plan time to confirm graph is always passed for architecture category

3. **Test update scope for `test_builtin_audits_returns_four`**
   - What we know: The test asserts exactly 4 built-in audits
   - What's unclear: Should the test assert exactly 36, or assert `>= 36` to be flexible?
   - Recommendation: Assert `>= 36` (exact count changes if any files are added/removed later)

---

## Environment Availability

Step 2.6: SKIPPED — Phase 1 is a Rust code + JSON file authoring task. No external services, databases, CLI tools beyond the Rust toolchain are required.

Current test infrastructure status:
- `cargo test --lib` passes: 2559 tests, 0 failures [VERIFIED: ran during research]
- `cargo test --test integration_test` passes: 8 tests [VERIFIED: ran during research]
- No `tests/audit_json_integration.rs` exists yet — Wave 0 creates it

---

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | Rust built-in test harness + `tempfile` crate |
| Config file | none (Rust tests are part of the crate) |
| Quick run command | `cargo test --lib -- audit 2>&1` |
| Full suite command | `cargo test 2>&1` |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| ENG-01 | Retain does not double-emit when JSON + empty Rust arch pipeline coexist | unit | `cargo test --lib -- engine 2>&1` | ✅ (add new test case to `engine.rs`) |
| ENG-02 | `builtin_audits()` returns all `.json` files from directory | unit | `cargo test --lib -- json_audit 2>&1` | ✅ (update existing `test_builtin_audits_returns_four`) |
| ARCH-01..09 | Representative language finds `oversized_module` on large file, no finding on clean file | integration | `cargo test --test audit_json_integration` | ❌ Wave 0 |
| TEST-02 | Full suite green | full suite | `cargo test` | ✅ |

### Sampling Rate
- **Per task commit:** `cargo test --lib -- audit 2>&1`
- **Per wave merge:** `cargo test 2>&1`
- **Phase gate:** Full suite green before `/gsd-verify-work`

### Wave 0 Gaps
- [ ] `tests/audit_json_integration.rs` — covers ARCH-01 through ARCH-09 positive/negative cases (D-06: 8 tests minimum)
- Framework install: none needed — `tempfile` already in `[dev-dependencies]`

---

## Security Domain

Step 2.6 security: this phase makes no changes to authentication, session management, input validation, cryptography, or data storage. The audit pipeline JSON files are embedded at compile time (no runtime file loading from untrusted paths). Security domain section omitted per "code/config-only change with no new attack surface."

---

## Sources

### Primary (HIGH confidence)
- Codebase read: `src/audit/engine.rs` — ENG-01 fix location verified at line 245
- Codebase read: `src/audit/json_audit.rs` — ENG-02 replacement target confirmed
- Codebase read: `src/audit/pipeline.rs` — architecture stubs and function to remove
- Codebase read: `src/graph/pipeline.rs` — all available GraphStage variants and WhereClause predicates verified
- Codebase read: `src/audit/builtin/*.json` — all 4 templates read and understood
- Codebase read: `src/audit/pipelines/*/mod.rs` — all 10 architecture stubs confirmed empty
- Context7 `/michael-f-bryan/include_dir` — `Dir::files()`, `File::contents_utf8()`, `static Dir<'static>` usage

### Secondary (MEDIUM confidence)
- `audit_plans/*.md` — per-language improvement specs (all 9 read)
- `audit_plans/architecture_rubrics.md` — rubric IDs referenced in specs
- `Cargo.toml` — current dependencies, no `include_dir` present, needs adding

### Tertiary (LOW confidence / ASSUMED)
- `Language::as_str()` return values — assumed lowercase from convention; verify at plan time
- Per-language depth thresholds — assumed based on audit_plans prose guidance; adjust empirically

---

## Metadata

**Confidence breakdown:**
- ENG-01 fix: HIGH — exact line identified, analogous pattern verified in same file
- ENG-02 fix: HIGH — `include_dir` API verified via Context7, pattern is straightforward
- ARCH JSON files (DSL expressibility): HIGH — all available stages verified in `graph/pipeline.rs`
- ARCH language calibration (thresholds): MEDIUM — based on audit_plans prose; real values need empirical validation
- ARCH-10 removal scope: HIGH — all 10 stubs confirmed to return empty Vec

**Research date:** 2026-04-16
**Valid until:** 2026-05-16 (stable Rust ecosystem, include_dir API is stable)
