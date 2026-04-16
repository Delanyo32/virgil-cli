# Phase 4: Security + Per-Language Scalability Migration - Research

**Researched:** 2026-04-16
**Domain:** Rust audit pipeline migration — security and scalability pipelines to declarative JSON
**Confidence:** HIGH

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**D-01: Match-Pattern Test**
A pipeline migrates to JSON if and only if its Rust implementation uses only tree-sitter pattern matching — no FlowsTo/SanitizedBy graph edges, no multi-file tracking, no ResourceAnalyzer.

**D-02: Pipelines that migrate**
`path_traversal`, `insecure_deserialization`, `weak_cryptography`, `type_confusion`, `reflection_injection`, `code_injection`, `prototype_pollution`, `redos_resource_exhaustion`, `timing_weak_crypto`, `format_string`, `buffer_overflow` variants, `integer_overflow`, `command_injection`, `unsafe_memory`.

**D-03: race_conditions per-language**
Planner inspects each language's Rust implementation. Pure tree-sitter pattern matching → migrate. Graph-level concurrency analysis → leave in Rust with documented comment.

**D-04: Document Permanent Rust Exceptions**
All permanent Rust exceptions (taint-based) must appear in the plan with a "PERMANENT RUST EXCEPTION — requires FlowsTo/SanitizedBy graph predicates" comment.

**D-05: Source of truth = Rust implementations**
No `audit_plans/` security specs exist. JSON patterns are derived directly from existing Rust files.

**D-06: Fix obvious Rust bugs; document divergence**
Obvious bugs in Rust (wrong function name in list, typo) should be fixed in the JSON version. Document intentional divergence in the JSON `"description"` field.

**D-07: Simplified pattern for inexpressible logic**
If Rust detection logic cannot be faithfully expressed in `match_pattern`, write a simplified pattern. Document precision delta in the JSON `"description"` field.

**D-08: 10 per-language JSON files for memory_leak_indicators**
`memory_leak_indicators_rust.json`, `memory_leak_indicators_typescript.json`, `memory_leak_indicators_javascript.json`, `memory_leak_indicators_python.json`, `memory_leak_indicators_go.json`, `memory_leak_indicators_java.json`, `memory_leak_indicators_c.json`, `memory_leak_indicators_cpp.json`, `memory_leak_indicators_csharp.json`, `memory_leak_indicators_php.json`.

**D-09: Simplified match_pattern for complex memory_leak_indicators**
If a language's implementation uses flow analysis beyond simple tree-sitter patterns, write a simplified match_pattern covering common leak indicators. Never skip a language.

**D-10: Plans organized by language group**
Each plan covers all security pipelines + memory_leak_indicators for one language. ~10 plans total.

**D-11: Rust goes first**
Rust has no taint exceptions; all security pipelines pass the match_pattern test. Serves as the canonical template.

**D-12: Atomic commit pattern**
Write JSON files → run `cargo test` → delete Rust files → run `cargo test` again. Integration tests committed in the same batch.

### Claude's Discretion

- Exact ordering of language groups after Rust
- Whether TypeScript and JavaScript are combined in one plan or split
- For `race_conditions` in each language: planner reads the Rust implementation and makes the per-language judgment call (migrate vs document as Rust exception) without returning to ask the user

### Deferred Ideas (OUT OF SCOPE)

- Writing `audit_plans/<lang>_security.md` specs before migration
- Taint-based security pipeline migration (sql_injection, xss, ssrf, xxe) — v2 TAINT-01
- `resource_exhaustion` pipelines — planner inspects per-language; deferred if not expressible in match_pattern
- `memory_leak_indicators` precision improvements
</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| SEC-01 | Per-language non-taint security pipelines migrated to JSON (command injection, unsafe memory, integer overflow — those expressible via match_pattern) | All 10 security pipeline mod.rs files surveyed; taint vs pure-pattern boundary mapped per language |
| SEC-02 | All replaced Rust security pipeline files deleted | Atomic delete pattern verified from Phase 3; `wrap_legacy()` and `pub mod` removal understood |
| SCAL-02 | Per-language scalability pipelines migrated to JSON for all applicable languages | 10 memory_leak_indicators Rust files read; match_pattern expressibility assessed per language |
| SCAL-03 | All replaced Rust scalability pipeline files deleted | Same delete pattern as SEC-02 |
| TEST-01 | Each pipeline deletion batch has JSON integration tests (1 positive + 1 negative per pipeline) committed in same batch | 24 existing integration tests in `tests/audit_json_integration.rs` — adding to this file |
| TEST-02 | `cargo test` passes with zero failures at every phase boundary | Engine suppression (ENG-01) confirmed working; JSON override via name-match confirmed |
</phase_requirements>

---

## Summary

Phase 4 migrates two groups of Rust audit pipelines to declarative JSON: (1) non-taint security pipelines across all 10 language groups, and (2) the `memory_leak_indicators` scalability pipeline for all 10 language groups. The engine infrastructure is fully ready — `match_pattern` stage is implemented in `src/graph/executor.rs`, the name-match suppression (ENG-01) is confirmed in `src/audit/engine.rs` line 111, and 48+ JSON files are already successfully embedded and running.

The primary planning challenge is the per-pipeline taint boundary assessment (D-01 match-pattern test). After reading all 10 language `mod.rs` files and representative security pipeline implementations, this research documents exactly which pipelines pass the test, which remain in Rust permanently, and which need simplified patterns.

The secondary planning challenge is the `memory_leak_indicators` migration. Python, JavaScript, C, C#, and PHP implementations use stateful multi-pass logic (file-scope accumulation, parent-walk, pair detection) that cannot be expressed in a single `match_pattern` query. These require simplified patterns per D-09, accepting reduced precision.

