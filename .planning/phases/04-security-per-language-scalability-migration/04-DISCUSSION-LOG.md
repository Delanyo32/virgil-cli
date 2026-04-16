# Phase 4: Security + Per-Language Scalability Migration - Discussion Log

> **Audit trail only.** Do not use as input to planning, research, or execution agents.
> Decisions are captured in CONTEXT.md — this log preserves the alternatives considered.

**Date:** 2026-04-16
**Phase:** 04-security-per-language-scalability-migration
**Areas discussed:** Taint boundary, No security specs, memory_leak_indicators, Phase batching

---

## Taint Boundary

### Q1: How to classify ambiguous security pipelines?

| Option | Description | Selected |
|--------|-------------|----------|
| Match-pattern test | Migrate if Rust implementation uses only tree-sitter pattern matching (no FlowsTo/SanitizedBy, no multi-file tracking) | ✓ |
| Strict: named patterns only | Only command injection, unsafe memory, integer overflow (the 3 named in SEC-01) | |
| Permissive: all non-taint | Anything not requiring FlowsTo/SanitizedBy migrates, including race_conditions | |

**User's choice:** Match-pattern test
**Notes:** path_traversal, insecure_deserialization, weak_cryptography, type_confusion, reflection_injection all pass the test and migrate.

---

### Q2: How to handle race_conditions specifically?

| Option | Description | Selected |
|--------|-------------|----------|
| Per-language judgment | Inspect each language's Rust impl; migrate if pure tree-sitter, leave in Rust with docs if graph-level | ✓ |
| Skip race_conditions entirely | Defer all to v2 to reduce Phase 4 scope | |
| Migrate all race_conditions | Force all to JSON, accept precision loss | |

**User's choice:** Per-language judgment
**Notes:** Rust and Go versions likely migrate (mutex/channel AST patterns). C# and Java versions may stay Rust if they need thread-level analysis.

---

## No Security Specs

### Q1: How to handle missing security audit_plans?

| Option | Description | Selected |
|--------|-------------|----------|
| Derive from Rust directly | Read existing Rust impl, translate tree-sitter queries to JSON. No new audit_plans. | ✓ |
| Write security audit_plans first | Expand Phase 4 to write specs before migration | |

**User's choice:** Derive from Rust directly
**Notes:** Velocity decision consistent with Phase 3's n_plus_one_queries tradeoff.

---

### Q2: Fix obvious Rust bugs or strict parity?

| Option | Description | Selected |
|--------|-------------|----------|
| Fix obvious bugs | Fix clear/trivial bugs during migration. Non-obvious bugs deferred. | ✓ |
| Strict parity | Preserve Rust behavior exactly, even bugs | |

**User's choice:** Fix obvious bugs
**Notes:** "Obvious bug" = wrong function name in hardcoded list, typo in pattern, missed clear variant. Non-trivial logic changes deferred.

---

## memory_leak_indicators

### Q1: Per-language files or cross-language?

| Option | Description | Selected |
|--------|-------------|----------|
| 10 per-language files | One JSON per language, same convention as sync_blocking_in_async | ✓ |
| Cross-language with variants | One file, multiple match_pattern stages | |
| Defer memory_leak_indicators | Skip migration, leave all in Rust | |

**User's choice:** 10 per-language files
**Notes:** memory_leak_indicators_rust.json, _typescript.json, _javascript.json, etc.

---

### Q2: Fallback when language can't be expressed in match_pattern?

| Option | Description | Selected |
|--------|-------------|----------|
| Simplify the pattern | Write simplified match_pattern, accept precision loss, document delta | ✓ |
| Leave that language in Rust | Permanent Rust exception for that language's memory_leak_indicators | |
| Skip that language entirely | No migration for that language | |

**User's choice:** Simplify the pattern
**Notes:** Completeness preferred over precision for scalability migration.

---

## Phase Batching

### Q1: Plan organization strategy?

| Option | Description | Selected |
|--------|-------------|----------|
| By language group | Each plan = one language (all security + memory_leak_indicators) | ✓ |
| By pipeline category | Each plan = one pipeline type across all languages | |
| Security first, then scalability | Two macro-batches | |

**User's choice:** By language group
**Notes:** ~10 plans total, self-contained per language.

---

### Q2: Which language goes first?

| Option | Description | Selected |
|--------|-------------|----------|
| Rust first | No taint exceptions, all pass match_pattern test, establishes template | ✓ |
| TypeScript/JS first | Largest security surface (9+3 pipelines) | |
| C first | No taint exceptions, good middle complexity | |

**User's choice:** Rust first
**Notes:** Rust's clean security pipeline set (integer_overflow, unsafe_memory, race_conditions, path_traversal, etc.) with no sql/ssrf to exclude makes it the ideal template language.

---

## Claude's Discretion

- Exact ordering of language groups after Rust (planner decides)
- Whether TypeScript and JavaScript are combined or split into separate plans
- Per-language race_conditions judgment call (planner inspects Rust impl, decides)
- For memory_leak_indicators where simplification needed: extent of simplification

## Deferred Ideas

- Writing `audit_plans/<lang>_security.md` specs — velocity decision to skip for now
- Taint-based pipeline migration — deferred to v2 (TAINT-01)
