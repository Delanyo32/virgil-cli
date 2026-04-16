---
phase: 1
slug: engine-fixes-architecture-json-expansion
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-04-16
---

# Phase 1 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust built-in test (`cargo test`) |
| **Config file** | `Cargo.toml` |
| **Quick run command** | `cargo test --lib` |
| **Full suite command** | `cargo test` |
| **Estimated runtime** | ~30 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --lib`
- **After every plan wave:** Run `cargo test`
- **Before `/gsd-verify-work`:** Full suite must be green
- **Max feedback latency:** 60 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 1-01-01 | 01 | 1 | ENG-01 | — | N/A | unit | `cargo test eng_01` | ❌ W0 | ⬜ pending |
| 1-01-02 | 01 | 1 | ENG-02 | — | N/A | unit | `cargo test builtin_audits` | ✅ | ⬜ pending |
| 1-02-01 | 02 | 2 | ARCH-01 | — | N/A | integration | `cargo test audit_json_integration` | ❌ W0 | ⬜ pending |
| 1-02-02 | 02 | 2 | ARCH-02 | — | N/A | integration | `cargo test audit_json_integration` | ❌ W0 | ⬜ pending |
| 1-03-01 | 03 | 2 | ARCH-03–09 | — | N/A | integration | `cargo test audit_json_integration` | ❌ W0 | ⬜ pending |
| 1-04-01 | 04 | 3 | ARCH-10 | — | N/A | compile | `cargo build` | ✅ | ⬜ pending |
| 1-05-01 | 05 | 3 | TEST-01, TEST-02 | — | N/A | integration | `cargo test` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- [ ] `tests/audit_json_integration.rs` — integration test stubs for ARCH-01 through ARCH-09 + ENG-01 suppression
- [ ] Test fixtures in `tests/fixtures/` — minimal Rust, TypeScript, Python, Go, Java, C, C++, C#, PHP source trees for smoke tests

*Existing `cargo test` infrastructure covers the framework layer.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| `leaky_abstraction_boundary` deferred | ARCH-10 (partial) | Requires tree-sitter field inspection not in Phase 1 DSL | Verify it is NOT included in any Phase 1 JSON file |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 60s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
