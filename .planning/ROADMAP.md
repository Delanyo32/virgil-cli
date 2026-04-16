# Roadmap: virgil-cli — Audit Pipeline JSON Migration

## Overview

This milestone migrates all remaining Rust audit pipelines (~298 files) to declarative JSON definitions, removes the legacy Rust code, and restores test health. The work proceeds in five phases: fix engine bugs and complete architecture coverage first, then implement the two executor stages that unlock all AST-based migrations, then migrate tech debt and scalability pipelines in bulk, then tackle security and per-language scalability, and finally clean up dead code and verify test health. Each phase leaves the codebase in a fully working, testable state.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [ ] **Phase 1: Engine Fixes + Architecture JSON Expansion** - Fix doubled-findings and include_str! bugs; write per-language JSON architecture pipelines for all 9 languages; delete replaced Rust files; add integration test scaffolding
- [ ] **Phase 2: Executor Stage Implementation** - Implement match_pattern and compute_metric stages; stub remaining stages loudly; no pipeline migrations (pure engine work)
- [ ] **Phase 3: Tech Debt + Scalability JSON Migration** - Migrate shared cross-language complexity/tech-debt pipelines and shared scalability pipelines using the new stages; delete replaced Rust files
- [ ] **Phase 4: Security + Per-Language Scalability Migration** - Migrate non-taint security pipelines and per-language scalability pipelines; delete replaced Rust files
- [ ] **Phase 5: Final Cleanup + Test Health** - Remove dead helpers, empty the pipelines directory, verify zero test failures

## Phase Details

### Phase 1: Engine Fixes + Architecture JSON Expansion
**Goal**: The engine is safe to migrate against — doubled-findings suppression works, new JSON files load automatically, stub stages fail loudly — and all 9 language groups have complete JSON architecture pipeline coverage
**Depends on**: Nothing (first phase)
**Requirements**: ENG-01, ENG-02, ARCH-01, ARCH-02, ARCH-03, ARCH-04, ARCH-05, ARCH-06, ARCH-07, ARCH-08, ARCH-09, ARCH-10, TEST-01, TEST-02
**Success Criteria** (what must be TRUE):
  1. Running `virgil audit architecture` against a TypeScript, Python, Rust, Go, Java, C, C++, C#, and PHP project each returns findings — no language group is missing architecture results
  2. Adding a new `.json` file to `src/audit/builtin/` is automatically discovered by the engine without any change to `json_audit.rs` source code
  3. No Rust files remain in `src/audit/pipelines/architecture/` — the directory is empty or deleted
  4. `cargo test` passes with zero failures after all architecture Rust files are deleted
  5. A JSON pipeline and its former Rust pipeline running simultaneously produce a single set of findings, not doubled results
**Plans:** 5 plans

Plans:
- [x] 01-01-PLAN.md — Engine fixes: include_dir auto-discovery + doubled-findings suppression
- [x] 01-02-PLAN.md — JSON architecture pipelines for TypeScript/JS, Python, Rust, Go, Java (20 files)
- [x] 01-03-PLAN.md — JSON architecture pipelines for C, C++, C#, PHP (16 files)
- [x] 01-04-PLAN.md — Delete old JSON files + remove Rust architecture stubs
- [x] 01-05-PLAN.md — Integration tests (8 tests: 4 positive + 4 negative)

### Phase 2: Executor Stage Implementation
**Goal**: The JSON executor can run tree-sitter pattern matching and metric computation per file — `match_pattern` and `compute_metric` stages produce correct findings; all stub stages either work or fail loudly with a clear error
**Depends on**: Phase 1
**Requirements**: ENG-03, ENG-04, ENG-05
**Success Criteria** (what must be TRUE):
  1. A JSON pipeline using `match_pattern` with a valid tree-sitter S-expression query against a TypeScript file produces per-match findings with correct file and line information
  2. A JSON pipeline using `compute_metric` with `cyclomatic_complexity` produces non-zero findings for functions that exceed the threshold
  3. Executor stages `traverse`, `filter`, `match_name`, `count_edges`, and `pair` either perform their intended operation or return a descriptive error — none silently pass all nodes through unchanged
  4. `cargo test` passes with zero failures
**Plans:** 2 plans

