---
phase: 3
slug: tech-debt-scalability-json-migration
status: draft
nyquist_compliant: true
wave_0_complete: true
created: 2026-04-16
---

# Phase 3 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust `cargo test` (built-in) |
| **Config file** | `Cargo.toml` |
| **Quick run command** | `cargo test audit_json` |
| **Full suite command** | `cargo test` |
| **Estimated runtime** | ~30–60 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test audit_json`
- **After every plan wave:** Run `cargo test`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 60 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 3-01-01 | 01 | 0 | TECH-01 | — | N/A | unit | `cargo test` | ✅ | ⬜ pending |
| 3-01-02 | 01 | 1 | TECH-01 | — | N/A | integration | `cargo test audit_json` | ✅ | ⬜ pending |
| 3-02-01 | 02 | 1 | TECH-01 | — | N/A | integration | `cargo test audit_json` | ✅ | ⬜ pending |
| 3-03-01 | 03 | 2 | SCAL-01 | — | N/A | integration | `cargo test audit_json` | ✅ | ⬜ pending |
| 3-04-01 | 04 | 2 | TEST-01 | — | N/A | integration | `cargo test` | ✅ | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `src/graph/pipeline.rs` — Add `cyclomatic_complexity`, `function_length`, `cognitive_complexity`, `comment_to_code_ratio` fields to `WhereClause` and `eval_metrics()`
- [ ] `src/graph/pipeline.rs` — Add `kind: Option<Vec<String>>` to `WhereClause` for symbol kind filtering

*These are Rust code changes (not test stubs), required before JSON pipelines can compile and run.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| `virgil audit code-quality complexity` returns findings for all 9 language groups | TECH-01 | Requires real codebase with known violations | Run against `tests/fixtures/` with known complex functions; check output |
| `virgil audit scalability` n_plus_one_queries and sync_blocking_in_async findings | SCAL-01 | Requires fixture files with async loop patterns | Run against dedicated fixture; verify finding count > 0 |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 60s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
