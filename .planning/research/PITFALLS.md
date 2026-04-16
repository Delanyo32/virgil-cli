# Domain Pitfalls: JSON Audit Pipeline Migration

**Domain:** Rust-to-JSON audit pipeline migration for a static analysis CLI
**Researched:** 2026-04-16
**Source confidence:** HIGH — all findings derived from direct inspection of source files in this repo

---

## Critical Pitfalls

Mistakes that cause rewrites, regressions, or `cargo test` failures that are not immediately obvious.

---

### Pitfall 1: Rust Pipeline Still Runs After JSON File Is Added

**What goes wrong:** A developer creates `src/audit/builtin/panic_detection.json` but the Rust pipeline
`src/audit/pipelines/rust/panic_detection.rs` is never deleted. Both run. Users get duplicate findings.

**Why it happens:** The engine override mechanism only suppresses `ProjectAnalyzer` implementations
(the `project_analyzers.retain(|a| !json_pipeline_names.contains(a.name()))` call in `engine.rs`
lines 245–246). Regular `Pipeline`/`NodePipeline`/`GraphPipeline` implementations registered via
`pipelines_for_language()` in `pipeline.rs` are **not suppressed** by the JSON system. The
`json_pipeline_names` set is only used to filter `ProjectAnalyzer` entries. The per-file pipeline
map is built unconditionally from `pipelines_for_language()` before the JSON override set is even
consulted.

**Consequence:** Every audit run produces doubled findings for the pipeline. Finding counts double
in summary output. `--pipeline` filter will activate both. CI noise spikes with no error thrown.

**Warning signs:**
- `cargo test` still passes (both implementations are internally consistent)
- Finding counts in integration tests suddenly double for a specific pipeline
- `--pipeline panic_detection` returns exactly 2x the expected count

**Prevention:**
- Delete the Rust `.rs` file in the **same commit** that adds the JSON file
- Add an integration test asserting the specific pipeline appears exactly once in engine output
- Phase rule: JSON file + Rust deletion must be atomic; never merge a PR that adds a JSON file
  without deleting the corresponding Rust pipeline file

**Which phase:** Every pipeline migration phase. Non-negotiable atomic operation.

---

### Pitfall 2: JSON Pipeline File Name Does Not Match the `pipeline` Field

**What goes wrong:** File is named `src/audit/builtin/panic_detection.json` but the JSON content
has `"pipeline": "panic_detections"` (note the trailing `s`). The file loads without error, registers
under the wrong name, and the intended Rust pipeline override never fires.

**Why it happens:** The engine name-match override compares `json_pipeline_names` (built from the
`pipeline` field in JSON content) against Rust pipeline names. The JSON filename is irrelevant — only
the `pipeline` field value matters. There is no validation that the filename matches the pipeline field.
`json_audit.rs`'s `discover_json_audits` loads all `.json` files from the directory and parses the
content; it never checks filename-vs-field consistency.

**Consequence:** The old Rust pipeline keeps running (because the name-match override never fires),
the new JSON pipeline runs under a phantom name that no Rust code knows about (producing findings
tagged with an unknown pipeline name), and the CLI output becomes inconsistent — two differently-named
pipelines report what appears to be the same kind of finding.

**Warning signs:**
- `cargo test` passes (both pipeline names are technically valid)
- Audit output shows a new, never-before-seen pipeline name alongside the expected one
- `--pipeline panic_detection` returns Rust pipeline findings; `--pipeline panic_detections` returns JSON findings

**Prevention:**
- Enforce a convention: the JSON filename stem must equal the `pipeline` field value
- Add a test in `json_audit.rs`: for each built-in JSON file embedded via `include_str!`, assert
  that the `pipeline` field matches the filename stem (derive the expected name from the `include_str!`
  path using a compile-time assertion or a startup validation test)
- New built-in JSON files are added via `include_str!` in `builtin_audits()` — the list in that
  function is the ground truth; each entry can be paired with an expected pipeline name in the test

**Which phase:** Phase 1 (first JSON pipeline batch). Establish the test pattern before the bulk migration.

---

### Pitfall 3: Silent Empty Output From Unimplemented Executor Stages