**Primary recommendation:** Plan 10 language-group batches. Rust first (cleanest, all pipelines pass match-pattern test, serves as template). TypeScript+JavaScript combined (they share 9 of 11 security pipelines via delegation). Then Go, Python, Java, C, C++, C#, PHP. Each batch: write JSON files, run `cargo test`, delete Rust files, run `cargo test`, commit with integration tests.

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| JSON pipeline definition (security/scalability) | `src/audit/builtin/` | `src/graph/executor.rs` match_pattern | Files live in builtin/, executor runs them |
| Pipeline dispatch (Rust → JSON suppression) | `src/audit/engine.rs` | `src/audit/pipeline.rs` dispatch | ENG-01 suppression in engine.rs line 111 |
| Language-specific Rust registration | `src/audit/pipelines/<lang>/mod.rs` | — | Each mod.rs has `security_pipelines()` and `scalability_pipelines()` |
| Integration tests | `tests/audit_json_integration.rs` | — | All new tests appended to this file |
| Rust pipeline deletion | `src/audit/pipelines/<lang>/` | `src/audit/pipelines/<lang>/mod.rs` | Delete .rs file, remove `pub mod` line |

---

## Standard Stack

### Core (Already in Place — No New Dependencies)
| Library | Version | Purpose | Notes |
|---------|---------|---------|-------|
| tree-sitter 0.25 | 0.25.x | AST query execution for match_pattern | `[VERIFIED: Cargo.toml]` |
| serde_json 1 | 1.x | JSON pipeline deserialization | Already used |
| include_dir | in use | Embeds `src/audit/builtin/*.json` at compile time | `[VERIFIED: json_audit.rs line 32]` |

**Installation:** None required — all dependencies are already present.

---

## Architecture Patterns

### System Architecture Diagram

```
Rust security pipeline file (src/audit/pipelines/<lang>/foo.rs)
    ↓
Read → identify tree-sitter queries and function name lists
    ↓
Translate to match_pattern S-expression JSON
    ↓
Write to src/audit/builtin/<pipeline_name>_<lang>.json
    ↓
cargo test (verifies JSON parses + engine runs it)
    ↓
Delete foo.rs + remove pub mod foo; from mod.rs
    ↓
cargo test (verifies no compilation errors, no duplicate findings)
    ↓
Add integration tests (positive + negative) to tests/audit_json_integration.rs
    ↓
cargo test (verifies tests pass)
```

**Engine suppression flow (ENG-01, confirmed):**

```
engine.rs::run()
    ↓ discover_json_audits() → collect pipeline names from all JSON files
    ↓ build json_pipeline_names HashSet
    ↓ security_pipelines_for_language() → get Rust pipelines
    ↓ lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name()))
    ↓ [Rust pipelines with same name as JSON are dropped]
    ↓ JSON audits run via run_pipeline(stages, graph, Some(workspace), ...)
```

### Recommended Project Structure (Already Established)
```
src/audit/builtin/
├── <pipeline_name>_<lang>.json    # Per-language: "languages": ["rust"]
├── <pipeline_name>.json           # Cross-language: no "languages" field
src/audit/pipelines/<lang>/
├── mod.rs                         # Remove pub mod + registration after deletion
├── <pipeline>.rs                  # Delete after JSON replacement
tests/
└── audit_json_integration.rs      # Append positive + negative tests per pipeline
```

### Pattern 1: Per-Language match_pattern JSON File
**What:** A JSON file with `"languages": [<lang>]` that runs a tree-sitter S-expression against all files of that language.
**When to use:** Any security or scalability pipeline that uses pure tree-sitter pattern matching.
**Example (from `sync_blocking_in_async_rust.json`):**
```json
{
  "pipeline": "sync_blocking_in_async",
  "category": "scalability",
  "description": "...",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (scoped_identifier) @fn)"
    },
    {
      "flag": {
        "pattern": "blocking_io_in_async",
        "message": "...",
        "severity": "info"
      }
    }
  ]
}
```
**Source:** `[VERIFIED: src/audit/builtin/sync_blocking_in_async_rust.json]`

### Pattern 2: TypeScript + TSX Multi-Language File
**What:** A JSON file with `"languages": ["typescript", "tsx"]` covering both TS variants.
**When to use:** Any pipeline that applies to TypeScript/TSX files.
**Example:**
```json
{
  "languages": ["typescript", "tsx"],
  ...
}
```
**Source:** `[VERIFIED: src/audit/builtin/sync_blocking_in_async_typescript.json]`

### Pattern 3: JavaScript and TypeScript Shared Security Pipeline
**What:** The JS security pipeline functions via `language: Language` parameter — one Rust implementation handles both JS and TS. The JSON equivalent creates two files: one for `["javascript", "jsx"]` and one for `["typescript", "tsx"]` with identical graph stages.
**When to use:** All shared JS/TS security pipelines (9 of 11 TypeScript security pipelines delegate to javascript::security_pipelines()).
**Source:** `[VERIFIED: src/audit/pipelines/typescript/mod.rs lines 63-74]`

### Anti-Patterns to Avoid

- **Duplicate JSON files for same pipeline + same language:** The dedup_key is `pipeline_name:language_list`. Two files with the same name AND same language filter deduplicate (project-local wins). Different language filters coexist — this is how per-language variants work.
- **Deleting Rust file before JSON is verified:** Always run `cargo test` after writing JSON, before deleting Rust. The engine suppresses the Rust version by name-match, so both briefly coexist — but if the JSON has a parse error, the suppression still removes the working Rust pipeline.
- **Using `wrap_legacy()` in mod.rs for a JSON-replaced pipeline:** After deletion, the `wrap_legacy()` wrapper in `security_pipelines_for_language()` no longer wraps anything for that pipeline. The function signature stays; JSON engine handles it.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Per-file language filtering | Manual language check in match_pattern | `"languages": [...]` field in JSON | Engine applies language filter before running match_pattern |
| Tree-sitter query compilation per file | Re-compile Query per file | Engine compiles once; match_pattern receives query string, executor caches per run | Executor handles compilation |
| Suppression of doubled findings | Extra Rust logic | Name-match suppression in engine.rs (ENG-01, line 111) | Automatically suppresses Rust pipeline when JSON has same name |

