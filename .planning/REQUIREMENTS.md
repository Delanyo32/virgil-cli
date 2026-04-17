# Requirements: virgil-cli Audit Pipeline JSON Migration

**Defined:** 2026-04-16
**Core Value:** All audit pipelines run as declarative JSON definitions — no Rust code required to add, modify, or ship an audit rule.

## v1 Requirements

### Engine Fixes

- [ ] **ENG-01**: Doubled-findings bug fixed — engine suppression extended so a Rust pipeline + JSON pipeline with the same name does not produce duplicate results
- [ ] **ENG-02**: JSON pipeline registration no longer requires manual `include_str!` addition — new JSON files in `src/audit/builtin/` are discovered automatically
- [ ] **ENG-03**: `match_pattern` stage implemented in executor — accepts a tree-sitter S-expression query string, runs it per-file, emits matching nodes as findings
- [ ] **ENG-04**: `compute_metric` stage implemented in executor — wires existing `helpers.rs` functions (cyclomatic complexity, function length, cognitive complexity, comment ratio) into the stage dispatch
- [ ] **ENG-05**: Stub executor stages implemented: `traverse`, `filter`, `match_name`, `count_edges`, `pair` — each stage performs its intended operation rather than passing nodes through unchanged

### Architecture Pipelines (per language)

- [ ] **ARCH-01**: TypeScript/JavaScript architecture pipelines converted to JSON (module_size_distribution, circular_dependencies, dependency_depth, api_surface_area variants with language filter)
- [ ] **ARCH-02**: Python architecture pipelines converted to JSON
- [ ] **ARCH-03**: Rust architecture pipelines converted to JSON
- [ ] **ARCH-04**: Go architecture pipelines converted to JSON
- [ ] **ARCH-05**: Java architecture pipelines converted to JSON
- [ ] **ARCH-06**: C architecture pipelines converted to JSON
- [ ] **ARCH-07**: C++ architecture pipelines converted to JSON
- [ ] **ARCH-08**: C# architecture pipelines converted to JSON
- [ ] **ARCH-09**: PHP architecture pipelines converted to JSON
- [ ] **ARCH-10**: All replaced Rust architecture pipeline files deleted — no Rust architecture pipelines remain

### Tech Debt Pipelines

- [ ] **TECH-01**: Cross-language shared tech debt pipelines migrated to JSON: `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio` (requires ENG-03 + ENG-04)
- [ ] **TECH-02**: Per-language tech debt pipelines migrated to JSON for all 9 language groups (using match_pattern stage per audit_plans/ specs)
- [ ] **TECH-03**: All replaced Rust tech debt pipeline files deleted

### Security Pipelines

- [ ] **SEC-01**: Per-language non-taint security pipelines migrated to JSON (command injection patterns, unsafe memory patterns, integer overflow patterns — those expressible via match_pattern)
- [ ] **SEC-02**: All replaced Rust security pipeline files deleted

### Scalability Pipelines

- [ ] **SCAL-01**: Cross-language shared scalability pipelines migrated to JSON: `n_plus_one_queries`, `sync_blocking_in_async` (requires ENG-03)
- [ ] **SCAL-02**: Per-language scalability pipelines migrated to JSON for all applicable languages
- [ ] **SCAL-03**: All replaced Rust scalability pipeline files deleted

### Test Coverage

- [ ] **TEST-01**: Each pipeline deletion batch has corresponding JSON integration tests added in the same phase — minimum one positive case (finds expected pattern) and one negative case (clean code, no finding) per pipeline
- [x] **TEST-02**: `cargo test` passes with zero failures at every phase boundary — no intermediate broken state committed

### Cleanup

- [x] **CLEAN-01**: `src/audit/analyzers/` helpers removed if no longer referenced by any remaining pipeline
- [x] **CLEAN-02**: `src/audit/pipelines/` directory empty or removed after full migration
- [x] **CLEAN-03**: Dead imports and unused helper functions in `src/audit/` cleaned up after all deletions

## v2 Requirements

### Taint-based security

- **TAINT-01**: Taint-propagation security pipelines (SQL injection via FlowsTo paths, XSS, SSRF) migrated to JSON — requires adding `is_taint_source`, `is_taint_sink`, `has_unsanitized_path` predicates to WhereClause

### Complex analysis

- **COMPLEX-01**: `duplicate_code` pipeline migrated to JSON — requires rolling-hash similarity algorithm expressible in JSON stages (high complexity, likely stays Rust)
- **COMPLEX-02**: `memory_leak_indicators` pipeline migrated to JSON — requires resource lifecycle tracking beyond current ResourceAnalyzer

## Out of Scope

| Feature | Reason |
|---------|--------|
| Taint-analysis security pipelines (v1) | Require `FlowsTo`/`SanitizedBy` graph predicates not in current engine — deferred to v2 |
| `duplicate_code` migration | Rolling-hash similarity has no JSON equivalent; intentionally stays Rust |
| New audit categories beyond audit_plans/ specs | Not in scope for this migration milestone |
| Changes to query engine, language parsers, server, S3 | Orthogonal to audit migration; zero-risk isolation required |
| CFG-dependent pipelines (recursive depth analysis, callback hell) | Require AST accumulator patterns inexpressible in linear JSON stages |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| ENG-01 | Phase 1 | Pending |
| ENG-02 | Phase 1 | Pending |
| ENG-03 | Phase 2 | Pending |
| ENG-04 | Phase 2 | Pending |
| ENG-05 | Phase 2 | Pending |
| ARCH-01 | Phase 1 | Pending |
| ARCH-02 | Phase 1 | Pending |
| ARCH-03 | Phase 1 | Pending |
| ARCH-04 | Phase 1 | Pending |
| ARCH-05 | Phase 1 | Pending |
| ARCH-06 | Phase 1 | Pending |
| ARCH-07 | Phase 1 | Pending |
| ARCH-08 | Phase 1 | Pending |
| ARCH-09 | Phase 1 | Pending |
| ARCH-10 | Phase 1 | Pending |
| TECH-01 | Phase 3 | Pending |
| TECH-02 | Phase 3 | Pending |
| TECH-03 | Phase 3 | Pending |
| SEC-01 | Phase 4 | Pending |
| SEC-02 | Phase 4 | Pending |
| SCAL-01 | Phase 3 | Pending |
| SCAL-02 | Phase 4 | Pending |
| SCAL-03 | Phase 4 | Pending |
| TEST-01 | Phase 1-4 | Pending |
| TEST-02 | Phase 1-4 | Complete |
| CLEAN-01 | Phase 5 | Complete |
| CLEAN-02 | Phase 5 | Complete |
| CLEAN-03 | Phase 5 | Complete |

**Coverage:**
- v1 requirements: 28 total
- Mapped to phases: 28
- Unmapped: 0 ✓

---
*Requirements defined: 2026-04-16*
*Last updated: 2026-04-16 after roadmap creation — traceability confirmed, all 28 v1 requirements mapped*
