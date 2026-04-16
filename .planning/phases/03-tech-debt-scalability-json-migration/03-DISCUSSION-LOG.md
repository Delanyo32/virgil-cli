# Phase 3: Tech Debt + Scalability JSON Migration - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-16
**Phase:** 03-tech-debt-scalability-json-migration
**Areas discussed:** TECH-02 scope, compute_metric JSON structure, Scalability pipeline fidelity, Test depth

---

## TECH-02 scope in Phase 3

| Option | Description | Selected |
|--------|-------------|----------|
| Defer to Phase 4+ | Phase 3 covers only TECH-01 + SCAL-01 = 6 shared pipelines. TECH-02 moves to Phase 4+. Keeps Phase 3 bounded without GraphPipeline blocker. | ✓ |
| Partial: match_pattern-only TECH-02 | Include TECH-02 pipelines expressible as match_pattern, exclude GraphPipeline ones. Requires upfront per-pipeline classification. | |
| Full TECH-02 as specified | All per-language tech-debt migrations in Phase 3. Some pipelines simplified, some stay Rust temporarily. | |

**User's choice:** Defer to Phase 4+ (Recommended)
**Notes:** Most TECH-02 pipelines are GraphPipeline candidates that can't be fully expressed in current JSON DSL. Phase 3 stays focused on the 6 shared pipelines.

---

## compute_metric JSON structure

| Option | Description | Selected |
|--------|-------------|----------|
| One cross-language JSON per pipeline | 4 JSON files total. compute_metric works on graph symbol nodes (language-aware). One cyclomatic_complexity.json for all 9 languages. | ✓ |
| Per-language JSON (4 × 9 = 36) | Matches Phase 1 architecture pattern. Language-calibrated thresholds per file. | |
| Hybrid: cross-language base + language overrides | One base + per-language override files for different thresholds. Adds engine lookup complexity. | |

**User's choice:** One cross-language JSON per pipeline (Recommended)
**Notes:** compute_metric's language-awareness comes from symbol nodes, not the JSON file itself.

---

### Follow-up: select scope before compute_metric

| Option | Description | Selected |
|--------|-------------|----------|
| Filter in JSON: select kind=function | JSON includes {select: symbol, kind: [function, method, arrow_function]} before compute_metric. Explicit, consistent with graph DSL patterns. | ✓ |
| compute_metric skips non-functions | Executor handles kind filtering internally. Less verbose JSON but hidden behavior. | |

**User's choice:** Filter in JSON (Recommended)
**Notes:** Keeps executor logic clean; follows established DSL patterns.

---

## Scalability pipeline fidelity

### n_plus_one_queries

| Option | Description | Selected |
|--------|-------------|----------|
| Faithful with match_pattern limits | JSON replaces Rust, accepts precision loss on DB-method-name filtering, documents the precision delta in the JSON file. | ✓ |
| Skeleton only — defer real migration | Minimal JSON detecting simplest N+1 pattern, schedule fuller migration for Phase 4. | |
| Keep Rust as permanent exception | n_plus_one and sync_blocking stay Rust like duplicate_code. Remove SCAL-01 from Phase 3. | |

**User's choice:** Faithful with match_pattern limits (Recommended)
**Notes:** Loop+call AST structure detection replaces the ~30 hardcoded DB method names. Precision delta documented in the JSON description field.

### sync_blocking_in_async

| Option | Description | Selected |
|--------|-------------|----------|
| Per-language JSON files | One JSON per language group with language-specific match_pattern query. Correct per-language logic at the cost of more files. | ✓ |
| Cross-language generic patterns only | One JSON matching broadest syntactic pattern. Higher false positive/negative rate, simpler. | |

**User's choice:** Per-language JSON files (Recommended)
**Notes:** The blocking-in-async pattern looks fundamentally different per language (std::fs in Rust vs time.sleep in Python vs async/await patterns in TS).

---

## Test depth

| Option | Description | Selected |
|--------|-------------|----------|
| Same minimum: 1 pos + 1 neg per pipeline | Consistent with Phase 1 baseline. Verifies pipeline fires and doesn't false-positive. | ✓ |
| Extend: add threshold boundary tests | For compute_metric pipelines, add a third case at the threshold boundary. 3 tests per compute_metric pipeline. | |

**User's choice:** Same minimum (Recommended)
**Notes:** Keep test velocity consistent with Phase 1. Threshold boundary testing deferred.

---

## Claude's Discretion

- Which language groups get sync_blocking_in_async JSON files (start with TS, Python, Rust, Go; C/C++/PHP/Java optional)
- Exact match_pattern S-expression queries for n_plus_one_queries and sync_blocking_in_async
- comment_to_code_ratio pipeline structure (may need file-level vs function-level treatment)

## Deferred Ideas

- TECH-02 per-language tech-debt migrations → Phase 4+
- SCAL-02 per-language scalability pipelines → Phase 4 (already in roadmap)
- Threshold boundary tests → considered and deferred
- match_pattern text predicates for DB method name filtering → future engine enhancement
