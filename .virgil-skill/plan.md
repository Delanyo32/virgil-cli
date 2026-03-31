# Plan: Reduce Benchmark Extras via Graph-Based Analysis

## Config
Auto-commit: yes
Auto-proceed phases: no

## Desired State
Virgil-cli's audit pipelines leverage the CodeGraph for context-aware analysis, dramatically reducing false positives (extras) in the Python technical debt benchmark. Currently 1681 extras; target ≤200 by moving noisy tree-sitter-only detectors to graph-aware implementations.

### Criteria
- [ ] All 6 audit subcommands pass the graph to `engine.run()` (currently only 2 do)
- [ ] `missing_type_hints` pipeline uses graph to scope to cross-module API functions (444 → ≤50 extras)
- [ ] `memory_leak_indicators` pipeline uses graph/CFG to filter bounded collections (≤15 extras from unbounded_growth)
- [ ] `n_plus_one_queries` pipeline uses graph to resolve receiver types (≤10 extras from query_in_loop)
- [ ] `dead_code` unused_import uses graph import/export edges (≤20 extras)
- [ ] `coupling/low_cohesion` uses graph to check polymorphic usage (≤10 extras)
- [ ] `injection` pipeline suppresses tree-sitter findings when graph shows safety (≤10 extras)
- [ ] `path_traversal` pipeline adds graph-based taint check (≤5 extras)
- [ ] `api_surface_area` uses graph cross-module import data (≤10 extras)
- [ ] `module_size_distribution` uses graph export analysis (≤5 extras)
- [ ] Benchmark GAP_REPORT regenerated showing ≤200 total extras
- [ ] No regressions in matched count (≥370)

## Root Cause Analysis

**Graph availability by subcommand (in `main.rs`):**

| Subcommand | Graph? | Function | Line |
|---|---|---|---|
| `code-quality/tech-debt` | `None` | `run_tech_debt_ws` | 437 |
| `code-quality/complexity` | `None` | `run_complexity_ws` | 476 |
| `code-quality/code-style` | `Some` | `run_code_style_ws` | 517 |
| `security` | `None` | `run_security_ws` | 556 |
| `scalability` | `None` | `run_scalability_ws` | 595 |
| `architecture` | `Some` | `run_architecture_ws` | 636 |

4/6 subcommands don't build or pass the graph. Even if pipelines had graph logic, they'd receive `None`.

**Extras breakdown by pipeline (tree-sitter only, no graph):**

| Pipeline | Extras | Problem |
|---|---|---|
| `missing_type_hints` | 444 | Flags every public untyped function |
| `dead_code/unused_import` | ~80 | Text occurrence counting misses re-exports |
| `coupling/low_cohesion` | ~30 | Flags every method not using `self` |
| `coupling/parameter_overload` | ~15 | Correct but noisy |
| `duplicate_code/duplicate_elif_branch` | ~10 | Correct |
| `memory_leak_indicators/unbounded_growth` | ~60 | Flags every `.append()` in a loop |
| `n_plus_one_queries/query_in_loop` | ~25 | Flags `.get()` on dicts |
| `memory_leak_indicators/file_handle_leak` | ~8 | Misses try/finally |
| `sql_injection/sql_fstring` | ~30 | Flags every f-string in execute() |
| `path_traversal/*` | ~15 | Flags every path param |
| `api_surface_area/*` | ~25 | Flags Python modules with no `_` prefix |
| `module_size_distribution/*` | ~8 | Correct structural observations |
| `function_length/*` | ~50 | Correct findings (add to manifest) |
| `cyclomatic/cognitive_complexity` | ~36 | Correct findings (add to manifest) |

**Benchmark location:** `../virgil-skills/benchmarks/python/technical-debt/`
**GAP_REPORT:** `../virgil-skills/benchmarks/python/technical-debt/GAP_REPORT.md`
**Test harness:** `../virgil-skills/benchmarks/tests/`

## Phase 1: Wire Graph to All Audit Subcommands
Status: completed
Goal: All 6 audit subcommands build and pass the CodeGraph to `engine.run()`