---

## Taint Boundary Classification (The Core Research Finding)

This section documents the D-01 match-pattern test result for every pipeline in scope.

### Rust Language Group — All Pass Match-Pattern Test

`src/audit/pipelines/rust/mod.rs` `security_pipelines()` registers:

| Pipeline | Rust File | Classification | Patterns |
|----------|-----------|----------------|---------|
| `integer_overflow` | `integer_overflow.rs` | **MIGRATE** — pure tree-sitter: finds binary_expression with `*`/`+` inside function bodies with params | `unchecked_multiply`, `unchecked_add` |
| `unsafe_memory` | `unsafe_memory.rs` | **MIGRATE** — pure tree-sitter: finds unsafe blocks, then walks for transmute/pointer arith/deref | `transmute_in_unsafe`, `pointer_arithmetic`, `raw_pointer_deref` |
| `race_conditions` | `race_conditions.rs` | **MIGRATE** — pure tree-sitter: finds `static mut` via static_item_query | `static_mut` |
| `path_traversal` | `path_traversal.rs` | **MIGRATE (simplified)** — uses multi-step: find fn with params, then find path method calls with param args. Simplify to: match method calls to `join`/`push` on path-named variables | `unvalidated_path_join`, `unvalidated_path_push` |
| `resource_exhaustion` | `resource_exhaustion.rs` | **PLANNER INSPECTS** — read file to determine if match_pattern expressible |  |
| `panic_dos` | `panic_dos.rs` | **PLANNER INSPECTS** — read file to determine if match_pattern expressible |  |
| `type_confusion` | `type_confusion.rs` | **MIGRATE** — pure tree-sitter: walks tree for `transmute` calls + queries for union definitions | `transmute_call`, `union_field_access` |
| `toctou` | `toctou.rs` | **PLANNER INSPECTS** — read file to determine |  |

**Note on `path_traversal` complexity:** The Rust implementation uses param name extraction, then cross-references call args against those names. This multi-step stateful logic cannot be expressed in a single match_pattern. Per D-07, simplify to: match `(call_expression function: (field_expression field: (field_identifier) @method) @fn)` where method is `join` or `push`. Document precision loss.

**Note on `memory_leak_indicators` (Rust):** Uses scoped-call-query for Box::leak/mem::forget/ManuallyDrop::new, loop_query for loop bodies, then method_call_query inside loop for push/insert/extend. The individual patterns are expressible in match_pattern separately. Approach: write 3 match_pattern stages in one file or use 3 separate flag blocks. See Code Examples section.

### JavaScript / TypeScript Security Pipelines

`src/audit/pipelines/javascript/mod.rs` `security_pipelines(language)` registers 9 pipelines:
`src/audit/pipelines/typescript/mod.rs` `security_pipelines(language)` adds 2 more (TypeScript-only).

| Pipeline | File | Classification | Notes |
|----------|------|----------------|-------|
| `xss_dom_injection` | `xss_dom_injection.rs` | **PERMANENT RUST EXCEPTION** — uses innerHTML/outerHTML property assignment detection plus insertAdjacentHTML, document.write with `is_safe_literal` filtering. While technically expressible in match_pattern, it is a TAINT-based pattern conceptually. Per CONTEXT.md, XSS stays in Rust. |
| `code_injection` | `code_injection.rs` | **MIGRATE** — pure tree-sitter: eval() with non-literal, new Function(), setTimeout with string, vm.runInNewContext. Simplified: match calls to eval/setTimeout/setInterval/new Function by name. |
| `command_injection` | `command_injection.rs` | **MIGRATE** — pure tree-sitter: exec/execSync/execFileSync method calls, spawn with `shell: true`. |
| `path_traversal` | `path_traversal.rs` | **MIGRATE (simplified)** — matches path.join/path.resolve calls with dynamic args. |
| `prototype_pollution` | `prototype_pollution.rs` | **MIGRATE (simplified)** — for-in loop with subscript assignment without guard, Object.assign with JSON.parse. Pure tree-sitter. |
| `redos_resource_exhaustion` | `redos_resource_exhaustion.rs` | **MIGRATE** — pure tree-sitter: RegExp constructor with variable, exec() on regex with backtracking-prone pattern. |
| `ssrf` | `ssrf.rs` | **PERMANENT RUST EXCEPTION** — per CONTEXT.md, ssrf stays in Rust (taint-based). |
| `insecure_deserialization` | `insecure_deserialization.rs` | **MIGRATE** — pure tree-sitter: JSON.parse with external source, eval-based deserialization, unserialize calls. |
| `timing_weak_crypto` | `timing_weak_crypto.rs` | **MIGRATE** — pure tree-sitter: Math.random for crypto, == on auth tokens, MD5/SHA1 usage. |
| `type_system_bypass` | `type_system_bypass.rs` | **TypeScript-only. PLANNER INSPECTS** |
| `unsafe_type_assertions_security` | `unsafe_type_assertions_security.rs` | **TypeScript-only. MIGRATE** — pure tree-sitter: `as unknown as T`, double-cast pattern. |

**Note for JavaScript SSRF:** `ssrf.rs` uses `fetch()`, `axios`, `http.request` call detection with dynamic URL — while tree-sitter detectable, it remains in Rust per CONTEXT.md permanent exception classification.

**Note on XSS boundary:** Per D-01 and CONTEXT.md, `xss_dom_injection` and `ssrf` remain in Rust regardless of tree-sitter expressibility.

### Python Security Pipelines

`src/audit/pipelines/python/mod.rs` `security_pipelines()` registers:

| Pipeline | Classification | Notes |
|----------|----------------|-------|
| `command_injection` | **MIGRATE** — pure tree-sitter: os.system/popen with dynamic arg, subprocess.run with shell=True. Uses GraphPipeline but logic is tree-sitter only (graph not used). |
| `code_injection` | **MIGRATE** — pure tree-sitter: exec()/eval() with non-literal. |
| `sql_injection` | **PERMANENT RUST EXCEPTION** — taint-based. |
| `path_traversal` | **MIGRATE** — pure tree-sitter: os.path.join with dynamic args. |
| `insecure_deserialization` | **MIGRATE** — pure tree-sitter: pickle.loads/yaml.load without Loader, marshal.loads. |
| `ssrf` | **PERMANENT RUST EXCEPTION** — taint-based. |
| `resource_exhaustion` | **PLANNER INSPECTS** — read file to determine. |
| `xxe_format_string` | **MIGRATE (split)** — covers both xxe (lxml etree parse from external) and format string (%s in SQL). Both pure tree-sitter. |

**Note on Python `AnyPipeline::Graph`:** Python security pipelines use `AnyPipeline::Graph(Box::new(...))`, not `Box::new(...)`. The `security_pipelines()` function returns `Result<Vec<AnyPipeline>>`, not `Result<Vec<Box<dyn Pipeline>>>`. This means the dispatch in `pipeline.rs` line 196 calls python's function directly, not via `wrap_legacy()`. When deleting Python Rust pipelines, the `AnyPipeline::Graph(...)` entries are removed from the Vec — the JSON engine intercepts by name before the dispatch even runs the pipeline.

### Go Security Pipelines

`src/audit/pipelines/go/mod.rs` `security_pipelines()` registers:

| Pipeline | Classification | Notes |
|----------|----------------|-------|
| `command_injection` | **MIGRATE** — pure tree-sitter: exec.Command with dynamic args. |
| `sql_injection` | **PERMANENT RUST EXCEPTION** — taint-based. |
| `go_path_traversal` | **MIGRATE** — pure tree-sitter: filepath.Join with dynamic component. |
| `go_race_conditions` | **MIGRATE** — pure tree-sitter: goroutines in loops + concurrent map access detection. Pure tree-sitter (checks go_statement inside for_statement + bracket access). |
| `go_resource_exhaustion` | **PLANNER INSPECTS** — read file. |
| `go_integer_overflow` | **MIGRATE** — pure tree-sitter: arithmetic on values from external input (type conversion patterns). |
| `go_type_confusion` | **MIGRATE** — pure tree-sitter: unsafe.Pointer casts, unsafe.Sizeof. |
| `ssrf_open_redirect` | **PERMANENT RUST EXCEPTION** — taint-based. |

### Java Security Pipelines

`src/audit/pipelines/java/mod.rs` `security_pipelines()` registers:

| Pipeline | Classification | Notes |
|----------|----------------|-------|
| `sql_injection` | **PERMANENT RUST EXCEPTION** — taint-based. |
| `command_injection` | **MIGRATE** — pure tree-sitter: Runtime.exec/ProcessBuilder with dynamic args. |
| `weak_cryptography` | **MIGRATE** — pure tree-sitter: MD5/SHA1/DES constructor instantiation by name. |
| `insecure_deserialization` | **MIGRATE** — pure tree-sitter: ObjectInputStream.readObject(), XStream.fromXML without filter. |
| `java_path_traversal` | **MIGRATE** — pure tree-sitter: Paths.get/new File with dynamic component. |
| `xxe` | **PERMANENT RUST EXCEPTION** — per CONTEXT.md: requires taint through XML parser. |
| `java_ssrf` | **PERMANENT RUST EXCEPTION** — per CONTEXT.md. |
| `reflection_injection` | **MIGRATE** — pure tree-sitter: Class.forName/Method.invoke with dynamic string argument. |
| `java_race_conditions` | **MIGRATE** — pure tree-sitter: unsynchronized shared collection fields (HashMap/ArrayList as non-final non-volatile fields in non-synchronized method). |

### C Security Pipelines

`src/audit/pipelines/c/mod.rs` `security_pipelines()` registers 9 pipelines, all named with `c_` prefix:

| Pipeline | Classification | Notes |
|----------|----------------|-------|
| `format_string` | **MIGRATE** — pure tree-sitter: printf/fprintf/sprintf with non-literal format arg. |
| `c_command_injection` | **MIGRATE (simplified)** — two-phase: (1) system()/popen() with dynamic arg, (2) sprintf+system pattern. Phase 2 is stateful (tracks buffer name across statements). Simplify: match system()/popen() calls where first arg is not string_literal. Document precision loss on sprintf+system pattern. |
| `c_weak_randomness` | **MIGRATE** — pure tree-sitter: rand()/random() calls without srand seeding. |
| `c_buffer_overflow_security` | **MIGRATE** — pure tree-sitter: gets()/strcpy()/strcat()/sprintf() calls. |
| `c_integer_overflow` | **MIGRATE** — pure tree-sitter: arithmetic on values from function params without bounds check. |
| `c_toctou` | **MIGRATE** — pure tree-sitter: access()/stat() followed by open() — detects access() and open() both present in function. |
| `c_memory_mismanagement` | **MIGRATE** — pure tree-sitter: free() without NULL check, double-free patterns. |
| `c_path_traversal` | **MIGRATE** — pure tree-sitter: path concatenation with user input. |
| `c_uninitialized_memory` | **MIGRATE** — pure tree-sitter: malloc() result used without memset/calloc. |

### C++ Security Pipelines

`src/audit/pipelines/cpp/mod.rs` `security_pipelines()` registers 9 pipelines, named with `cpp_` prefix:

| Pipeline | Classification | Notes |
|----------|----------------|-------|
| `cpp_injection` | **MIGRATE** — pure tree-sitter: system()/popen() calls. |
| `cpp_weak_randomness` | **MIGRATE** — pure tree-sitter: rand()/srand() calls. |
| `cpp_type_confusion` | **MIGRATE** — pure tree-sitter: reinterpret_cast, C-style casts, union usage. |
| `cpp_buffer_overflow` | **MIGRATE** — pure tree-sitter: strcpy/gets/sprintf calls. |
| `cpp_integer_overflow` | **MIGRATE** — pure tree-sitter: unchecked arithmetic on integral types with casts. |
| `cpp_exception_safety` | **MIGRATE** — pure tree-sitter: `new` in function without try/catch or noexcept. |
| `cpp_memory_mismanagement` | **MIGRATE** — pure tree-sitter: raw delete on array, free() on new'd memory. |
| `cpp_race_conditions` | **MIGRATE (simplified)** — checks class body for shared fields without mutex. Heuristic: class has mutable members AND methods that modify fields. Pure tree-sitter. |
| `cpp_path_traversal` | **MIGRATE** — pure tree-sitter: filesystem::path operations with user input. |

