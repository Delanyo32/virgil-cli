---
phase: 01-engine-fixes-architecture-json-expansion
verified: 2026-04-16T10:30:00Z
status: passed
score: 14/14
overrides_applied: 0
---

# Phase 1: Engine Fixes + Architecture JSON Expansion — Verification Report

**Phase Goal:** Migrate all architecture audit pipelines to JSON-driven approach — fix engine bugs (ENG-01, ENG-02) that blocked migration, create 36 per-language JSON pipeline files for all 9 language groups, remove all legacy Rust architecture dispatch code, and establish integration test coverage.
**Verified:** 2026-04-16T10:30:00Z
**Status:** PASSED
**Re-verification:** No — initial verification

---

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | Running `virgil audit architecture` against a TypeScript, Python, Rust, Go, Java, C, C++, C#, and PHP project each returns findings — no language group is missing architecture results | VERIFIED | All 9 language groups have exactly 4 per-language JSON pipeline files in `src/audit/builtin/` (36 total). Each file contains a `languages` filter field ensuring per-language routing. All 36 files parse successfully via `builtin_audits()`. |
| 2 | Adding a new `.json` file to `src/audit/builtin/` is automatically discovered by the engine without any change to `json_audit.rs` source code | VERIFIED | `builtin_audits()` uses `BUILTIN_AUDITS_DIR.files()` (include_dir! macro) at lines 35-51 of `src/audit/json_audit.rs`. No `include_str!` remains. Test asserts `>= 36` allowing future additions. |
| 3 | No Rust files remain that implement architecture_pipelines() — the directory/stubs are empty or deleted | VERIFIED | `grep -rn "fn architecture_pipelines" src/audit/pipelines/` returns empty. `grep "fn architecture_pipelines_for_language" src/audit/pipeline.rs` returns empty. `src/audit/pipelines/architecture/` never existed (architecture was in language mod.rs stubs, now all removed). |
| 4 | `cargo test` passes with zero failures after all architecture Rust files are deleted | VERIFIED | `cargo test --lib`: 2559 passed, 0 failed. `cargo test --test audit_json_integration`: 8 passed, 0 failed. `cargo build`: exits 0 (1 pre-existing warning unrelated to phase work). |
| 5 | A JSON pipeline and its former Rust pipeline running simultaneously produce a single set of findings, not doubled results | VERIFIED | `engine.rs` line 110-111: `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))` suppresses Rust lang_pipelines overridden by JSON pipelines. ENG-01 comment present. Placement verified: after match block (line 108), before pipeline_filter check (line 113). |

**Score:** 5/5 roadmap success criteria verified

---

### Requirement-Level Truths (from PLAN frontmatter)

| # | Must-Have Truth | Status | Evidence |
|---|-----------------|--------|----------|
| 1 | Adding a new .json file to src/audit/builtin/ is automatically discovered by the engine without any change to json_audit.rs source code | VERIFIED | ENG-02: `include_dir!` macro + `BUILTIN_AUDITS_DIR.files()` in `builtin_audits()` |
| 2 | A JSON pipeline and its former Rust pipeline running simultaneously produce a single set of findings, not doubled results | VERIFIED | ENG-01: `retain` at engine.rs:111 |
| 3 | Running virgil audit architecture against a TypeScript project returns architecture findings | VERIFIED | 4 `*_javascript.json` files with `languages: [typescript, javascript, tsx, jsx]` |
| 4 | Running virgil audit architecture against a Python project returns architecture findings | VERIFIED | 4 `*_python.json` files with `languages: [python]` |
| 5 | Running virgil audit architecture against a Rust project returns architecture findings | VERIFIED | 4 `*_rust.json` files with `languages: [rust]` |
| 6 | Running virgil audit architecture against a Go project returns architecture findings | VERIFIED | 4 `*_go.json` files with `languages: [go]` |
| 7 | Running virgil audit architecture against a Java project returns architecture findings | VERIFIED | 4 `*_java.json` files with `languages: [java]` |
| 8 | Running virgil audit architecture against a C project returns architecture findings | VERIFIED | 4 `*_c.json` files with `languages: [c]` |
| 9 | Running virgil audit architecture against a C++ project returns architecture findings | VERIFIED | 4 `*_cpp.json` files with `languages: [cpp]` |
| 10 | Running virgil audit architecture against a C# project returns architecture findings | VERIFIED | 4 `*_csharp.json` files with `languages: [csharp]` |
| 11 | Running virgil audit architecture against a PHP project returns architecture findings | VERIFIED | 4 `*_php.json` files with `languages: [php]` |
| 12 | No Rust files remain that implement architecture_pipelines() — all stubs are deleted | VERIFIED | `grep -rn "fn architecture_pipelines"` in `src/audit/pipelines/` returns empty |
| 13 | No language-agnostic JSON architecture files remain in src/audit/builtin/ | VERIFIED | `module_size_distribution.json`, `api_surface_area.json`, `circular_dependencies.json`, `dependency_depth.json` — all 4 deleted; confirmed `ls` returns "No such file" |
| 14 | Each architecture pipeline type has at least one positive integration test and one negative integration test | VERIFIED | 8 tests in `tests/audit_json_integration.rs`: 4 positive + 4 negative covering all 4 pipeline types |