### Task 1.1: Build and pass graph in tech-debt, complexity, security, scalability subcommands
Status: completed
Change: In `src/main.rs`, modify 4 functions to build graph and pass `Some(&index)`:
- `run_tech_debt_ws()` (~line 425): Add `GraphBuilder::new(&workspace, &languages).build()?`, pass `Some(&index)` at line 437
- `run_complexity_ws()` (~line 462): Same pattern, pass `Some(&index)` at line 476
- `run_security_ws()` (~line 542): Same pattern, pass `Some(&index)` at line 556
- `run_scalability_ws()` (~line 581): Same pattern, pass `Some(&index)` at line 595
- Also fix the "all" combined mode functions that pass `None` for tech-debt, complexity, security, scalability

Test: `virgil-cli audit ../virgil-skills/benchmarks/python/technical-debt/app security --format json --language py` should still produce the same findings (graph is optional, pipelines default to tree-sitter when they don't override `check_with_context`)
Research:
- [code] `GraphBuilder::new(&workspace, &languages).build()?` — existing pattern used in code-style and architecture (source: main.rs:503, main.rs:622)
- [code] `engine.run(workspace, Some(&index))` — existing pattern (source: main.rs:517, main.rs:636)
- [code] Pipeline trait defaults: `check_with_context` delegates to `check_with_ids` which delegates to `check` — so adding graph won't change behavior until pipelines opt in (source: pipeline.rs)
Findings: All 6 locations updated. 4 individual subcommands now build their own graph. Combined modes (run_code_quality_summary_ws, run_full_audit_ws) build a single shared graph for all categories. Build clean, 1931 tests pass.

## Phase 2: Reduce Scalability Extras (96 → ~20)
Status: completed
Goal: `memory_leak_indicators` and `n_plus_one_queries` use graph context to eliminate false positives

### Task 2.1: Graph-aware unbounded_growth in memory_leak_indicators
Status: completed
Change: In `src/audit/pipelines/python/memory_leak_indicators.rs`:
- Override `check_with_context()` instead of relying on `check()`
- When graph is available, for each `.append()`/`.extend()` in a loop:
  1. Find the target collection variable name
  2. Check if it's a local variable (defined in current function scope)
  3. If local AND function returns it (or passes to return value), it's a result builder → skip
  4. Use function CFG from `graph.function_cfgs` to check if the loop is bounded (e.g., iterates over function parameter, not unbounded generator)
- When graph is `None`, fall back to current tree-sitter behavior
- Also: for `file_handle_leak`, check for try/finally blocks, not just `with` statements

Test: Run `virgil-cli audit ../virgil-skills/benchmarks/python/technical-debt/app scalability --format json --language py | jq '[.[] | select(.pattern == "unbounded_growth")] | length'` — count should drop from ~60 to ~15
Research:
- [code] Current detection: `.append()` inside `for_statement`/`while_statement` → finding (source: memory_leak_indicators.rs)
- [code] `PipelineContext.graph.function_cfgs` — HashMap<NodeIndex, FunctionCfg> for control flow analysis
- [code] Current `file_handle_leak`: only checks `with_statement` parent (source: memory_leak_indicators.rs)
Findings: Overrode check_with_context() with result-builder detection (init as [] + returned), bounded loop iteration (for-loop over parameter), and try/finally for file_handle_leak. 6 new tests, all pass.

### Task 2.2: Graph-aware query_in_loop in n_plus_one_queries
Status: completed
Change: In `src/audit/pipelines/python/n_plus_one_queries.rs`:
- Override `check_with_context()`
- When graph is available, for each flagged call in a loop:
  1. Resolve the receiver variable to its definition (use graph symbol lookup)
  2. If receiver is assigned from a dict literal, `{}`, or `dict()` call → skip (not a DB object)
  3. If receiver comes from a function returning a collection type (list, dict, set) → skip
  4. Only flag when receiver traces back to a DB/ORM/cursor source
- Tighten the `.get()` heuristic: only flag `.get()` when receiver is in DB-related namespace
- When graph is `None`, fall back to current behavior

Test: Run audit and count `query_in_loop` findings — should drop from ~25 to ~10
Research:
- [code] Current receiver validation: inclusive list (session, query, db, objects) + exclusive list (list, dict, set, cache) (source: n_plus_one_queries.rs)
- [code] `graph.find_symbol(file_path, line)` → Option<NodeIndex> for symbol resolution
- [code] `graph.traverse_callers()` for tracing data sources
Findings: Overrode check_with_context() with receiver assignment tracing (suppresses dict/list/set literals, comprehensions, dict()/list()/set() calls) and .get() with default arg heuristic. 7 new tests, all pass.

## Phase 3: Reduce Type-Safety Extras (444 → ~50)
Status: pending
Goal: `missing_type_hints` only flags functions that are part of cross-module public API

### Task 3.1: Graph-aware missing_type_hints — scope to cross-module API
Status: pending
Change: In `src/audit/pipelines/python/missing_type_hints.rs`:
- Override `check_with_context()`
- When graph is available, for each untyped public function:
  1. Look up function in graph: `graph.find_symbol(file_path, start_line)`
  2. Check if it has callers from OTHER files: `graph.traverse_callers(&[node], 1)` and filter for callers in different files
  3. If function is only called within same file or not called at all → skip (internal implementation detail)
  4. If function is exported AND called cross-module → flag (genuine public API without types)
- This preserves flagging for genuinely public interfaces while eliminating noise from internal functions
- When graph is `None`, fall back to current behavior

Test: Count `missing_type_hints` findings — should drop from ~444 to ~50
Research:
- [code] `NodeWeight::Symbol { exported: bool, ... }` — tracks export status (source: graph/mod.rs)
- [code] `graph.traverse_callers(&[node_idx], 1)` → Vec<NodeIndex> — finds direct callers
- [code] `graph.find_symbol(file_path, start_line)` → Option<NodeIndex>
Findings:

## Phase 4: Reduce Style Extras (143 → ~30)
Status: pending
Goal: `dead_code` and `coupling` use graph for import tracing and cohesion analysis

### Task 4.1: Graph-aware unused_import detection
Status: pending
Change: In `src/audit/pipelines/python/dead_code.rs`:
- Override `check_with_context()`
- When graph is available, for each flagged unused import:
  1. Check if imported symbol has `Exports` edge from current file (re-export pattern)
  2. Check if imported symbol appears in `Calls` edges from any symbol in current file
  3. Check if the import is used in type annotations (look for the name in function signatures via tree-sitter)
  4. If any of these → skip (symbol is used, just not detected by text counting)
- When graph is `None`, fall back to current behavior

Test: Count `unused_import` findings — should drop from ~80 to ~20
Research:
- [code] Current detection: counts identifier occurrences, if count == 0 in non-import nodes → unused (source: dead_code.rs)
- [code] `EdgeWeight::Exports` — tracks file export relationships
- [code] `EdgeWeight::Calls` — tracks call relationships between symbols
Findings:

### Task 4.2: Graph-aware low_cohesion in coupling
Status: pending
Change: In `src/audit/pipelines/python/coupling.rs`:
- Override `check_with_context()` for the low_cohesion check
- When graph is available, for each method flagged as low-cohesion (has `self` but doesn't use it):
  1. Check if the method overrides a parent class method (part of interface/protocol)
  2. Check if the method is called polymorphically from outside the class
  3. If the class implements an ABC or Protocol → skip all methods (interface contract)
- When graph is `None`, fall back to current behavior

Test: Count `low_cohesion` findings — should drop from ~30 to ~10
Research:
- [code] Current detection: checks if method body contains `self.` attribute access (source: coupling.rs)
- [code] `NodeWeight::Symbol { kind: SymbolKind::Method, ... }` for method symbols
- [code] Graph can trace inheritance via Contains edges and class hierarchies
Findings:

## Phase 5: Reduce Security Extras (45 → ~10)
Status: pending
Goal: Tree-sitter security findings suppressed when graph taint analysis shows safety

### Task 5.1: Suppress safe sql_fstring findings via graph
Status: pending
Change: In `src/audit/pipelines/python/injection.rs`:
- Modify `check_with_context()` (already exists):
  - Currently: runs tree-sitter check AND graph check, returning both
  - Change: when graph is available, run tree-sitter check BUT filter out findings where the f-string variables are NOT taint sources (i.e., they're constants, config values, or sanitized)
  - For each tree-sitter `sql_fstring` finding, extract the interpolated variable names
  - Check graph for `FlowsTo` edges from `ExternalSource` nodes to those variables
  - If no taint path exists → suppress the finding
- When graph is `None`, keep all tree-sitter findings (conservative)

Test: Count `sql_fstring` findings — should drop from ~30 to ~10
Research:
- [code] `check_with_context()` already implemented in injection.rs — needs modification, not creation
- [code] `NodeWeight::ExternalSource { kind: SourceKind::UserInput, ... }` — marks untrusted inputs
- [code] `EdgeWeight::FlowsTo` — taint propagation edges
- [code] `EdgeWeight::SanitizedBy { sanitizer }` — sanitization markers
Findings:

### Task 5.2: Add graph-based taint analysis to path_traversal
Status: pending
Change: In `src/audit/pipelines/python/path_traversal.rs`:
- Override `check_with_context()`
- When graph is available, for each flagged path operation:
  1. Resolve the parameter to its function's graph node
  2. Check if any caller passes user-controlled input (trace `FlowsTo` edges)
  3. Check for `SanitizedBy` edges (path validation/normalization)
  4. If parameter never receives external input OR is sanitized → suppress
- When graph is `None`, fall back to current tree-sitter behavior

Test: Count `path_traversal` findings — should drop from ~15 to ~5
Research:
- [code] Current detection: flags open()/path.join() when argument is a function parameter (source: path_traversal.rs)
- [code] Pattern matches injection.rs graph integration structure
Findings:

## Phase 6: Reduce Architecture Extras (33 → ~10)
Status: pending
Goal: Architecture metrics consider actual cross-module usage via graph

### Task 6.1: Graph-aware api_surface_area
Status: pending
Change: In `src/audit/pipelines/python/api_surface_area.rs`:
- Override `check_with_context()`
- For `excessive_public_api`:
  1. Use graph to count how many exported symbols are actually imported by OTHER modules
  2. If effective API (actually-used exports) is ≤80% threshold → suppress
  3. A module exporting 30 symbols but only 8 used cross-module is fine
- For `leaky_abstraction_boundary`:
  1. Use graph to check if public attributes are accessed from outside the class
  2. If attributes are only accessed internally → suppress
- When graph is `None`, fall back to current behavior

Test: Count `excessive_public_api` + `leaky_abstraction_boundary` findings — should drop from ~25 to ~10
Research:
- [code] Current: counts symbols starting without `_`, ratio > 80% → finding (source: api_surface_area.rs)
- [code] `graph.symbol_nodes` — HashMap<(String, u32), NodeIndex> for all symbols
- [code] `EdgeWeight::Imports` — file-level import edges for cross-module analysis
Findings:

### Task 6.2: Graph-aware module_size_distribution
Status: pending
Change: In `src/audit/pipelines/python/module_size_distribution.rs`:
- Override `check_with_context()`
- For `monolithic_export_surface`:
  1. Use graph to count actually-imported exports (same as 6.1)
  2. If effective export count is under threshold → suppress
- For `oversized_module`:
  1. Check if module is a "barrel" (mostly re-exports from submodules)
  2. If >80% of definitions are re-exports → suppress (it's an intentional aggregation point)
- When graph is `None`, fall back to current behavior

Test: Count `oversized_module` + `monolithic_export_surface` findings — should drop from ~8 to ~3
Research:
- [code] Current: ≥20 exported symbols → monolithic, ≥30 definitions or ≥1000 lines → oversized (source: module_size_distribution.rs)
- [code] Graph can distinguish local definitions from re-exported imports
Findings:

## Phase 7: Add Correct Extras to Manifest + Regenerate Report
Status: pending
Goal: code-quality extras that are genuine debt get added to manifest, final GAP_REPORT regenerated

Note: This phase runs in the `virgil-skills` repo, not here.

### Task 7.1: Add code-quality extras to DEBT_MANIFEST.toml
Status: pending
Change: Add ~86 entries for genuinely correct findings (function_length, cyclomatic_complexity, cognitive_complexity, god_functions) to `../virgil-skills/benchmarks/python/technical-debt/DEBT_MANIFEST.toml`
Test: TOML parses correctly, entries have valid file:line references

### Task 7.2: Regenerate GAP_REPORT and verify improvement
Status: pending
Change: Run full benchmark test suite from `../virgil-skills/benchmarks/tests/`, regenerate GAP_REPORT.md
Test: Extras ≤200, matched ≥450 (370 original + ~86 new manifest entries), no detection regressions
