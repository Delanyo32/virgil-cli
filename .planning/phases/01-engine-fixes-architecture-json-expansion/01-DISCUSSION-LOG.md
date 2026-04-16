# Phase 1: Engine Fixes + Architecture JSON Expansion - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-16
**Phase:** 01-engine-fixes-architecture-json-expansion
**Areas discussed:** Per-language JSON file structure, Spec depth for Phase 1, ENG-02 auto-discovery mechanism, Integration test scope

---

## Per-language JSON file structure

| Option | Description | Selected |
|--------|-------------|----------|
| 36 new files, one per language | 4 JSON files per language group each with "languages" filter. 40 total files in builtin/. Clean separation, per-language tuning. | ✓ |
| Modify existing 4 files only | Add "languages" array to existing 4 JSON files. Simpler but no per-language threshold tuning. | |
| Keep 4 generic + add language-specific delta files | Leave existing 4 unchanged, add delta files for improvements only. Fewer new files but double-coverage complexity. | |

**User's choice:** 36 new files, one per language

---

### Follow-up: Fate of existing 4 language-agnostic files

| Option | Description | Selected |
|--------|-------------|----------|
| Delete them — per-language files replace them | Once all 9 language groups have specific files, delete the 4 generic files. Clean final state: 36 files only. | ✓ |
| Keep them as fallbacks | Leave generic files in place for unsupported languages. Since engine filters unsupported languages, these would never run anyway. | |

**User's choice:** Delete them

---

## Spec depth for Phase 1

| Option | Description | Selected |
|--------|-------------|----------|
| Graph-stage improvements only | Implement all improvements the existing DSL supports (thresholds, barrel exclusions, filters). Skip match_pattern-dependent improvements. | ✓ |
| Language filters only — defer improvements | Just add "languages" filter to language copies of the 4 existing pipelines. No audit_plans/ improvements yet. | |
| Full spec fidelity where possible | Implement everything audit_plans/ specifies that can be expressed without match_pattern. | |

**User's choice:** Graph-stage improvements only

---

### Follow-up: Thresholds across languages

| Option | Description | Selected |
|--------|-------------|----------|
| Language-calibrated thresholds | Each language's JSON files use thresholds that fit language idioms. | ✓ |
| Shared thresholds across all languages | All 36 files use same numeric thresholds as current 4 generic pipelines. | |

**User's choice:** Language-calibrated thresholds

---

## ENG-02: Auto-discovery mechanism

| Option | Description | Selected |
|--------|-------------|----------|
| include_dir! macro | Add "include_dir" crate. Embeds entire builtin/ directory at compile time. New .json files auto-picked up. | ✓ |
| build.rs codegen | build.rs scans builtin/ and generates include_str! list. No new dep, but adds build script complexity. | |
| Runtime filesystem scan | Scan at startup. Breaks single-binary distribution. | |

**User's choice:** include_dir! macro

---

## Integration test scope

| Option | Description | Selected |
|--------|-------------|----------|
| One representative language per pipeline | 4 pipelines × 1 language = 8 tests. Fast to write, proves mechanism works. | ✓ |
| Per-pipeline-total using multi-language snippets | 8 tests using multi-language code. Harder to write realistic triggers. | |
| Per-language per-pipeline | 72 tests. Most thorough but high maintenance for Phase 1 scaffolding. | |

**User's choice:** One representative language per pipeline

---

### Follow-up: Test file location

| Option | Description | Selected |
|--------|-------------|----------|
| tests/audit_json_integration.rs | Separate integration test file using full AuditEngine API end-to-end. | ✓ |
| Inline in src/audit/json_audit.rs | Add to existing #[cfg(test)] module. Unit tests not integration tests. | |

**User's choice:** tests/audit_json_integration.rs

---

## Claude's Discretion

- ENG-01 fix approach: Add `lang_pipelines.retain(|p| !json_pipeline_names.contains(...))` in engine.rs — same pattern as existing project_analyzer suppression
- ARCH-10 scope: Remove empty `architecture_pipelines()` stubs from all language mod.rs files; remove `architecture_pipelines_for_language()` dispatch in pipeline.rs

## Deferred Ideas

None
