---
phase: 5
slug: final-cleanup-test-health
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-16
---

# Phase 5 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust built-in (`cargo test`) |
| **Config file** | `Cargo.toml` |
| **Quick run command** | `cargo test --lib 2>&1 | tail -5` |
| **Full suite command** | `cargo test 2>&1 | tail -20` |
| **Estimated runtime** | ~60 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --lib 2>&1 | tail -5`
- **After every plan wave:** Run `cargo test 2>&1 | tail -20`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 120 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 5-01-01 | 01 | 1 | CLEAN-01 | — | N/A | integration | `cargo test 2>&1 \| grep "test result"` | ✅ | ⬜ pending |
| 5-01-02 | 01 | 1 | CLEAN-02 | — | N/A | integration | `cargo test 2>&1 \| grep "test result"` | ✅ | ⬜ pending |
| 5-01-03 | 01 | 1 | CLEAN-03 | — | N/A | integration | `cargo test 2>&1 \| grep "test result"` | ✅ | ⬜ pending |
| 5-xx-xx | TBD | TBD | TEST-02 | — | N/A | integration | `cargo test 2>&1 \| grep "test result"` | ✅ | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

Existing infrastructure covers all phase requirements — `cargo test` infrastructure exists and passes. No new test framework installation needed.

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| `virgil audit` produces non-empty output for all categories | CLEAN-03 | Requires live binary execution against a sample codebase | Run `cargo run -- audit /path/to/sample --format json` and verify each category has `> 0` findings |
| No dead files in `src/audit/analyzers/` | CLEAN-01 | Structural/reference check requires codebase scanning | `grep -r "use crate::audit::analyzers" src/ --include="*.rs"` and verify each file in `src/audit/analyzers/` appears |
| No Rust pipelines remain for migrated categories | CLEAN-02 | Structural check | `ls src/audit/pipelines/` should be empty or only contain files for unmigrated categories |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 120s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
