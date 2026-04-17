# virgil-cli Simplification Design

**Date:** 2026-04-16  
**Status:** Approved

## Problem

virgil-cli has two parallel audit systems: a modern JSON pipeline DSL and a legacy set of Rust-native pipelines and analyzers. The Rust side is larger, harder to maintain, and partially redundant with the JSON side. The CLI command surface reflects the old Rust structure (nested subcommands) rather than the JSON model (self-describing pipelines).

Goals:
- Document every gap where the JSON API currently cannot replace Rust
- Extend the JSON DSL to close those gaps
- Delete all Rust code the JSON now replaces
- Flatten the CLI to match the JSON-first model
- Remove stale documentation

---

## What the JSON API Currently Cannot Handle

### 1. Taint analysis (security pipelines)
SQL injection, SSRF, XSS, XXE — 14 Rust files across 7 languages in `src/audit/pipelines/*/`. These track data flow from sources (user input, env vars) through a CFG to sinks (SQL execute, network calls), filtered by sanitizers. The hardcoded source/sink/sanitizer tables live in `src/graph/taint.rs` (~1,545 LOC).

**Why JSON cannot handle it today:** Taint propagation is stateful CFG traversal — no JSON stage exists to express it.

### 2. Cross-file analyzers
Three Rust analyzers in `src/audit/analyzers/`:
- `coupling.rs` (354 LOC) — fan-in/fan-out edge counting per file
- `dead_exports.rs` (346 LOC) — exported symbols with no incoming references
- `duplicate_symbols.rs` (178 LOC) — same symbol name defined in multiple files

**Why JSON cannot handle it today:** No stages for edge-count metrics, unreferenced-symbol filtering, or cross-file deduplication.

### 3. Nested CLI dispatch
`src/main.rs` (~938 LOC) branches across 6 audit subcommands, each calling hardcoded Rust functions (`security_pipelines_for_language`, `scalability_pipelines_for_language`, etc.) that return static pipeline lists. JSON pipelines self-declare their language and category, making this dispatch layer redundant.

---

## Design

### Approach: JSON-First, Delete-After (Option 1)

Extend the JSON DSL → write JSON equivalents → validate output parity → delete Rust → flatten CLI.

---

### Section 1: JSON DSL Extensions

Five additions to `src/graph/pipeline.rs` and `src/graph/executor.rs`:

#### 1. New `taint` GraphStage

```json
{
  "taint": {
    "sources": [
      {"pattern": "request.body", "kind": "user_input"},
      {"pattern": "req.query",    "kind": "user_input"},
      {"pattern": "os.environ",   "kind": "env_var"}
    ],
    "sinks": [
      {"pattern": "cursor.execute",  "vulnerability": "sql_injection"},
      {"pattern": "db.query",        "vulnerability": "sql_injection"}
    ],
    "sanitizers": [
      {"pattern": "escape"},
      {"pattern": "prepare"}
    ]
  }
}
```

The executor performs CFG-based taint propagation (BFS along `FlowsTo` edges, stopped by `SanitizedBy`). Each source→sink path found becomes one `PipelineNode` carrying `file`, `line`, `sink`, and `vulnerability` as metrics, consumed by the subsequent `flag` stage.

On CFG parse failure: emit a `warning`-severity diagnostic and skip the file — never hard-fail the pipeline.

#### 2. New `find_duplicates` GraphStage

```json
{"find_duplicates": {"by": "name", "min_count": 2}}
```

Groups the current node set by the specified property across the entire workspace. Keeps only groups where the count meets `min_count`. Each group becomes one `PipelineNode` with `name`, `count`, and `files` metrics.

#### 3. Extended `compute_metric`

Two new options:
- `"compute_metric": "efferent_coupling"` — count of outgoing `Imports` edges per file node
- `"compute_metric": "afferent_coupling"` — count of incoming `Imports` + `Calls` edges per file node

Result stored as `efferent_coupling` / `afferent_coupling` in the node's metric map, usable in subsequent `where` filters and `flag` message templates.

#### 4. New `WhereClause` predicates

Four new fields on `WhereClause`:

| Field | Type | Meaning |
|---|---|---|
| `efferent_coupling` | `NumericPredicate` | Fan-out threshold after `compute_metric: efferent_coupling` |
| `afferent_coupling` | `NumericPredicate` | Fan-in threshold after `compute_metric: afferent_coupling` |
| `unreferenced` | `bool` | Symbol has no incoming `Calls`/`Imports` edges from outside its own file |
| `is_entry_point` | `bool` | File is an entry point (`main`, `lib`, `index`, `__init__`, `__main__`, `mod`) |