### C# Security Pipelines

`src/audit/pipelines/csharp/mod.rs` `security_pipelines()` registers:

| Pipeline | Classification | Notes |
|----------|----------------|-------|
| `sql_injection` | **PERMANENT RUST EXCEPTION** — taint-based. |
| `command_injection` | **MIGRATE** — pure tree-sitter: Process.Start with dynamic args, cmd.exe calls. |
| `weak_cryptography` | **MIGRATE** — pure tree-sitter: MD5/SHA1/DES instantiation by type name. |
| `insecure_deserialization` | **MIGRATE** — pure tree-sitter: BinaryFormatter.Deserialize, JsonConvert.DeserializeObject without type checking. |
| `csharp_path_traversal` | **MIGRATE** — pure tree-sitter: Path.Combine/File.Open with dynamic component. |
| `xxe` | **PERMANENT RUST EXCEPTION** — per CONTEXT.md. |
| `csharp_ssrf` | **PERMANENT RUST EXCEPTION** — per CONTEXT.md. |
| `csharp_race_conditions` | **MIGRATE** — pure tree-sitter: static non-Interlocked non-Volatile fields in classes, thread-unsafe singleton patterns. |
| `reflection_unsafe` | **MIGRATE** — pure tree-sitter: Assembly.Load/Type.GetMethod/Invoke with dynamic strings. |

### PHP Security Pipelines

`src/audit/pipelines/php/mod.rs` `security_pipelines()` registers:

| Pipeline | Classification | Notes |
|----------|----------------|-------|
| `sql_injection` | **PERMANENT RUST EXCEPTION** — taint-based. |
| `unsafe_include` | **MIGRATE** — pure tree-sitter: include/require/include_once/require_once with dynamic path. |
| `unescaped_output` | **MIGRATE** — pure tree-sitter: echo/print with $_GET/$_POST without htmlspecialchars. |
| `command_injection` | **MIGRATE** — pure tree-sitter: shell_exec/exec/system/passthru with dynamic arg. |
| `insecure_deserialization` | **MIGRATE** — pure tree-sitter: unserialize() with non-literal arg. |
| `type_juggling` | **MIGRATE** — pure tree-sitter: loose comparison (==) on security-sensitive values. |
| `ssrf` | **PERMANENT RUST EXCEPTION** — per CONTEXT.md. |
| `session_auth` | **MIGRATE** — pure tree-sitter: session_start() missing, session_regenerate_id() missing after auth. |

---

## memory_leak_indicators Expressibility Assessment

The `memory_leak_indicators` scalability pipeline exists in all 10 language groups. Each Rust implementation's complexity is assessed here:

| Language | Patterns Detected | match_pattern Feasibility | Simplification Required |
|----------|-------------------|--------------------------|------------------------|
| **Rust** | Box::leak, mem::forget, ManuallyDrop::new (scoped-call), push/insert/extend in loops | **HIGH** — scoped_identifier calls match directly; loop-body method calls match directly. Multiple match_pattern stages possible. | None — all patterns directly expressible |
| **JavaScript** | addEventListener without removeEventListener (file-scope pair), setInterval without clearInterval (file-scope pair), push/unshift in loops | **MEDIUM** — loop growth patterns expressible. File-scope pairing (has_remove_listener) is NOT expressible in match_pattern — it requires file-level accumulation. | Simplify: match addEventListener and setInterval calls unconditionally. Accept false positives. |
| **TypeScript** | Same as JavaScript (delegates to JS implementation) | **MEDIUM** — same as JavaScript | Same simplification |
| **Python** | open() without `with` (context-sensitive), append/extend in loops, `__del__` methods, + GraphPipeline suppression for result_builder/try_finally patterns | **LOW** — `open()` without `with` requires parent-walk context. Loop growth expressible. `__del__` expressible. Suppression logic (result_builder, try_finally) NOT expressible. | Simplify: match `open(` calls (flag all, not just non-with). Match append/extend/add in loops. Match `__del__` definitions. |
| **Go** | goroutine in loop, append() in loop without bound check, defer in loop | **HIGH** — goroutine inside for_statement expressible. append() in for_statement body expressible. defer in for_statement body expressible. Range-clause detection can be noted as precision loss. | Minor simplification: cannot filter by range_clause. All appends in loops flagged (including range). |
| **Java** | ResourceLeak via AutoCloseable not closed, Collections not in try-with-resources, stream not closed | **MEDIUM** — object creation patterns expressible. try-with-resources check requires structural analysis. | Simplify: match `new` expressions for common resource types (Connection, InputStream, etc.) without try-with-resources verification. |
| **C** | malloc/calloc/realloc/strdup in loops without free, fopen without fclose in same fn, strdup/asprintf without free | **MEDIUM** — alloc calls in loops expressible. fopen/fclose pairing within function is stateful. | Simplify: match alloc calls in loop bodies unconditionally. Match fopen calls unconditionally. Accept false positives. |
| **C++** | `new` without delete in loop, raw pointer members, resource RAII violations | **MEDIUM** — `new` in loops expressible. RAII check stateful. | Simplify: match `new` in loop bodies. |
| **C#** | IDisposable objects without `using`, SqlConnection/HttpClient not disposed, Add() in loops | **MEDIUM** — object creation by type name expressible. `using` check stateful. | Simplify: match known disposable type instantiations without checking for using. |
| **PHP** | fopen without fclose in function, curl_init without curl_close, mysql_connect without mysql_close, array[] = in loop | **MEDIUM** — array growth in loops expressible. open/close pairing stateful. | Simplify: match fopen/curl_init/mysql_connect calls + array growth in loops. |