**What goes wrong:** A JSON pipeline uses a `traverse`, `filter`, `match_name`, `count_edges`, or `pair`
stage. The executor (`executor.rs` lines 165–185) treats all five of these as stubs that **silently pass
nodes through unchanged**. A pipeline authored to use `{"traverse": {"edge": "calls"}}` appears to work
(no error, no panic), but the traversal does nothing — nodes from the previous stage pass through as-is,
producing findings for every node selected rather than only traversal-filtered nodes. Or, if traverse is
meant to expand the node set, it fails to do so and the pipeline produces zero findings.

**Why it happens:** The executor has explicit `TODO: implement` comments on five `GraphStage` variants.
These stubs exist to allow the enum to compile without restricting future stage types, but they are
invisible failure modes: no warning is emitted, no error is returned, and pipeline execution completes
with `Ok(...)`.

**Consequence:** Depends on how the stub stage is used:
- `traverse` used to filter: produces too many findings (all pre-traverse nodes flagged)
- `traverse` used to expand: produces zero findings (no nodes added)
- `filter` used to gate: produces too many findings (filter is a no-op)
- `count_edges` used for threshold: count metric is never set, `metric_f64("edge_count")` returns 0.0,
  causing severity maps that test `edge_count` to always match the zero-edge branch

**Warning signs:**
- A pipeline using a stub stage produces findings for every file in the workspace
- A pipeline using a stub stage produces zero findings even on code that clearly matches the pattern
- `edge_count` in a finding message renders as `0` regardless of actual import count

**Prevention:**
- Audit the `audit_plans/` spec for every pipeline being migrated: if the spec describes traversal
  or edge-counting logic, verify that the corresponding executor stage is fully implemented before
  writing the JSON
- Add a test in `executor.rs` (or a dedicated test file) that asserts each stub stage produces a
  compile-time or runtime warning when executed — or, better, make the stub arms return `Err(anyhow!("not implemented: traverse"))` so failures are loud, not silent
- Do not write any JSON pipeline that relies on `traverse`, `filter`, `match_name`, `count_edges`,
  or `pair` until those stages are implemented

**Which phase:** Phase 1. The stub inventory must be documented before any pipeline uses them.

---

### Pitfall 4: JSON Pipeline Requires `graph` Argument That Is None at Runtime

**What goes wrong:** JSON pipelines only execute when `graph` is `Some` (engine.rs line 257:
`if let Some(g) = graph`). Many audit invocations pass `graph: None` — for example, when the
tech-debt or complexity audit is run without `--graph` (common in the test suite and in the
`engine_basic` test). A JSON pipeline intended for the `code-quality` category will silently produce
zero findings in these cases without any error or warning.

**Why it happens:** The `AuditEngine::run()` method routes JSON pipelines through the graph-dependent
block unconditionally. `PipelineSelector::TechDebt`, `PipelineSelector::Complexity`, and
`PipelineSelector::CodeStyle` call `run()` from `main.rs` without constructing a `CodeGraph` unless
the user explicitly builds one. The existing Rust pipelines for these categories use `AnyPipeline::Node`
or `AnyPipeline::Legacy`, which run in the per-file parallel loop that does **not** require a graph.
New JSON pipelines do not participate in that loop at all.

**Consequence:** Any tech-debt, complexity, or code-style pipeline migrated to JSON will silently
produce zero findings when invoked via the standard CLI path for those audit categories. Users see
an empty audit result and assume their code is clean.

**Warning signs:**
- `cargo audit code-quality tech-debt ./src` returns zero findings for a pipeline known to detect patterns in the test codebase
- Integration test for a migrated pipeline passes only when `graph: Some(...)` is supplied

**Prevention:**
- Only migrate pipelines that were already `ProjectAnalyzer` implementations (i.e., graph-dependent
  by design) to JSON in the short term
- For tree-sitter-based pipelines (those using `AnyPipeline::Node` or `AnyPipeline::Legacy`), the
  JSON engine is not a drop-in replacement — the executor only operates on graph nodes, not on raw AST
- Document clearly in `PROJECT.md`: "JSON pipelines are graph-only. Tree-sitter-based pipelines cannot
  be expressed in JSON with the current executor."