**Score:** 14/14 must-have truths verified

---

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `Cargo.toml` | include_dir dependency | VERIFIED | `include_dir = "0.7"` present |
| `src/audit/json_audit.rs` | Auto-discovery via BUILTIN_AUDITS_DIR | VERIFIED | `static BUILTIN_AUDITS_DIR: Dir<'static> = include_dir!(...)` at module level; `builtin_audits()` uses `.files()` not `include_str!` |
| `src/audit/engine.rs` | ENG-01 retain + Architecture arm inline vec![] | VERIFIED | `lang_pipelines.retain(|p| !json_pipeline_names.contains(...))` at line 111; `PipelineSelector::Architecture => { vec![] }` at lines 104-107 |
| `src/audit/pipeline.rs` | No architecture_pipelines_for_language | VERIFIED | Function deleted; no references remain |
| All 36 per-language JSON files | Valid JSON with pipeline/category/languages fields | VERIFIED | All 36 parse cleanly; all have `category: "architecture"` and `languages` filter |
| `tests/audit_json_integration.rs` | 8 integration tests | VERIFIED | 8 tests present; all 8 pass |
| 4 old language-agnostic JSON files | Deleted | VERIFIED | `module_size_distribution.json`, `api_surface_area.json`, `circular_dependencies.json`, `dependency_depth.json` — all absent |

---

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| `src/audit/json_audit.rs` | `src/audit/builtin/` | `include_dir!` macro embedding | VERIFIED | `include_dir!("$CARGO_MANIFEST_DIR/src/audit/builtin")` at line 32 |
| `src/audit/engine.rs` | `src/audit/json_audit.rs` | `json_pipeline_names.contains` in retain | VERIFIED | Line 111: `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))` |
| `src/audit/engine.rs` | architecture pipelines (JSON) | Architecture arm returns `vec![]`, JSON loop handles all | VERIFIED | Lines 104-107: Architecture arm returns empty vec; JSON audit loop at lines 262-270 handles all architecture pipelines via `languages` field filter |
| `tests/audit_json_integration.rs` | `src/audit/engine.rs` | `AuditEngine::new().pipeline_selector(PipelineSelector::Architecture).run()` | VERIFIED | 8 tests exercise full path; all pass |

---

### Data-Flow Trace (Level 4)

Not applicable — this phase delivers JSON configuration files and Rust engine logic, not UI components or data rendering layers.

---

### Behavioral Spot-Checks

| Behavior | Command | Result | Status |
|----------|---------|--------|--------|
| Integration tests pass (all 8) | `cargo test --test audit_json_integration` | 8 passed, 0 failed | PASS |
| Lib test suite passes | `cargo test --lib` | 2559 passed, 0 failed | PASS |
| Binary compiles clean | `cargo build` | Finished (1 pre-existing warning) | PASS |
| All 36 JSON files are valid and parseable | Python json.load on all files | 0 errors | PASS |
| Language-calibrated depth thresholds correct | Python threshold extraction | rust:4, go:5, js:6, c:4, cpp:4, csharp:6, php:6 — all correct | PASS |
| PHP api_surface_area raised to gte:15 | Python threshold extraction | count.gte = 15 | PASS |
| JS pipeline covers all 4 TS/JS dialects | File inspection | `languages: [typescript, javascript, tsx, jsx]` | PASS |

