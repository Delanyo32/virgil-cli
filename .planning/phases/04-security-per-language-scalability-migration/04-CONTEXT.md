# Phase 4: Security + Per-Language Scalability Migration - Context

**Gathered:** 2026-04-16
**Status:** Ready for planning

<domain>
## Phase Boundary

Migrate all non-taint security pipelines and per-language scalability pipelines from Rust to declarative JSON definitions in `src/audit/builtin/`. Delete replaced Rust pipeline files. Add integration tests per batch.

**In scope:**
- Non-taint security pipelines for all 10 language groups (command injection, unsafe memory, integer overflow, path_traversal, insecure_deserialization, weak_cryptography, type_confusion, reflection_injection, race_conditions where pure tree-sitter)
- `memory_leak_indicators` scalability pipeline for all 10 languages

**Permanent Rust exceptions (not migrated):**
- `sql_injection` (all languages) — requires FlowsTo taint propagation
- `xss_dom_injection` (JavaScript/TypeScript) — requires taint flow to DOM
- `ssrf` / `java_ssrf` / `csharp_ssrf` / `ssrf_open_redirect` (multiple languages) — requires taint flow
- `xxe` (Java, C#) — requires taint flow through XML parser

**Requirements covered:** SEC-01, SEC-02, SCAL-02, SCAL-03, TEST-01, TEST-02

</domain>

<decisions>
## Implementation Decisions

### Taint Boundary Classification

- **D-01:** A pipeline migrates to JSON if and only if its Rust implementation uses **only tree-sitter pattern matching** — no FlowsTo/SanitizedBy graph edges, no multi-file tracking, no ResourceAnalyzer. This is the "match_pattern test."

- **D-02:** Pipelines that pass the match_pattern test and migrate: `path_traversal`, `insecure_deserialization`, `weak_cryptography`, `type_confusion`, `reflection_injection`, `code_injection`, `prototype_pollution`, `redos_resource_exhaustion`, `timing_weak_crypto`, `format_string`, `buffer_overflow` variants, `integer_overflow`, `command_injection`, `unsafe_memory`.

- **D-03:** `race_conditions` is handled **per-language**: planner inspects each language's Rust implementation. If it uses pure tree-sitter pattern matching (mutex/channel pattern detection) → migrate to JSON. If it requires graph-level concurrency analysis → leave in Rust as a documented exception with a comment in the corresponding language's `mod.rs` explaining why.

- **D-04:** All permanent Rust exceptions (taint-based pipelines) must be documented in the phase's plan with a clear "PERMANENT RUST EXCEPTION — requires FlowsTo/SanitizedBy graph predicates" comment.

### Security Pipeline Source of Truth

- **D-05:** No `audit_plans/` security specs exist. JSON patterns are derived **directly from the Rust implementations** — read the existing Rust file, translate the tree-sitter S-expression queries and hardcoded function/method name lists into the JSON `match_pattern` stage. No new audit_plans files are written.

- **D-06:** Strict parity is the baseline, but **obvious Rust bugs encountered during inspection should be fixed** in the JSON version. "Obvious bug" means: wrong function name in a hardcoded list, typo in a pattern string, missed language keyword variant clearly belonging to the pattern. Non-trivial logic changes are deferred. Document any intentional divergence from the Rust implementation in the JSON file's `"description"` field.

- **D-07:** If a Rust pipeline's detection logic cannot be faithfully expressed in `match_pattern` (e.g., requires multi-step stateful analysis), write a simplified pattern that catches the most common instances of the anti-pattern. Document the precision delta in the JSON `"description"` field (e.g., "simplified from Rust: detects direct calls only, not transitive flows").

### memory_leak_indicators Migration (SCAL-02)

- **D-08:** Produce **10 per-language JSON files** — one per language group — following the same naming convention established in Phase 3:
  - `memory_leak_indicators_rust.json`
  - `memory_leak_indicators_typescript.json`
  - `memory_leak_indicators_javascript.json`
  - `memory_leak_indicators_python.json`
  - `memory_leak_indicators_go.json`
  - `memory_leak_indicators_java.json`
  - `memory_leak_indicators_c.json`
  - `memory_leak_indicators_cpp.json`
  - `memory_leak_indicators_csharp.json`
  - `memory_leak_indicators_php.json`

- **D-09:** If a language's `memory_leak_indicators` Rust implementation uses flow analysis beyond simple tree-sitter patterns (e.g., complex resource tracking), write a **simplified match_pattern** that covers the most common leak indicators for that language. Do NOT skip the language. Accept precision loss; document it.

### Plan Organization

- **D-10:** Plans are organized **by language group** — each plan covers all security pipelines + memory_leak_indicators for one language. Approximately 10 plans total (one per language group).

- **D-11:** **Rust goes first** — it has no taint exceptions (no sql_injection, xss, ssrf), all security pipelines pass the match_pattern test straightforwardly, and its implementations serve as the canonical template for JSON security pipeline structure.

- **D-12:** Each plan follows the same atomic commit pattern established in Phase 3: write JSON files → run `cargo test` → delete Rust files → run `cargo test` again. Integration tests (1 positive + 1 negative per migrated pipeline) committed in the same batch.

### Claude's Discretion

- Exact ordering of language groups after Rust: planner chooses based on complexity (simpler language groups after Rust, more complex near end).
- Whether TypeScript and JavaScript are combined in one plan (shared base in `javascript/mod.rs`) or split into two plans.
- For `race_conditions` in each language: planner reads the Rust implementation and makes the per-language judgment call (migrate vs document as Rust exception) without returning to ask the user.

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

No external security audit_plans specs exist — JSON patterns are derived from Rust implementations.

### Security Pipeline Sources (Rust → JSON)
- `src/audit/pipelines/rust/mod.rs` — `security_pipelines()` function lists all Rust security pipelines for Rust language
- `src/audit/pipelines/c/mod.rs` — C security and scalability registrations
- `src/audit/pipelines/cpp/mod.rs` — C++ security and scalability registrations
- `src/audit/pipelines/csharp/mod.rs` — C# security and scalability registrations
- `src/audit/pipelines/go/mod.rs` — Go security and scalability registrations
- `src/audit/pipelines/java/mod.rs` — Java security and scalability registrations
- `src/audit/pipelines/javascript/mod.rs` — JavaScript security and scalability registrations
- `src/audit/pipelines/typescript/mod.rs` — TypeScript security registrations (delegates to JS + adds TS-specific)
- `src/audit/pipelines/php/mod.rs` — PHP security and scalability registrations
- `src/audit/pipelines/python/mod.rs` — Python security and scalability registrations

### JSON Template Reference (established patterns)
- `src/audit/builtin/sync_blocking_in_async_rust.json` — Template for per-language JSON file using `match_pattern` stage
- `src/audit/builtin/n_plus_one_queries.json` — Template for cross-language `match_pattern` pipeline
- `src/audit/builtin/cyclomatic_complexity.json` — Template for `compute_metric` pipeline (not relevant here but shows file structure)

### Engine Integration
- `src/audit/pipeline.rs` — `security_pipelines_for_language()` and `scalability_pipelines_for_language()` dispatch functions; these are updated per language as Rust pipelines are replaced
- `src/audit/engine.rs` — How the engine resolves JSON vs Rust pipeline precedence (name-match override established in Phase 1)

### Executor Stage Reference
- `src/graph/executor.rs` — `match_pattern` stage implementation (the only stage used by security JSON pipelines)

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `src/audit/builtin/sync_blocking_in_async_rust.json` — Per-language JSON security pipeline with `match_pattern` — exact structure to follow for security pipelines
- `src/audit/builtin/sync_blocking_in_async_typescript.json` — Per-language with `"languages": ["typescript", "tsx"]` multi-value filter
- `tests/audit_json_integration.rs` — Existing integration test scaffolding; add new positive/negative cases here

### Established Patterns
- Per-language JSON naming: `{pipeline_name}_{lang}.json` with `"languages": [...]` filter field
- Deletion is atomic: JSON added + Rust deleted in the same plan batch, verified by `cargo test`
- `wrap_legacy()` wrapper in `pipeline.rs` bridges old `Box<dyn Pipeline>` to `AnyPipeline` — used in language `security_pipelines_for_language()` dispatch; removed when Rust file is deleted
- Python uses `AnyPipeline::Graph(Box::new(...))` not `Box::new(...)` for some pipelines — pipeline dispatch in `python/mod.rs` differs from other languages

### Integration Points
- When a Rust file is deleted, its `pub mod {name};` line in the language's `mod.rs` is removed
- The JSON engine's name-match override (Phase 1, ENG-01) means JSON and Rust pipelines with the same name would double-fire unless the Rust file is deleted — hence atomic deletion in same batch
- `src/audit/pipeline.rs` `security_pipelines_for_language()` match arms remain unchanged; the JSON engine intercepts by name before the Rust pipeline runs

</code_context>

<specifics>
## Specific Ideas

- Rust language group goes first as the template-setting plan — all other languages follow its JSON structure
- JavaScript and TypeScript share 9 pipelines via `javascript::security_pipelines(language)` delegation; planner may combine them in one plan or treat as two (planner's call)
- `unsafe_type_assertions_security` is TypeScript-only (registered in typescript/mod.rs, not javascript) — handle separately from the shared JS/TS pipelines

</specifics>

<deferred>
## Deferred Ideas

- Writing `audit_plans/<lang>_security.md` specs before migration — decided against for velocity; specs can be back-filled in v2
- Taint-based security pipeline migration (sql_injection, xss, ssrf, xxe) — v2 TAINT-01 requirement; requires FlowsTo/SanitizedBy WhereClause predicates not yet in engine
- `resource_exhaustion` pipelines in Rust and Go security registrations — need planner inspection to determine if expressible in match_pattern; if not, document as Rust exception
- `memory_leak_indicators` precision improvements — the simplified match_pattern versions may have higher false-negative rates; future audit_plans security specs can address

</deferred>

---

*Phase: 04-security-per-language-scalability-migration*
*Context gathered: 2026-04-16*
