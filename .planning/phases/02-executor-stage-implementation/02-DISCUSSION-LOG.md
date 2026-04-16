# Phase 2: Executor Stage Implementation - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-16
**Phase:** 02-executor-stage-implementation
**Areas discussed:** match_pattern source access, compute_metric graph dependency, stub stage behavior, test strategy

---

## match_pattern source access

| Option | Description | Selected |
|--------|-------------|----------|
| Extend executor API | Add optional `&Workspace` to `run_pipeline()`, match_pattern parses files on demand | ✓ |
| Store source in CodeGraph | Cache file source text in graph during construction | |
| Separate per-file code path | Detect match_pattern in JSON pipeline, run in per-file loop | |

**User's choice:** Extend executor API — add optional `Option<&Workspace>` parameter to `run_pipeline()`

---

## match_pattern: source stage vs filter stage

| Option | Description | Selected |
|--------|-------------|----------|
| Source stage — iterates all workspace files | Scans files, runs query, emits per-match nodes. No prior select needed. | ✓ |
| Filter stage — narrows from prior select | Works on nodes already in pipeline | |

**User's choice:** Source stage — iterates workspace files filtered by pipeline `languages` field

---

## compute_metric: helper location

| Option | Description | Selected |
|--------|-------------|----------|
| Move helpers to graph/metrics.rs | New file, executor imports from there, helpers.rs re-exports for compat | ✓ |
| Keep helpers in audit, pass via closure | Complex API, avoids moving code | |
| compute_metric is per-file, not graph-based | Runs in per-file loop, uses lib.rs re-export | |

**User's choice:** Move to `src/graph/metrics.rs` — clean separation, no circular dep

---

## compute_metric: transform vs source

| Option | Description | Selected |
|--------|-------------|----------|
| Transform — enriches existing symbol nodes | select(symbol) → compute_metric → flag | ✓ |
| Source stage — produces nodes with metric set | Scans workspace files directly | |

**User's choice:** Transform stage — takes nodes from prior `select(symbol)`, computes metric per node via workspace re-parse

---

## Stub stage behavior

| Option | Description | Selected |
|--------|-------------|----------|
| Implement useful ones, error the rest | traverse/filter/match_name real, count_edges/pair error | |
| All return loud errors | Simplest, satisfies ENG-05 | |
| Implement all 5 fully | Most complete | |
| **Remove all stubs** | Delete stubs + config structs + tests — spec unclear | ✓ |

**User's choice:** Remove all 5 stub stages entirely (traverse, filter, match_name, count_edges, pair). Also remove config structs and tests.
**Notes:** User stated "lets remove these stubs as we do not know what they are meant for" — spec uncertainty is the reason.

---

## Stub removal scope

| Option | Description | Selected |
|--------|-------------|----------|
| Remove everything — variants + config structs + tests | Clean sweep | ✓ |
| Keep config structs, only remove enum variants | Might cause dead_code warnings | |

**User's choice:** Full removal — `GraphStage` variants, config structs, and all referencing tests

---

## Test strategy

| Option | Description | Selected |
|--------|-------------|----------|
| Unit tests in executor.rs with in-memory bytes | Fast, isolated, minimal Workspace | ✓ |
| Integration tests in audit_json_integration.rs | Full AuditEngine path | |
| Both | Best coverage, more work | |

**User's choice:** Unit tests in `executor.rs` with in-memory file bytes via MemoryFileSource

---

## Claude's Discretion

- `run_pipeline()` parameter order: `workspace` added as third parameter (before `seed_nodes`)
- `MatchPattern` and `ComputeMetric` JSON keys follow `GroupBy` pattern (struct field name = key name)
- streaming_iterator constraint for tree-sitter 0.25 `QueryMatches` applies to `match_pattern` implementation
- Graceful skip (eprintln + continue) when compute_metric can't locate a symbol's body in the re-parsed tree

## Deferred Ideas

None — all discussion stayed within Phase 2 scope.
