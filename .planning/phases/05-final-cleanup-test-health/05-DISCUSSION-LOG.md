# Phase 5: Final Cleanup + Test Health - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-16
**Phase:** 05-final-cleanup-test-health
**Areas discussed:** TECH-02 fate, helpers.rs disposition, Test health strategy, analyzers/ cleanup scope

---

## TECH-02 Fate

| Option | Description | Selected |
|--------|-------------|----------|
| Keep as permanent Rust | Accept 139 non-taint pipelines as permanent Rust implementations; Phase 5 is cleanup only | |
| Migrate all to JSON now | Convert all 139 pipelines to JSON; true JSON-first goal; Phase 5 becomes TECH-02 + cleanup | ✓ |
| Delete without replacing | Delete Rust pipelines without JSON replacements; lose those audit checks permanently | |

**User's choice:** Migrate all to JSON now

---

**Follow-up: Complex pipelines (can't express in match_pattern)?**

| Option | Description | Selected |
|--------|-------------|----------|
| Simplify + document delta | Write simplified match_pattern, document precision loss in JSON description field | ✓ |
| Keep complex ones as Rust | Leave graph-based pipelines (coupling, duplicate_code) as permanent Rust exceptions | |

**User's choice:** Simplify + document delta (same precedent as Phase 3/4)

---

**Follow-up: Source of truth for JSON patterns?**

| Option | Description | Selected |
|--------|-------------|----------|
| audit_plans/ specs first | Treat audit_plans/ as authoritative; fix Rust bugs in JSON version | ✓ |
| Rust implementations first | Read Rust files directly; translate patterns; inherits existing bugs | |

**User's choice:** audit_plans/ specs first

---

**Follow-up: Plan organization?**

| Option | Description | Selected |
|--------|-------------|----------|
| Per language group | One plan per language group; ~9 plans + 1 cleanup | ✓ |
| Per pipeline category | One plan per pipeline type across all languages | |
| Batched by complexity | Simple pipelines first, complex last | |

**User's choice:** Per language group (same as Phase 4)

---

## helpers.rs Disposition

| Option | Description | Selected |
|--------|-------------|----------|
| Keep in place | Leave helpers.rs at pipelines/helpers.rs; prune unused functions | ✓ |
| Move needed functions | Relocate ~5 needed functions to their caller modules; delete helpers.rs | |
| Rename to audit_helpers.rs | Move to audit/helpers.rs once pipelines/ empties | |

**User's choice:** Keep in place

---

**Follow-up: Prune unused helpers?**

| Option | Description | Selected |
|--------|-------------|----------|
| Yes — delete unreferenced helpers | Dead-code pass after pipeline deletions; satisfies CLEAN-03 | ✓ |
| No — leave as-is | Keep all functions even if pipeline callers are gone | |

**User's choice:** Yes — delete unreferenced helpers

---

## Test Health Strategy

| Option | Description | Selected |
|--------|-------------|----------|
| 1 positive + 1 negative per pipeline | Minimum from Phases 1-4; ~278 new integration tests | |
| Positive only | Faster; skip negative cases | |
| Match Rust test depth | Replicate unit test count as integration tests per pipeline | ✓ |

**User's choice:** Match Rust test depth — count `#[test]` functions per Rust file and create that many integration tests

---

**Follow-up: Test count target?**

| Option | Description | Selected |
|--------|-------------|----------|
| Whatever remains + new JSON tests | No fixed target; zero failures + meaningful coverage per pipeline | ✓ |
| Match current total (2,142+) | Add enough JSON tests to avoid significant count drop | |

**User's choice:** Whatever remains + new JSON tests

---

## analyzers/ Cleanup Scope

| Option | Description | Selected |
|--------|-------------|----------|
| Keep all three — they're active | coupling, dead_exports, duplicate_symbols all stay | ✓ |
| Keep coupling, check the others | Verify dead_exports and duplicate_symbols callers before deciding | |
| Remove all — convert to JSON | Requires engine graph support; out of scope | |

**User's choice:** Keep all three — they're active ProjectAnalyzers

---

**Follow-up: Add PERMANENT RUST EXCEPTION comments to taint files?**

| Option | Description | Selected |
|--------|-------------|----------|
| Yes — add explicit comment | Each of the 14 taint files gets a comment block at the top | ✓ |
| No — skip documentation | Exception already in CONTEXT.md and PROJECT.md | |

**User's choice:** Yes — add explicit PERMANENT RUST EXCEPTION comment

---

## Claude's Discretion

- Exact ordering of language groups across migration plans
- Whether JavaScript/TypeScript share one plan or split
- Which helpers.rs functions lose all callers (runtime dead-code audit)
- Exact simplified match_pattern approach for duplicate_code

## Deferred Ideas

None — discussion stayed within phase scope.
