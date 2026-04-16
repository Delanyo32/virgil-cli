# virgil-cli — Audit Pipeline JSON Migration

## What This Is

virgil-cli is a Rust CLI tool that parses TypeScript, JavaScript, C, C++, C#, Rust, Python, Go, Java, and PHP codebases on-demand and queries them with a composable JSON query language and runs static analysis audits. The audit system currently has two coexisting implementations: hundreds of legacy Rust pipeline files and a newer JSON-driven engine. This milestone migrates all remaining Rust pipelines to the JSON-driven approach, removes the old code, and restores test health.

## Core Value

All audit pipelines run as declarative JSON definitions — no Rust code required to add, modify, or ship an audit rule.

## Requirements

### Validated

- ✓ On-demand parsing with tree-sitter across 12 languages — existing
- ✓ Audit engine with 4 categories (code-quality, security, scalability, architecture) — existing
- ✓ JSON audit engine (`json_audit.rs`) integrated with `AuditEngine` by pipeline name — existing
- ✓ 4 architecture pipelines migrated to JSON (`api_surface_area`, `circular_dependencies`, `dependency_depth`, `module_size_distribution`) — existing
- ✓ audit_plans/ specs written for all remaining pipelines (architecture + tech debt across all 12 languages) — existing
- ✓ S3 support, server mode, query language — existing (out of scope for this milestone)

### Active

- [ ] Convert all remaining Rust audit pipelines to JSON definitions in `src/audit/builtin/`
- [ ] Wire all new JSON pipelines into the engine by name (engine discovers JSON files automatically)
- [ ] Remove all old Rust pipeline `.rs` files after JSON replacements are validated
- [ ] Remove or consolidate legacy analyzer helpers (`src/audit/analyzers/`) no longer needed
- [ ] Delete per-pipeline Rust unit tests that test removed code
- [ ] Add integration-level tests verifying JSON pipelines fire correctly and produce expected findings
- [ ] `cargo test` passes with zero failures after all changes

### Out of Scope

- GraphPipeline / cross-file graph query pipelines — audit_plans mention these as future work; out of scope here
- New audit categories beyond what audit_plans/ already specifies
- Changes to query engine, language parsers, server mode, or S3 support
- Rewriting `json_audit.rs` engine internals — engine is already working

## Context

**Current state:** There are ~327 Rust files in `src/audit/` including ~300 pipeline files across all languages and categories. The `audit_plans/` directory contains detailed per-pipeline analysis documents (22 files covering architecture and tech debt for all 12 languages) that specify what each new JSON pipeline should detect, with threshold values and pattern names.

**JSON pipeline format:** Defined in `src/audit/builtin/*.json`. The engine (`src/audit/json_audit.rs`) loads these at startup via `include_dir!` macro and matches pipeline names against registered Rust pipelines — when a JSON file name matches a Rust pipeline name, the JSON version takes precedence.

**Test situation:** 2,205 `#[test]` functions exist across the audit pipeline files. These are unit tests for the Rust pipeline implementations. When Rust files are removed, their tests disappear with them. New JSON pipeline tests should be integration-style (pass a code snippet, assert findings are produced), added to `src/audit/json_audit.rs` or a new `tests/audit_json_integration.rs`.

**Migration precedent:** The 4 architecture JSON pipelines that already exist show the correct output format. Use them as templates.

**Languages to cover:** TypeScript/JS, C, C++, C#, Rust, Python, Go, Java, PHP (all 9 language groups have audit_plans/ specs).

## Constraints

- **Tech stack**: Rust — all pipeline definitions must be valid JSON that the existing `json_audit.rs` engine can parse
- **Compatibility**: Pipeline names must remain identical (they appear in CLI output, API responses, and `--pipeline` filter flags)
- **No regressions**: `cargo test` must pass after every phase; no partial states where Rust + JSON pipelines conflict
- **Specs first**: audit_plans/ documents are authoritative — JSON pipelines should reflect the improved detection logic described there, not just re-implement the Rust bugs

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| JSON-first audit engine | Decouples rule authoring from Rust compilation; enables external contributions | ✓ Good |
| Engine name-match override | JSON file with same name as Rust pipeline takes precedence — zero-config migration path | ✓ Good |
| Delete Rust unit tests with pipeline files | Rust tests test Rust implementation details, not pipeline behavior; JSON integration tests replace them | — Pending |
| Use audit_plans/ as specs | Detailed analysis already done; plans identify bugs in current Rust implementations to fix during migration | — Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd-transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `/gsd-complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-04-16 after initialization*