- Add a test that runs JSON pipelines with `graph: None` and asserts the result is zero findings with a
  stderr warning, not a silent empty result

**Which phase:** Phase 0 (pre-migration design). This is an architectural constraint, not just a
coding mistake.

---

### Pitfall 5: Deleting 2205 Rust Tests Without Adequate Replacement Creates a Coverage Black Hole

**What goes wrong:** The existing 2205 `#[test]` functions are unit tests co-located with Rust
pipeline files. When a Rust file is deleted, its tests disappear atomically. If the corresponding
JSON pipeline has no integration tests, the behavior that was previously tested is now completely
uncovered. `cargo test` passes with fewer tests but gives no signal that coverage has collapsed.

**Why it happens:** There is no coverage enforcement (`TESTING.md` line 172: "None enforced —
no coverage tool configured"). Deleting a test file is not a test failure. The `builtin_audits`
test in `json_audit.rs` only checks that JSON parses without error and that the graph is non-empty
— it does not assert that specific code patterns produce findings.

**Consequence:** A regression in a JSON pipeline (threshold off-by-one, wrong pattern name, wrong
severity) goes undetected. Users file bug reports weeks after the migration.

**Specific coverage gaps to watch:**
- Threshold boundary values (exactly at threshold vs. one below)
- Negative cases (clean code that must not be flagged)
- False positive exclusions (test files, generated files, barrel files)
- Language-specific idioms documented in `audit_plans/` as previously buggy in the Rust version
- Severity graduation logic (severity maps in the JSON flag stage)

**Warning signs:**
- Test count drops by more than 10 per pipeline deleted without a corresponding addition elsewhere
- A migrated pipeline's JSON file contains a `severity_map` but no test exercises each severity level
- No fixture file exists for the language being migrated

**Prevention:**
- Before deleting any Rust file, count its tests and write one integration test per pattern the
  pipeline detects (minimum: one positive case asserting a finding, one negative case asserting none)
- Integration tests go in `tests/audit_json_integration.rs` using `MemoryFileSource` with inline
  source snippets — no disk I/O needed
- Specifically add tests for every case in the `### New Test Cases` section of the `audit_plans/`
  document for that pipeline — these are the bugs the migration is supposed to fix
- Track "tests deleted vs. tests added" as a phase-level metric; a phase is not done until the
  addition count equals or exceeds the deletion count

**Which phase:** Every pipeline migration phase. The replacement tests must be in the same PR as
the deletion.

---

### Pitfall 6: Behavior Regression — JSON Pipeline Detects Less Than the Rust Version

**What goes wrong:** The JSON pipeline for `api_surface_area` uses the `ratio` stage with
`{"numerator": {"where": {"exported": true}}, "denominator": {}}`. The Rust pipeline used
`count_top_level_definitions` which included `export_statement` wrapper nodes in the denominator.
The JSON pipeline uses graph Symbol nodes which do NOT include `export_statement` wrappers. The
denominator is smaller in JSON than in Rust, making the ratio higher — files that were borderline
in Rust now cross the threshold, producing MORE findings, not fewer. Or, for a different pipeline,
the wrong node type produces fewer matches.

**Why it happens:** The JSON executor's `execute_select` with `NodeType::Symbol` iterates
`NodeWeight::Symbol` entries in the graph. The graph's symbol set depends on what the `GraphBuilder`
extracted during construction — not all AST nodes become graph symbols. Patterns that the Rust
pipeline detected via direct tree-sitter queries may not have corresponding graph nodes.

**Specific known gaps (from reading the audit plans):**
- `pub(crate)` vs. `pub` — the graph's `Symbol.exported` field is a bool; it cannot distinguish
  `pub` from `pub(crate)`. The Rust pipeline had the same bug, but the JSON pipeline inherits it
  and cannot fix it without a graph schema change.
- Struct fields — the graph does not store struct fields as Symbol nodes. `leaky_abstraction_boundary`
  in the Rust pipeline used tree-sitter to find `pub` struct fields. The JSON engine has no
  equivalent capability (no tree-sitter access in the executor). This pattern **cannot** be expressed
  in the current JSON format.
- Line count — `NodeWeight::File` does not store a line count. Pipelines like `oversized_module`
  that threshold on `>= 1000 lines` cannot be replicated in JSON using the graph alone.
- TypeScript parameter properties (`constructor(public name: string)`) — not stored as Symbol nodes.

**Warning signs:**
- The test for the JSON pipeline produces zero findings for a code snippet that the Rust version
  flagged (false negative regression)
- The JSON pipeline produces findings for code the Rust version did not flag (false positive regression)
- A pattern from the `audit_plans/` spec says "requires tree-sitter for X" — that pattern cannot be
  expressed in JSON

**Prevention:**
- Before writing a JSON pipeline, cross-reference its required detection logic against what the
  executor actually supports: `select` (file/symbol/callsite), `group_by`, `count`, `ratio`,
  `find_cycles`, `max_depth`, `flag`
- If the audit plan says "tree-sitter is needed for Y", that pipeline cannot be fully expressed
  in JSON; either defer it, simplify the detection to what the graph supports, or note the regression
  explicitly in the PR
- Write a regression test for every pattern the Rust version detected that uses a code fixture
  matching the pattern

**Which phase:** Every pipeline migration phase. The comparison test must be written first (TDD style).

---

## Moderate Pitfalls

---

### Pitfall 7: Built-in JSON Files Use `include_str!` — Adding a New File Requires Modifying Rust Code

**What goes wrong:** A developer writes a new JSON pipeline file in `src/audit/builtin/` but forgets
to add it to the `sources` array in `builtin_audits()` in `json_audit.rs`. The file exists on disk but
is never loaded.

**Why it happens:** `builtin_audits()` uses an explicit array of `include_str!()` calls. Unlike the
user-local and project-local paths (which use `std::fs::read_dir` for automatic discovery), built-in
audits require a manual `include_str!` entry. There is no compile-time check that every `.json` file
in `src/audit/builtin/` has a corresponding `include_str!`.

**Consequence:** The pipeline is never run. Findings that should appear are silently absent. The
`test_builtin_audits_returns_four` test hardcodes the count (currently 4), so adding a fifth file
without updating the test makes the test fail — but failing tests are at least detectable.

**Warning signs:**
- Running `virgil audit architecture ./src` returns zero findings for the new pipeline
- The hardcoded count assertion in `test_builtin_audits_returns_four` fails (only if the test is updated)

**Prevention:**
- Update `builtin_audits()` and the count assertion in the same commit that adds the JSON file
- Consider replacing the hardcoded `include_str!` array with the `include_dir!` macro (already
  imported — see the `include_dir!` reference in `PROJECT.md` context) so new files are discovered
  automatically at compile time without manual list maintenance

**Which phase:** Phase 1 (first new built-in JSON pipeline batch).

---

### Pitfall 8: Language Filter Mismatch — JSON Pipeline Fires on Wrong Languages

**What goes wrong:** A JSON pipeline has `"languages": ["rust"]` (lowercase) but the engine compares
with `l.as_str().eq_ignore_ascii_case(lang_str)` — this comparison is case-insensitive, so this is
fine. However, if the `languages` field is omitted entirely, the pipeline runs on every language the
engine is invoked with. A pipeline designed for Rust-specific patterns (e.g., `pub(crate)` detection)
will run on TypeScript files and produce zero findings — but the pipeline still executes and consumes
time.

**A more serious variant:** A pipeline designed for TypeScript with `"languages": ["typescript"]` will
NOT run on `.tsx` files because the engine stores TSX files as `Language::Tsx` whose `as_str()` is
`"tsx"`, not `"typescript"`. The language strings are distinct in the enum. A TS-only pipeline must
list both `["typescript", "tsx"]` or the TSX audit is silently skipped.

**Warning signs:**
- A TS pipeline produces findings on `.ts` files but zero on `.tsx` files in a mixed codebase
- A pipeline produces unexpected findings on wrong-language files (missing `languages` field)

**Prevention:**
- Always specify the `languages` field in every JSON pipeline; never omit it
- For TypeScript pipelines, always list `["typescript", "tsx"]`
- Add a test for each JSON pipeline asserting it produces findings on the correct language fixture
  and zero findings on a wrong-language fixture

**Which phase:** Phase 1. Establish the convention before the bulk migration.

---

### Pitfall 9: Category Field Mismatch Causes Findings to Appear in Wrong Audit Category

**What goes wrong:** A JSON pipeline for cyclomatic complexity has `"category": "architecture"` instead
of `"category": "code-quality"`. The engine does not filter JSON pipelines by category — it runs ALL
discovered JSON pipelines regardless of which `PipelineSelector` is active. The category field is
stored but never compared against `self.pipeline_selector` in the engine's JSON execution block
(engine.rs lines 257–294).

**Why it happens:** The engine code at line 258 iterates `json_audits` without checking
`json_audit.category` against `self.pipeline_selector`. The category field exists for documentation
and CLI output labeling only; it does not gate execution.

**Consequence:** Running `virgil audit code-quality ./src` also runs architecture JSON pipelines.
Running `virgil audit architecture ./src` also runs code-quality JSON pipelines. Finding counts
are inflated and the wrong categories appear in the grouped output. The `--pipeline` filter still
works correctly, but the category-level routing is meaningless for JSON pipelines.

**Warning signs:**
- `virgil audit code-quality ./src` returns findings labeled with `pipeline: circular_dependencies`
  (an architecture pipeline)
- Summary output shows pipelines from unexpected categories

**Prevention:**
- This is an engine bug, not a per-pipeline authoring mistake — but until it is fixed, ensure the
  `category` field in every JSON pipeline matches the category under which the corresponding Rust
  pipeline was registered
- Note this as a known engine limitation in migration docs; do not treat it as a migration bug
  per se

**Which phase:** Phase 0. Document the limitation before migration begins. Fix in a dedicated
engine patch if needed.

---

### Pitfall 10: `ProjectAnalyzer` Override vs. Per-File Pipeline Override Is Asymmetric

**What goes wrong:** The `cross_file_coupling` Rust analyzer is a `ProjectAnalyzer`. When a JSON
file named `cross_file_coupling.json` is added, the engine correctly suppresses the `ProjectAnalyzer`
via `project_analyzers.retain(...)`. But many pipelines are `AnyPipeline::Legacy` or
`AnyPipeline::Node`, registered through `pipelines_for_language()`. Adding a JSON file with the
same name does NOT suppress these — both run.

**The asymmetry:** `ProjectAnalyzer` suppression is explicit (the retain call). Per-file pipeline
suppression does not exist. This means the JSON override mechanism only fully works for the three
cross-file `ProjectAnalyzer` implementations (`circular_deps`, `dependency_depth`, `coupling`)
plus any future ones. All ~300 legacy per-file pipelines must be deleted manually — the JSON engine
provides no protection against running both.

**Warning signs:**
- Adding a JSON file with the same name as a per-file Rust pipeline doubles the findings
- The engine tests for override (`engine_json_audit_overrides_rust_project_analyzer`) pass, giving
  false confidence that all overrides work

**Prevention:**
- Never rely on the JSON override mechanism for per-file pipelines; always delete the Rust file
  explicitly
- In the migration PR template, explicitly require: "Rust file deleted: YES/NO"

**Which phase:** Every phase. The asymmetry must be understood before the first migration commit.

---

## Minor Pitfalls

---

### Pitfall 11: `ratio` Stage Uses `{{ratio}}` as a Float With `{:.2}` Formatting

**What goes wrong:** The `interpolate_message` function in `pipeline.rs` formats `MetricValue::Float`
as `format!("{:.2}", f)` — always two decimal places. A ratio of 1.0 renders as `"1.00"` and a
ratio of 0.8 renders as `"0.80"`. The `api_surface_area.json` currently uses `{{ratio}}` in its
message but calls it a "percentage" (`"exports {{count}} symbols ({{ratio}}% of total)"`). The
value is actually a fraction between 0 and 1, so `0.85` renders as `"0.85%"` not `"85%"`. The
Rust pipeline multiplied by 100 before reporting.

**Warning signs:**
- Findings messages read `"exports 12 symbols (0.91% of total)"` instead of `"91% of total"`

**Prevention:**
- When writing message templates for ratio pipelines, either accept the `0.00–1.00` fraction format
  or multiply the ratio before passing it (which requires a pre-flag arithmetic stage not currently
  supported), or just document that `{{ratio}}` is a fraction
- The current `api_surface_area.json` already has this inconsistency; it is a pre-existing issue
  but new pipelines should not repeat it

**Which phase:** Phase 1. Establish consistent message template conventions.

---

### Pitfall 12: `count` Stage Picks First Member as Representative — Line Number May Be Misleading

**What goes wrong:** The `count` stage in `executor.rs` (line 386) emits one `PipelineNode` per
surviving group, using `members[0]` as the representative. The first member's `line` number becomes
the finding's line number. For a `group_by: "file"` + `count` pipeline, the representative is the
first symbol alphabetically/by node index — typically line 1 is not the correct location to point
users at for a module-size finding.

**Warning signs:**
- Findings always report line 1 for file-level patterns regardless of where the issue actually is
  (this is often acceptable for file-level findings, but can be confusing for symbol-group findings)

**Prevention:**
- For file-level findings, this is acceptable — line 1 is conventional for file-scope issues
- For symbol-group findings, consider sorting members before selecting the representative, or
  document the limitation

**Which phase:** Not blocking; document during Phase 1.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| First architecture pipeline batch | Rust file not deleted (Pitfall 1) | Atomic PR: JSON add + Rust delete |
| First architecture pipeline batch | Filename vs. `pipeline` field mismatch (Pitfall 2) | Add filename-match test in `json_audit.rs` |
| Any pipeline using `traverse` or `filter` | Silent stub pass-through (Pitfall 3) | Audit `executor.rs` stubs before authoring |
| Tech-debt / complexity / code-style migration | Pipeline silently produces zero findings (Pitfall 4) | JSON-only valid for ProjectAnalyzer-class pipelines |
| Deleting Rust pipeline files | Test coverage black hole (Pitfall 5) | Write replacement tests in same PR |
| Any pipeline with tree-sitter-required detection | Behavior regression (Pitfall 6) | Cross-reference executor capabilities against audit plan |
| Adding new built-in JSON files | File not loaded (Pitfall 7) | Update `include_str!` list atomically |
| TypeScript pipelines | TSX files skipped (Pitfall 8) | Always list `["typescript", "tsx"]` |
| Any JSON pipeline | Category routing (Pitfall 9) | Engine does not filter by category; treat as known limitation |
| Per-file Legacy/Node pipelines | Override asymmetry (Pitfall 10) | Always delete Rust file; never rely on JSON override mechanism |

---

## Engine Limitations That Block Certain Migrations

These are hard limits in the current JSON executor. Pipelines requiring these capabilities **cannot** be
fully migrated to JSON without engine changes first.

| Capability | Required By | Current Status | Blocker Level |
|------------|-------------|----------------|---------------|
| Tree-sitter AST access from JSON stage | Struct field visibility, parameter properties, line counts | Not supported | Hard block |
| File line count in graph | `oversized_module` line threshold | Not stored in `NodeWeight::File` | Hard block |
| `pub` vs `pub(crate)` distinction | Rust `api_surface_area`, `module_size_distribution` | Not in `Symbol.exported` bool | Hard block |
| `traverse` stage (BFS expansion) | Any pipeline needing call-graph or import-chain expansion | Stub, no-op | Soft block (implement first) |
| `count_edges` stage | Coupling thresholds | Stub, no-op | Soft block |
| `filter` stage | Removing nodes by edge properties | Stub, no-op | Soft block |
| Per-file pipeline execution | Tech-debt, complexity, code-style pipelines | JSON executor is graph-only | Architectural constraint |

---

*Sources: direct inspection of `src/audit/engine.rs`, `src/audit/json_audit.rs`, `src/graph/executor.rs`,
`src/graph/pipeline.rs`, `src/audit/pipeline.rs`, `src/audit/builtin/*.json`, `audit_plans/*.md`,
`.planning/PROJECT.md`, `.planning/codebase/CONCERNS.md`, `.planning/codebase/TESTING.md`*