Unknown `WhereClause` fields are ignored (existing serde default behaviour — no new failure modes).

#### 5. Executor changes

`src/graph/executor.rs` gains three new match arms: `GraphStage::Taint`, `GraphStage::FindDuplicates`, and the new metric/predicate variants. The taint propagation algorithm moves from `src/audit/pipelines/*/` into the executor (which already holds a reference to the `CodeGraph` with CFG edges). The `SOURCES`/`SINKS`/`SANITIZERS` const arrays in `taint.rs` are deleted; patterns come from the JSON stage definition.

---

### Section 2: New JSON Pipeline Files (21 files in `src/audit/builtin/`)

#### Security pipelines (18 files)

Example — `sql_injection_python.json`:

```json
{
  "pipeline": "sql_injection_python",
  "category": "security",
  "languages": ["python"],
  "graph": [
    {
      "taint": {
        "sources": [
          {"pattern": "request.form",       "kind": "user_input"},
          {"pattern": "request.args",       "kind": "user_input"},
          {"pattern": "request.data",       "kind": "user_input"},
          {"pattern": "os.environ.get",     "kind": "env_var"}
        ],
        "sinks": [
          {"pattern": "cursor.execute",     "vulnerability": "sql_injection"},
          {"pattern": "cursor.executemany", "vulnerability": "sql_injection"},
          {"pattern": "db.execute",         "vulnerability": "sql_injection"}
        ],
        "sanitizers": [
          {"pattern": "escape"},
          {"pattern": "quote"},
          {"pattern": "prepare"}
        ]
      }
    },
    {
      "flag": {
        "pattern": "sql_injection",
        "message": "SQL injection: tainted value reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

Full matrix:

| Vulnerability | Languages |
|---|---|
| `sql_injection` | python, go, java, javascript, typescript, php, csharp, cpp |
| `ssrf` | python, javascript, php, go, java, csharp |
| `xss` | javascript, typescript |
| `xxe` | python, java, csharp |

#### Cross-file analyzer replacements (3 files)

**`coupling.json`:**
```json
{
  "pipeline": "coupling",
  "category": "code_style",
  "graph": [
    {"select": "file", "exclude": {"or": [{"is_test_file": true}, {"is_barrel_file": true}]}},
    {"compute_metric": "efferent_coupling"},
    {"where": {"efferent_coupling": {"gte": 8}}},
    {
      "flag": {
        "pattern": "high_efferent_coupling",
        "message": "{{file}} imports from {{efferent_coupling}} modules (fan-out too high)",
        "severity": "warning"
      }
    }
  ]
}
```

**`dead_exports.json`:**
```json
{
  "pipeline": "dead_exports",
  "category": "code_style",
  "graph": [
    {
      "select": "symbol",
      "where": {"and": [{"exported": true}, {"is_entry_point": false}]},
      "exclude": {"or": [{"is_test_file": true}, {"is_generated": true}]}
    },
    {"where": {"unreferenced": true}},
    {
      "flag": {
        "pattern": "dead_export",
        "message": "{{name}} in {{file}} is exported but never referenced",
        "severity": "warning"
      }
    }
  ]
}
```

**`duplicate_symbols.json`:**
```json
{
  "pipeline": "duplicate_symbols",
  "category": "code_style",
  "graph": [
    {
      "select": "symbol",
      "where": {"exported": true},
      "exclude": {"or": [{"is_test_file": true}, {"is_generated": true}]}
    },
    {"find_duplicates": {"by": "name", "min_count": 2}},
    {
      "flag": {
        "pattern": "duplicate_symbol",
        "message": "{{name}} is defined in {{count}} files — potential naming collision",
        "severity": "info"
      }
    }
  ]
}
```

---

### Section 3: Rust Deletion Map

All deletions happen after Phase A validation confirms output parity.

#### Files deleted in full

```
src/audit/pipelines/python/sql_injection.rs
src/audit/pipelines/python/ssrf.rs
src/audit/pipelines/javascript/xss_dom_injection.rs
src/audit/pipelines/javascript/ssrf.rs
src/audit/pipelines/typescript/          (entire directory)
src/audit/pipelines/go/sql_injection.rs
src/audit/pipelines/go/ssrf_open_redirect.rs
src/audit/pipelines/java/java_ssrf.rs
src/audit/pipelines/java/sql_injection.rs
src/audit/pipelines/java/xxe.rs
src/audit/pipelines/php/sql_injection.rs
src/audit/pipelines/php/ssrf.rs
src/audit/pipelines/csharp/csharp_ssrf.rs
src/audit/pipelines/csharp/sql_injection.rs
src/audit/pipelines/csharp/xxe.rs
src/audit/analyzers/coupling.rs
src/audit/analyzers/dead_exports.rs
src/audit/analyzers/duplicate_symbols.rs
src/graph/taint.rs
audit_plans/                             (entire directory — 23 stale planning docs)
docs/superpowers/plans/                  (internal planning docs)
```

#### Files shrunk but kept

| File | What's removed |
|---|---|
| `src/audit/analyzers/mod.rs` | 3 module declarations |
| `src/audit/pipelines/python/mod.rs` | `sql_injection`, `ssrf` modules |
| `src/audit/pipelines/javascript/mod.rs` | `xss_dom_injection`, `ssrf` modules |
| `src/audit/pipelines/go/mod.rs` | `sql_injection`, `ssrf_open_redirect` modules |
| `src/audit/pipelines/java/mod.rs` | 3 security modules |
| `src/audit/pipelines/php/mod.rs` | `sql_injection`, `ssrf` modules |
| `src/audit/pipelines/csharp/mod.rs` | 3 security modules |
| `src/audit/engine.rs` | `ProjectAnalyzer` dispatch calls |
| `src/graph/mod.rs` | `taint` module import |

#### What stays in Rust (intentionally)

| Module | Reason |
|---|---|
| `src/graph/executor.rs` | Gains taint algorithm + new stage arms |
| `src/graph/cfg.rs` + `cfg_languages/` | CFG construction still needed for taint |
| `src/graph/builder.rs` | Graph construction, unchanged |
| `src/graph/metrics.rs` | Cyclomatic/cognitive complexity, unchanged |
| `src/audit/pipelines/*/primitives.rs` | Language-specific AST helpers used by executor |
| `src/audit/pipelines/helpers.rs` | `is_test_file`, `is_barrel_file`, etc., used by executor |

---

### Section 4: CLI Simplification

#### Before
```
virgil audit code-quality tech-debt   --pipeline X --per-page N --page N
virgil audit code-quality complexity  --pipeline X --per-page N --page N
virgil audit code-quality code-style  --pipeline X
virgil audit security                 --pipeline X --per-page N --page N
virgil audit scalability              --pipeline X --per-page N --page N
virgil audit architecture             --pipeline X --per-page N --page N
```

#### After
```
virgil audit [--dir <path> | --s3 <uri>]
             [--language <lang,...>]
             [--pipeline <name,...>]
             [--category <name,...>]
             [--format table|json|csv]
             [--per-page N] [--page N]
```

`--category` values match the `category` field already present in every JSON pipeline file: `security`, `architecture`, `code_style`, `tech_debt`, `complexity`, `scalability`.

#### Code impact
- `src/cli.rs`: Remove `AuditSubcommand`, `AuditCodeQualityCommands`, and all nested arg structs. Replace with a single flat `AuditArgs` struct. ~200 → ~80 lines.
- `src/main.rs`: Remove the 6-arm audit dispatch block and all `*_pipelines_for_language()` functions. Single `Commands::Audit` arm delegates entirely to `AuditEngine`. ~938 → ~400 lines.

---

### Section 5: Validation Strategy

#### Phase A — Extend and validate (no deletions)
1. Add DSL extensions to `pipeline.rs` and `executor.rs`
2. Write 21 new JSON builtin files
3. Run old Rust pipelines and new JSON pipelines side-by-side against the existing integration test corpus
4. Fix gaps until findings are equivalent

#### Phase B — Delete Rust
1. Delete Rust security pipelines and `taint.rs`
2. Delete 3 analyzer files
3. `cargo test` must pass before proceeding to Phase C

#### Phase C — Flatten CLI
1. Rewrite `cli.rs` and `main.rs`
2. Smoke-test all `--category` and `--pipeline` flag combinations

#### Parity gate
The existing per-language integration tests (142 JavaScript, 141 TypeScript, 131 C#, etc.) serve as the parity gate. Before deleting any Rust pipeline, its JSON equivalent must pass the same test cases.

---

## Summary

| Area | Before | After |
|---|---|---|
| Security pipelines | 14 Rust files + hardcoded taint tables | 18 JSON files + taint stage in executor |
| Cross-file analyzers | 3 Rust files (~878 LOC) | 3 JSON files |
| CLI audit commands | 6 nested subcommands | 1 flat `audit` command |
| Audit dispatch in `main.rs` | ~500 LOC branching | ~50 LOC single arm |
| Stale docs | `audit_plans/` (23 files) | Deleted |
| Estimated audit subsystem | ~15 KLOC | ~9 KLOC |
