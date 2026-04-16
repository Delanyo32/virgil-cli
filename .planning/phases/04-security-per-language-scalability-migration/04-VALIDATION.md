---
phase: 4
slug: security-per-language-scalability-migration
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-16
---

# Phase 4 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust built-in test harness (`cargo test`) |
| **Config file** | None (standard cargo test) |
| **Quick run command** | `cargo test --test audit_json_integration` |
| **Full suite command** | `cargo test` |
| **Estimated runtime** | ~5s (quick), ~30s (full) |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --test audit_json_integration`
- **After every plan wave:** Run `cargo test`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 30 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 4-xx-01 | xx | 1 | SEC-01 | — | JSON security pipeline finds command injection pattern in positive fixture | integration | `cargo test --test audit_json_integration -- command_injection_<lang>_finds` | ❌ Wave 0 | ⬜ pending |
| 4-xx-02 | xx | 1 | SEC-01 | — | JSON security pipeline produces no findings on clean fixture | integration | `cargo test --test audit_json_integration -- command_injection_<lang>_clean` | ❌ Wave 0 | ⬜ pending |
| 4-xx-03 | xx | 1 | SEC-02 | — | Deleted Rust pipeline file removed; cargo compiles cleanly | compile | `cargo test` | ❌ Wave 0 | ⬜ pending |
| 4-xx-04 | xx | 2 | SCAL-02 | — | JSON memory_leak_indicators finds pattern for language group | integration | `cargo test --test audit_json_integration -- memory_leak_<lang>_finds` | ❌ Wave 0 | ⬜ pending |
| 4-xx-05 | xx | 2 | SCAL-02 | — | JSON memory_leak_indicators clean fixture returns no findings | integration | `cargo test --test audit_json_integration -- memory_leak_<lang>_clean` | ❌ Wave 0 | ⬜ pending |
| 4-xx-06 | xx | 2 | SCAL-03 | — | Deleted Rust scalability file removed; cargo compiles cleanly | compile | `cargo test` | ❌ Wave 0 | ⬜ pending |
| 4-xx-07 | xx | all | TEST-01 | — | Each deleted pipeline batch has positive + negative integration tests committed in same batch | integration | `cargo test --test audit_json_integration` | ✅ existing | ⬜ pending |
| 4-xx-08 | xx | all | TEST-02 | — | Full test suite passes after every batch | integration | `cargo test` | ✅ existing | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

*Task IDs will be refined by the planner when plan batches are defined per language group.*

---

## Wave 0 Requirements

- [ ] New test functions in `tests/audit_json_integration.rs` — 2 per migrated pipeline (positive + negative fixture), added in same commit as each batch
- No new test files or test framework installs needed — existing `audit_json_integration.rs` infrastructure is sufficient

*Pattern: follow the 24 existing tests in `tests/audit_json_integration.rs` as templates.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| Taint-based pipelines (SQL injection, XSS, SSRF) continue returning findings after migration | SEC-01 | Requires running `virgil audit security` against real fixture codebase and confirming taint findings still appear | Run `virgil audit security <fixture_dir>` and verify sql_injection/xss/ssrf findings present; Rust pipeline files for these NOT deleted |
| `virgil audit scalability` returns findings for all 9 language groups with no regressions | SCAL-02 | Requires baseline comparison across all language groups | Run audit on pre-migration baseline, record counts per pipeline per language, verify no language group drops to 0 after migration |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 30s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
