---
phase: 2
slug: executor-stage-implementation
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-16
---

# Phase 2 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | cargo test (Rust built-in) |
| **Config file** | Cargo.toml |
| **Quick run command** | `cargo test 2>&1 | tail -5` |
| **Full suite command** | `cargo test` |
| **Estimated runtime** | ~30 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test 2>&1 | tail -5`
- **After every plan wave:** Run `cargo test`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 30 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 2-01-01 | 01 | 1 | ENG-03 | — | N/A | unit | `cargo test match_pattern` | ✅ | ⬜ pending |
| 2-01-02 | 01 | 1 | ENG-04 | — | N/A | unit | `cargo test compute_metric` | ✅ | ⬜ pending |
| 2-01-03 | 01 | 2 | ENG-05 | — | N/A | unit | `cargo test stub_stages` | ✅ | ⬜ pending |
| 2-01-04 | 01 | 2 | ENG-03 | — | N/A | integration | `cargo test` | ✅ | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] Existing `cargo test` infrastructure covers all phase requirements — no new test framework needed
- [ ] Unit tests for `match_pattern` stage (in-memory workspace with TypeScript fixture)
- [ ] Unit tests for `compute_metric` stage with `cyclomatic_complexity`
- [ ] Unit tests verifying stub stages return descriptive errors (not silent pass-through)

*Existing infrastructure covers test execution; new test functions required for new stage behavior.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| CLI output formatting for executor findings | ENG-03 | Requires visual inspection of JSON output | Run `cargo run -- audit <DIR> --pipeline <json_pipeline>` and inspect output |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 30s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