---

### Requirements Coverage

| Requirement | Source Plan | Description | Status | Evidence |
|-------------|-------------|-------------|--------|----------|
| ENG-01 | 01-01 | Doubled-findings suppression for lang_pipelines | SATISFIED | `engine.rs:111` retain call suppresses Rust pipelines overridden by JSON |
| ENG-02 | 01-01 | Auto-discovery of builtin JSON files via include_dir | SATISFIED | `builtin_audits()` uses `BUILTIN_AUDITS_DIR.files()` |
| ARCH-01 | 01-02 | TypeScript/JavaScript architecture pipelines as JSON | SATISFIED | 4 `*_javascript.json` files with `languages: [typescript, javascript, tsx, jsx]` |
| ARCH-02 | 01-02 | Python architecture pipelines as JSON | SATISFIED | 4 `*_python.json` files |
| ARCH-03 | 01-02 | Rust architecture pipelines as JSON | SATISFIED | 4 `*_rust.json` files |
| ARCH-04 | 01-02 | Go architecture pipelines as JSON | SATISFIED | 4 `*_go.json` files |
| ARCH-05 | 01-02 | Java architecture pipelines as JSON | SATISFIED | 4 `*_java.json` files |
| ARCH-06 | 01-03 | C architecture pipelines as JSON | SATISFIED | 4 `*_c.json` files |
| ARCH-07 | 01-03 | C++ architecture pipelines as JSON | SATISFIED | 4 `*_cpp.json` files |
| ARCH-08 | 01-03 | C# architecture pipelines as JSON | SATISFIED | 4 `*_csharp.json` files |
| ARCH-09 | 01-03 | PHP architecture pipelines as JSON | SATISFIED | 4 `*_php.json` files (with raised api_surface_area threshold) |
| ARCH-10 | 01-04 | All replaced Rust architecture pipeline files deleted | SATISFIED | 4 old JSON files deleted; 10 `architecture_pipelines()` stubs removed; `architecture_pipelines_for_language` and `supported_architecture_languages` deleted from `pipeline.rs` |
| TEST-01 | 01-05 | Each pipeline deletion batch has positive + negative integration tests | SATISFIED | 8 tests in `tests/audit_json_integration.rs` covering all 4 pipeline types; all pass |
| TEST-02 | 01-04, 01-05 | `cargo test` passes with zero failures at every phase boundary | SATISFIED | `cargo test --lib`: 2559 passed; `cargo test --test audit_json_integration`: 8 passed |

---

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| `src/audit/engine.rs` | (pre-existing) | `unused variable: source` warning | Info | Pre-existing, not introduced by this phase; no impact on functionality |

No blockers found. No stubs or placeholder code introduced.

---

### Human Verification Required

None. All success criteria are verifiable programmatically:
- JSON file existence and structure: verified via filesystem + json.load
- Rust code removal: verified via grep returning empty
- Test passage: verified via cargo test
- Compilation: verified via cargo build

---

### Gaps Summary

No gaps found. All 14 must-have truths verified, all 14 phase requirements satisfied, all behavioral spot-checks pass.

**Notable deviation from plan (accepted, auto-fixed):** Plan 05 specified using Go for the `dependency_graph_depth` positive integration test. The implementation used TypeScript instead because Go's `resolve_import` does not handle `./b` style relative imports (it expects full module paths like `github.com/foo/bar`). The TypeScript pipeline has the same `gte:6` threshold. This deviation was correctly anticipated in the plan as an acceptable alternative.

**Notable deviation from plan (accepted, auto-fixed):** Plan 04 did not account for callers of `supported_architecture_languages()` in `main.rs` and `server.rs`. The implementation correctly replaced these with `Language::all().to_vec()` — architecturally equivalent since JSON pipelines carry their own language filters.

---

_Verified: 2026-04-16T10:30:00Z_
_Verifier: Claude (gsd-verifier)_