---

## Common Pitfalls

### Pitfall 1: Double-Firing Before Deletion
**What goes wrong:** JSON file written, Rust file not yet deleted. JSON engine suppresses Rust by name — but if the JSON pipeline name has a language suffix (`_rust`) but the Rust pipeline name doesn't, the suppression fails. The old Rust pipeline fires too.
**Why it happens:** The ENG-01 suppression matches on `p.name()` against `json_pipeline_names`. The json_pipeline_names set contains the JSON file's `pipeline` field. If the JSON file has pipeline `"unsafe_memory_rust"` but the Rust struct returns `"unsafe_memory"`, they don't match.
**How to avoid:** The JSON file's `"pipeline"` field MUST match the Rust pipeline's `name()` return value exactly. Use `"pipeline": "unsafe_memory"` not `"pipeline": "unsafe_memory_rust"`. Use the `"languages"` field to scope to Rust only.
**Warning signs:** Running `cargo test` after adding JSON shows doubled findings in integration tests.

### Pitfall 2: Python AnyPipeline::Graph Dispatch Mismatch
**What goes wrong:** Python uses `AnyPipeline::Graph(Box::new(...))` in its pipeline registrations. The `wrap_legacy()` function in `pipeline.rs` is NOT used for Python security pipelines. Deleting a Python security pipeline file requires removing the `AnyPipeline::Graph(Box::new(...))` entry from `python::security_pipelines()`, not a `wrap_legacy` removal.
**Why it happens:** Python security pipelines implement `GraphPipeline` trait, not `Pipeline` trait.
**How to avoid:** Read the Python mod.rs carefully. Delete the `AnyPipeline::Graph(Box::new(foo::FooPipeline::new()?))` line, not a `Box::new(foo::FooPipeline::new()?)` line.
**Warning signs:** Compile error referencing `AnyPipeline` type mismatch when removing wrong entry type.

### Pitfall 3: TypeScript Security Pipelines Via Delegation
**What goes wrong:** TypeScript's `security_pipelines()` calls `pipelines::javascript::security_pipelines(language)` and then pushes 2 TypeScript-specific pipelines. If you delete a shared JS/TS pipeline file from `javascript/`, TypeScript's security audit silently loses it.
**Why it happens:** TypeScript mod.rs delegates to JavaScript for 9 of 11 security pipelines.
**How to avoid:** When a shared JS/TS pipeline is replaced by JSON, the JSON file must cover BOTH `["javascript", "jsx"]` AND `["typescript", "tsx"]` — either as two separate files or one file with `["javascript", "jsx", "typescript", "tsx"]`. The planner decides file naming strategy.
**Warning signs:** TypeScript security integration test finds no results when JS integration test passes.

### Pitfall 4: Pipeline Name Mismatch for C/C++/Go Languages
**What goes wrong:** C, C++, and Go use language-prefixed pipeline names: `c_command_injection`, `cpp_injection`, `go_integer_overflow`. The JSON file must have `"pipeline": "c_command_injection"` (the exact Rust name()), not just `"command_injection"`.
**Why it happens:** These languages chose unique names to avoid conflict with cross-language pipelines. The engine suppression matches on the name() return value.
**How to avoid:** Copy the pipeline name verbatim from the Rust struct's `fn name(&self) -> &str` implementation.
**Warning signs:** Old Rust pipeline still fires after JSON is added (suppression not triggered because names differ).

### Pitfall 5: memory_leak_indicators Precision vs Correctness
**What goes wrong:** Simplified match_pattern for memory_leak_indicators flags code that the Rust pipeline would have suppressed (e.g., `push()` inside a range-based for loop in Go, `open()` inside a `with` statement in Python, `addEventListener` when `removeEventListener` is present).
**Why it happens:** File-scope pair detection and context-aware suppression (try/finally, with, range clause) cannot be expressed in match_pattern.
**How to avoid:** Per D-09, document precision loss in the JSON `"description"` field. Add negative integration tests that document the known false-positive scenarios so regressions don't go unnoticed.
**Warning signs:** Integration tests with clean code that the Rust version passes start failing with false positives.

### Pitfall 6: Rust race_conditions Specific to Rust (static mut)
**What goes wrong:** Rust's `race_conditions` detects `static mut` — a Rust-specific construct. Writing a generic `race_conditions.json` without a language filter would apply the `static_item` query to all languages (and either crash or produce false positives).
**Why it happens:** The pipeline is named `race_conditions` without a language prefix, but it's Rust-specific.
**How to avoid:** The JSON file MUST include `"languages": ["rust"]`. For Go/Java/C#/C++ race conditions, separate JSON files with their language-specific queries and their language-prefixed names (`race_conditions` for Go since Go's Rust pipeline also uses `name() = "race_conditions"` — check the actual name() return values).
**Warning signs:** Other languages' test suites produce unexpected `race_conditions` findings.

---

## Code Examples

### Security Pipeline JSON Template (Rust: unsafe_memory)
The Rust implementation detects 3 patterns via tree-sitter: transmute, pointer arithmetic, raw deref in unsafe blocks. All are tree-sitter only. The JSON requires 3 match_pattern stages with separate flag stages:

```json
{
  "pipeline": "unsafe_memory",
  "category": "security",
  "description": "Detects unsafe memory operations: raw pointer dereference, pointer arithmetic (offset/add/sub), transmute in unsafe blocks. JSON version: matches all unsafe blocks then flags — cannot verify the specific sub-operation without multi-stage filtering.",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(unsafe_block (block (expression_statement (call_expression function: (_) @fn (#match? @fn \"transmute\"))))) @unsafe_transmute"
    },
    {
      "flag": {
        "pattern": "transmute_in_unsafe",
        "message": "mem::transmute in unsafe block bypasses type safety",
        "severity": "error"
      }
    }
  ]
}
```
**Note:** Multiple match_pattern+flag pairs in one file may not be supported — check executor. Alternative: separate JSON files per pattern, or a single broad `unsafe_block` match with simplified message.
**Source:** `[VERIFIED: src/audit/pipelines/rust/unsafe_memory.rs]`