Plans:
- [x] 02-01-PLAN.md — Create metrics module, update GraphStage enum (add MatchPattern/ComputeMetric, delete 5 stubs)
- [x] 02-02-PLAN.md — Implement match_pattern + compute_metric executor stages, update engine.rs call site, add tests

### Phase 3: Tech Debt + Scalability JSON Migration
**Goal**: All shared cross-language complexity pipelines and shared scalability pipelines run as JSON; corresponding Rust files are deleted; no regression in findings for cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio, n_plus_one_queries, or sync_blocking_in_async
**Depends on**: Phase 2
**Requirements**: TECH-01, TECH-02, TECH-03, SCAL-01, TEST-01, TEST-02
**Success Criteria** (what must be TRUE):
  1. `virgil audit code-quality complexity` returns cyclomatic complexity, function length, cognitive complexity, and comment ratio findings for all 9 supported language groups
  2. `virgil audit scalability` returns n_plus_one_queries and sync_blocking_in_async findings — shared JSON pipelines cover all languages
  3. No Rust pipeline files remain for tech debt complexity pipelines (`cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio`) or the shared scalability pipelines
  4. Each deleted pipeline batch has at least one positive-case integration test (code that should trigger a finding) and one negative-case test (clean code, no finding) in `tests/audit_json_integration.rs`
  5. `cargo test` passes with zero failures after all tech-debt Rust files are deleted
**Plans:** 4 plans

Plans:
- [x] 03-01-PLAN.md — Extend WhereClause with kind + 4 metric predicate fields (prerequisite)
- [x] 03-02-PLAN.md — Create 4 cross-language complexity JSON pipelines + delete 40 Rust files
- [ ] 03-03-PLAN.md — Create 5 scalability JSON pipelines + delete 15 Rust files
- [ ] 03-04-PLAN.md — Add 12 integration tests (6 pipelines x 2 tests)

### Phase 4: Security + Per-Language Scalability Migration
**Goal**: All non-taint security patterns and all per-language scalability pipelines run as JSON; corresponding Rust files are deleted; taint-based pipelines remain in Rust as documented permanent exceptions
**Depends on**: Phase 3
**Requirements**: SEC-01, SEC-02, SCAL-02, SCAL-03, TEST-01, TEST-02
**Success Criteria** (what must be TRUE):
  1. `virgil audit security` returns command injection, unsafe memory, and integer overflow findings via JSON pipelines for all applicable languages — taint-based findings (SQL injection, XSS, SSRF) continue via existing Rust pipelines
  2. `virgil audit scalability` returns per-language scalability findings for all 9 language groups — no language group is missing results compared to the pre-migration baseline
  3. No Rust pipeline files remain for non-taint security patterns or scalability pipelines that have JSON replacements
  4. Each deleted pipeline batch has integration tests (positive and negative cases) committed in the same batch
  5. `cargo test` passes with zero failures
**Plans**: TBD

### Phase 5: Final Cleanup + Test Health
**Goal**: The codebase has no dead audit code — `src/audit/analyzers/` and `src/audit/pipelines/` contain only files still in active use; `cargo test` passes with zero failures as the final verified state
**Depends on**: Phase 4
**Requirements**: CLEAN-01, CLEAN-02, CLEAN-03, TEST-02
**Success Criteria** (what must be TRUE):
  1. `src/audit/analyzers/` contains no helper modules that are unreferenced by any remaining pipeline — either the directory is removed or every file in it has at least one non-test caller
  2. `src/audit/pipelines/` is empty or deleted — no Rust pipeline files remain for any category that has been fully migrated to JSON
  3. `cargo test` passes with zero failures and no compiler warnings about unused imports or dead code in `src/audit/`
  4. `virgil audit` (all categories, all languages) produces non-empty output — no category silently regressed to zero findings during cleanup
**Plans**: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Engine Fixes + Architecture JSON Expansion | 5/5 | Complete | - |
| 2. Executor Stage Implementation | 2/2 | Complete | - |
| 3. Tech Debt + Scalability JSON Migration | 0/4 | Planned | - |
| 4. Security + Per-Language Scalability Migration | 0/TBD | Not started | - |
| 5. Final Cleanup + Test Health | 0/TBD | Not started | - |
