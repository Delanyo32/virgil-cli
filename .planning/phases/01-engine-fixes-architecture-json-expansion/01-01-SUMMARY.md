---
phase: 01-engine-fixes-architecture-json-expansion
plan: 01
subsystem: audit-engine
tags: [engine, include_dir, auto-discovery, deduplication, json-audit]
dependency_graph:
  requires: []
  provides: [ENG-01, ENG-02]
  affects: [src/audit/json_audit.rs, src/audit/engine.rs, Cargo.toml]
tech_stack:
  added: [include_dir = "0.7"]
  patterns: [include_dir! macro for compile-time directory embedding, retain() for pipeline deduplication]
key_files:
  created: []
  modified:
    - Cargo.toml
    - src/audit/json_audit.rs
    - src/audit/engine.rs
decisions:
  - "include_dir! macro used at module-level static to satisfy Dir<'static> lifetime requirement"
  - "builtin_audits() now iterates BUILTIN_AUDITS_DIR.files() — no source change needed to add JSON files"
  - "ENG-01 retain placed after match block, before pipeline_filter check — mirrors existing project_analyzers.retain() pattern"
metrics:
  duration: "2 minutes"
  completed_date: "2026-04-16"
  tasks_completed: 2
  files_modified: 3
---

# Phase 1 Plan 01: Engine Fixes (ENG-01 + ENG-02) Summary

**One-liner:** Auto-discovery of builtin JSON audit files via `include_dir!` macro embedding, plus `retain`-based doubled-findings suppression for Rust lang_pipelines overridden by JSON pipelines.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Add include_dir dependency and rewrite builtin_audits() for auto-discovery (ENG-02) | 171b7e8 | Cargo.toml, src/audit/json_audit.rs |
| 2 | Add doubled-findings suppression for lang_pipelines (ENG-01) | eb9dcba | src/audit/engine.rs |

## What Was Built

### ENG-02: Auto-discovery via include_dir!

The hardcoded `include_str!` array in `builtin_audits()` was replaced with an `include_dir!`-backed static. Previously, adding a new `.json` file to `src/audit/builtin/` required a manual source code edit in `json_audit.rs`. Now `BUILTIN_AUDITS_DIR.files()` iterates all `.json` files embedded at compile time — no source change needed.

Key implementation details:
- `static BUILTIN_AUDITS_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/src/audit/builtin");` at module level (required for `'static` lifetime)
- `builtin_audits()` filters for `.json` extension, handles UTF-8 errors and parse errors gracefully with `eprintln!` warnings
- Test `test_builtin_audits_returns_four` assertion changed from `== 4` to `>= 4` to accommodate future file additions

### ENG-01: Doubled-findings suppression for lang_pipelines

A `retain` call was added in `AuditEngine::run()` after the `match self.pipeline_selector` block (which assigns `lang_pipelines`) and before the `pipeline_filter` check. This mirrors the existing `project_analyzers.retain()` pattern at line 245:

```rust
// ENG-01: suppress Rust lang_pipelines that are overridden by a JSON pipeline
lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()));
```

Without this fix, running an audit with both a Rust pipeline and a same-named JSON pipeline would produce doubled findings — one set from each implementation.

## Verification Results

- `cargo test --lib -- json_audit`: 9/9 tests pass
- `cargo test --lib -- engine`: 15/15 tests pass
- `cargo build`: compiles cleanly (2 pre-existing warnings, not introduced by this plan)
- `cargo test --lib`: 2559/2559 tests pass

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None.

## Threat Flags

None. Changes are build-time embedding and internal deduplication only. No new trust boundaries introduced.

## Self-Check: PASSED

- [x] Cargo.toml contains `include_dir = "0.7"`: confirmed
- [x] src/audit/json_audit.rs contains `use include_dir::{include_dir, Dir}`: confirmed
- [x] src/audit/json_audit.rs contains `static BUILTIN_AUDITS_DIR: Dir<'static>`: confirmed
- [x] src/audit/json_audit.rs contains `include_dir!("$CARGO_MANIFEST_DIR/src/audit/builtin")`: confirmed
- [x] builtin_audits() uses `BUILTIN_AUDITS_DIR.files()` not `include_str!`: confirmed
- [x] src/audit/engine.rs contains `lang_pipelines.retain(|p| !json_pipeline_names.contains(&p.name().to_string()))`: confirmed
- [x] engine.rs retain appears after match block and before pipeline_filter check: confirmed
- [x] Commit 171b7e8 exists: confirmed
- [x] Commit eb9dcba exists: confirmed