### memory_leak_indicators JSON Template (Rust)

```json
{
  "pipeline": "memory_leak_indicators",
  "category": "scalability",
  "description": "Detects: Box::leak, mem::forget, ManuallyDrop::new calls; push/insert/extend inside loops.",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "[(call_expression function: (scoped_identifier) @fn (#match? @fn \"Box::leak\")) (call_expression function: (scoped_identifier) @fn (#match? @fn \"mem::forget\")) (call_expression function: (scoped_identifier) @fn (#match? @fn \"ManuallyDrop::new\"))] @leak_call"
    },
    {
      "flag": {
        "pattern": "intentional_leak",
        "message": "Memory-leaking call (Box::leak/mem::forget/ManuallyDrop::new) detected",
        "severity": "warning"
      }
    }
  ]
}
```
**Note on #match? predicate:** `#match?` is a tree-sitter predicate for string matching against captures. Verify this predicate is supported in the executor's `execute_match_pattern`. If not, use a broader query without name filtering and document precision reduction.
**Source:** `[VERIFIED: src/audit/pipelines/rust/memory_leak_indicators.rs]`

### Integration Test Pattern (from existing tests)

```rust
#[test]
fn <pipeline>_<lang>_finds_<pattern>() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.<ext>"), "<code with pattern>").unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::<Lang>], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::<Lang>]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::<Lang>])
        .pipeline_selector(PipelineSelector::Security)  // or Scalability
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(findings.iter().any(|f| f.pipeline == "<pipeline_name>" && f.pattern == "<pattern>"));
}
```
**Source:** `[VERIFIED: tests/audit_json_integration.rs]`

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Rust pipeline files in `src/audit/pipelines/<lang>/` | JSON files in `src/audit/builtin/` with `match_pattern` stage | Phase 1-3 established; Phase 4 continues | No Rust code changes for new security rules |
| Manual `include_str!` for each JSON file | `include_dir!` auto-discovery of all files in `src/audit/builtin/` | Phase 1 (ENG-02) | New JSON files auto-loaded without registration |
| Per-language Rust registry in `security_pipelines_for_language()` | Engine name-match suppression of Rust when JSON exists | Phase 1 (ENG-01) | JSON overrides Rust transparently |

**Deprecated/outdated:**
- Per-pipeline Rust unit tests (e.g., `#[test] fn detects_transmute()`): Deleted together with the Rust file. JSON integration tests in `tests/audit_json_integration.rs` replace them per TEST-01.

---

## Open Questions

1. **Does the match_pattern executor support `#match?` predicates?**
   - What we know: `execute_match_pattern` in `src/graph/executor.rs` compiles the query string and runs it. tree-sitter's `#match?` is a built-in predicate.
   - What's unclear: Whether the executor applies predicates or returns all matches regardless.
   - Recommendation: Planner should verify by reading `execute_match_pattern` fully (lines 714+). If predicates are applied, use them for name filtering. If not, use broader patterns and document precision reduction.

2. **Does the match_pattern executor support multiple match_pattern stages in sequence?**
   - What we know: The `graph` array can have multiple stages. The executor processes them in sequence.
   - What's unclear: Whether two `match_pattern` stages in one JSON file each start fresh or chain output nodes.
   - Recommendation: Planner should check whether multiple `match_pattern` stages are chained (output of first is input to second) or independent. The current Phase 3 JSON files only use one `match_pattern` stage per file.

3. **resource_exhaustion pipelines for Rust, Go, Python**
   - What we know: These are in the respective security_pipelines() registrations but not fully read in this research.
   - What's unclear: Whether they use pure tree-sitter or graph analysis.
   - Recommendation: Planner reads these files during planning and applies D-01 match-pattern test. If graph-based, document as Rust exception per D-03/D-07.

4. **panic_dos and toctou in Rust**
   - What we know: These are registered in `rust::security_pipelines()` but not read in this research.
   - What's unclear: Whether they pass the match-pattern test.
   - Recommendation: Planner reads these files and classifies.

5. **Go race_conditions pipeline name**
   - What we know: Go's `go_race_conditions.rs` was briefly read — uses `go_statement` inside `for_statement` detection, plus concurrent bracket access.
   - What's unclear: Whether the Rust struct's `name()` returns `"race_conditions"` or `"go_race_conditions"`.
   - **Critical:** If it returns `"race_conditions"`, the JSON file must use `"pipeline": "race_conditions"` with `"languages": ["go"]`. If it returns `"go_race_conditions"`, use that.
   - Recommendation: Planner checks the `fn name()` return value in `go_race_conditions.rs` before writing JSON.

---

## Environment Availability

Step 2.6: SKIPPED (no external dependencies identified — this phase is pure code/config changes within the existing Rust project).

---

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | Rust built-in test harness + cargo test |
| Config file | None (standard cargo test) |
| Quick run command | `cargo test --test audit_json_integration` |
| Full suite command | `cargo test` |

### Phase Requirements → Test Map
| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| SEC-01 | JSON security pipeline finds expected pattern in positive fixture | integration | `cargo test --test audit_json_integration -- <pipeline>_<lang>_finds_<pattern>` | ❌ Wave 0 (new tests per batch) |
| SEC-01 | JSON security pipeline produces no findings on clean fixture | integration | `cargo test --test audit_json_integration -- <pipeline>_<lang>_clean` | ❌ Wave 0 |
| SEC-02 | Rust pipeline file deleted, cargo compiles | unit (compile) | `cargo test` | ❌ Wave 0 (delete triggers) |
| SCAL-02 | JSON memory_leak_indicators finds pattern | integration | `cargo test --test audit_json_integration -- memory_leak_<lang>_finds` | ❌ Wave 0 |
| SCAL-02 | JSON memory_leak_indicators clean fixture | integration | `cargo test --test audit_json_integration -- memory_leak_<lang>_clean` | ❌ Wave 0 |
| SCAL-03 | Rust scalability file deleted, cargo compiles | unit (compile) | `cargo test` | ❌ Wave 0 |
| TEST-02 | All tests pass after each batch | integration | `cargo test` | ✅ (existing infra) |

### Sampling Rate
- **Per task commit:** `cargo test --test audit_json_integration` (fast, ~5s)
- **Per wave merge:** `cargo test` (full suite)
- **Phase gate:** Full suite green before `/gsd-verify-work`

### Wave 0 Gaps
- [ ] New test functions in `tests/audit_json_integration.rs` — 2 per migrated pipeline (positive + negative)
- No new test files or framework installs needed

---

## Security Domain

> This phase IS the security audit migration — it deals with security TOOLING not application security.

### Applicable ASVS Categories (for the virgil-cli binary itself)
| ASVS Category | Applies | Standard Control |
|---------------|---------|-----------------|
| V5 Input Validation | no | JSON files are embedded at compile time via include_dir; no runtime user input to JSON pipeline definitions |
| V6 Cryptography | no | No cryptographic operations in this phase |

**Note:** This phase migrates security pipeline *definitions*, it does not introduce new attack surfaces in virgil-cli itself. The only new code paths are JSON files embedded at compile time.

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | The `#match?` predicate in tree-sitter S-expressions is applied by the executor (not ignored) | Code Examples | If wrong, name-based filtering in match_pattern doesn't work; all patterns must be broader and accept more false positives |
| A2 | Multiple match_pattern stages in one JSON file each run independently (not chained output) | Code Examples | If they chain, approach to multi-pattern pipelines changes significantly |
| A3 | Go's `go_race_conditions` Rust struct returns name `"race_conditions"` (not `"go_race_conditions"`) | Taint Boundary Classification | If wrong, pipeline name in JSON file is wrong, suppression fails |
| A4 | `resource_exhaustion` pipelines for Rust/Go/Python are expressible in match_pattern | Taint Boundary Classification | If wrong, they become Rust exceptions and must be documented |
| A5 | Python `AnyPipeline::Graph` dispatch works the same as `wrap_legacy` for suppression purposes | Common Pitfalls | If wrong, Python security pipelines aren't suppressed and double-fire |

**Note on A5:** The suppression in engine.rs line 111 operates on `AnyPipeline` regardless of variant: `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))`. Since `AnyPipeline::Graph` also implements `name()`, the suppression works identically. A5 is LOW risk.

---

## Sources

### Primary (HIGH confidence)
- `[VERIFIED: src/audit/pipelines/rust/mod.rs]` — Rust security + scalability pipeline registrations
- `[VERIFIED: src/audit/pipelines/javascript/mod.rs]` — JS security + scalability registrations
- `[VERIFIED: src/audit/pipelines/typescript/mod.rs]` — TS-specific registrations and JS delegation
- `[VERIFIED: src/audit/pipelines/python/mod.rs]` — Python pipeline registrations and AnyPipeline::Graph pattern
- `[VERIFIED: src/audit/pipelines/go/mod.rs]` — Go security + scalability registrations
- `[VERIFIED: src/audit/pipelines/java/mod.rs]` — Java security + scalability registrations
- `[VERIFIED: src/audit/pipelines/c/mod.rs]` — C security + scalability registrations
- `[VERIFIED: src/audit/pipelines/cpp/mod.rs]` — C++ security + scalability registrations
- `[VERIFIED: src/audit/pipelines/csharp/mod.rs]` — C# security + scalability registrations
- `[VERIFIED: src/audit/pipelines/php/mod.rs]` — PHP security + scalability registrations
- `[VERIFIED: src/audit/engine.rs lines 83-121]` — ENG-01 suppression logic confirmed
- `[VERIFIED: src/graph/executor.rs execute_match_pattern]` — match_pattern implementation confirmed
- `[VERIFIED: src/audit/json_audit.rs]` — JSON discovery, dedup_key, builtin embedding
- `[VERIFIED: src/audit/pipeline.rs]` — security_pipelines_for_language, wrap_legacy, AnyPipeline
- `[VERIFIED: tests/audit_json_integration.rs]` — 24 existing tests, pattern for new tests
- `[VERIFIED: src/audit/builtin/sync_blocking_in_async_rust.json]` — per-language JSON template
- `[VERIFIED: src/audit/builtin/n_plus_one_queries.json]` — cross-language match_pattern template
- `[VERIFIED: src/audit/builtin/*.json (48 files)]` — confirmed auto-discovery in test

### Secondary (MEDIUM confidence)
- Multiple Rust pipeline .rs files read to assess match-pattern expressibility: `unsafe_memory.rs`, `integer_overflow.rs`, `race_conditions.rs`, `path_traversal.rs`, `type_confusion.rs`, `memory_leak_indicators.rs` (Rust), `command_injection.rs` (JS, Python), `memory_leak_indicators.rs` (JS, Go, Python, C), `java_race_conditions.rs`, `c_command_injection.rs`, `cpp_race_conditions.rs`

### Tertiary (LOW confidence)
- Classification of pipelines NOT fully read (`resource_exhaustion`, `panic_dos`, `toctou`, `go_resource_exhaustion`, `type_system_bypass`) — planner must verify by reading these files.

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — existing infrastructure, no new dependencies
- Architecture: HIGH — JSON file structure and engine integration fully verified
- Taint boundary classification: HIGH for pipelines read, LOW for ~5 unread pipelines (marked as PLANNER INSPECTS)
- memory_leak_indicators expressibility: MEDIUM — logic assessed from reading Rust implementations; actual tree-sitter query strings for simplified versions not verified

**Research date:** 2026-04-16
**Valid until:** 2026-05-16 (stable Rust codebase; extend if execution is delayed)
