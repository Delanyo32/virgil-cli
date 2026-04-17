# A1 Pattern Name Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename three audit pipeline pattern fields so they match benchmark manifest names, unlocking detection of `cyclomatic_complexity`, `function_length`, and `deep_nesting` across 7–9 languages.

**Architecture:** All three metrics already compute correctly. The only gap is the `pattern` string inside each pipeline's `flag` stage — the benchmark manifests compare on this exact string. No Rust code changes needed. The `deep_nesting` fix also consolidates 10 per-language files into one cross-language file (matching the approach used by `cyclomatic_complexity.json` and `function_length.json`), which requires updating test assertions that reference the old per-language pipeline names.

**Tech Stack:** JSON (pipeline DSL), Rust (integration tests in `tests/audit_json_integration.rs`), `cargo test`

---

### Task 1: Fix `cyclomatic_complexity` pattern name

**Files:**
- Modify: `src/audit/builtin/cyclomatic_complexity.json`
- Modify: `tests/audit_json_integration.rs`

- [ ] **Step 1: Edit the JSON flag pattern**

In `src/audit/builtin/cyclomatic_complexity.json`, change line 24:

```json
"pattern": "cyclomatic_complexity",
```

The full flag block after the change (lines 23–48):

```json
    {
      "flag": {
        "pattern": "cyclomatic_complexity",
        "message": "Function `{{name}}` has cyclomatic complexity of {{cyclomatic_complexity}} (threshold: 10)",
        "severity_map": [
          {
            "when": {
              "metrics": {
                "cyclomatic_complexity": {
                  "gte": 20
                }
              }
            },
            "severity": "error"
          },
          {
            "when": {
              "metrics": {
                "cyclomatic_complexity": {
                  "gt": 10
                }
              }
            },
            "severity": "warning"
          }
        ]
      }
    }
```

- [ ] **Step 2: Update integration test assertions**

Replace all occurrences of `"high_cyclomatic_complexity"` with `"cyclomatic_complexity"` in `tests/audit_json_integration.rs`. Use the Edit tool with `replace_all: true`:

old_string: `"high_cyclomatic_complexity"`
new_string: `"cyclomatic_complexity"`

This covers all 16 occurrences (assertions, error messages) in one pass.

- [ ] **Step 3: Run tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.` — zero failures.

- [ ] **Step 4: Commit**

```bash
git add src/audit/builtin/cyclomatic_complexity.json tests/audit_json_integration.rs
git commit -m "fix(audit): rename cyclomatic_complexity pattern from high_cyclomatic_complexity"
```

---

### Task 2: Fix `function_length` pattern name

**Files:**
- Modify: `src/audit/builtin/function_length.json`
- Modify: `tests/audit_json_integration.rs`

- [ ] **Step 1: Edit the JSON flag pattern**

In `src/audit/builtin/function_length.json`, change line 24:

```json
"pattern": "function_length",
```

The full flag block after the change (lines 23–50):

```json
    {
      "flag": {
        "pattern": "function_length",
        "message": "Function `{{name}}` is {{function_length}} lines long (threshold: 50)",
        "severity_map": [
          {
            "when": {
              "metrics": {
                "function_length": {
                  "gte": 100
                }
              }
            },
            "severity": "error"
          },
          {
            "when": {
              "metrics": {
                "function_length": {
                  "gt": 50
                }
              }
            },
            "severity": "warning"
          }
        ]
      }
    }
```

- [ ] **Step 2: Update integration test assertions**

Replace all occurrences of `"function_too_long"` with `"function_length"` in `tests/audit_json_integration.rs`. Use the Edit tool with `replace_all: true`:

old_string: `"function_too_long"`
new_string: `"function_length"`

This covers all 4 occurrences in one pass.

- [ ] **Step 3: Run tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.` — zero failures.

- [ ] **Step 4: Commit**

```bash
git add src/audit/builtin/function_length.json tests/audit_json_integration.rs
git commit -m "fix(audit): rename function_length pattern from function_too_long"
```

---

### Task 3: Consolidate and rename `deep_nesting` pipelines

**Files:**
- Create: `src/audit/builtin/deep_nesting.json`
- Delete ×10: `src/audit/builtin/deep_nesting_c.json`, `deep_nesting_cpp.json`, `deep_nesting_csharp.json`, `deep_nesting_go.json`, `deep_nesting_java.json`, `deep_nesting_javascript.json`, `deep_nesting_php.json`, `deep_nesting_python.json`, `deep_nesting_rust.json`, `deep_nesting_typescript.json`
- Modify: `tests/audit_json_integration.rs`

- [ ] **Step 1: Create `deep_nesting.json`**

Create `src/audit/builtin/deep_nesting.json` with this exact content:

```json
{
  "pipeline": "deep_nesting",
  "category": "complexity",
  "description": "Detects functions with excessive control flow nesting depth. Threshold: warning >= 4, error >= 6.",
  "graph": [
    {
      "select": "symbol",
      "where": { "kind": ["function", "method", "arrow_function"] },
      "exclude": { "is_test_file": true }
    },
    { "compute_metric": "nesting_depth" },
    {
      "flag": {
        "pattern": "deep_nesting",
        "message": "Function `{{name}}` has nesting depth of {{nesting_depth}} (threshold: 4)",
        "severity_map": [
          { "when": { "metrics": { "nesting_depth": { "gte": 6 } } }, "severity": "error" },
          { "when": { "metrics": { "nesting_depth": { "gte": 4 } } }, "severity": "warning" }
        ]
      }
    }
  ]
}
```

Notes on the `kind` superset: languages without `arrow_function` (C, Rust, Go, etc.) never produce nodes of that kind, so including it causes no false positives. `method` covers class methods in Java, C#, Python, PHP, etc.

- [ ] **Step 2: Delete the 10 per-language files**

```bash
rm src/audit/builtin/deep_nesting_c.json \
   src/audit/builtin/deep_nesting_cpp.json \
   src/audit/builtin/deep_nesting_csharp.json \
   src/audit/builtin/deep_nesting_go.json \
   src/audit/builtin/deep_nesting_java.json \
   src/audit/builtin/deep_nesting_javascript.json \
   src/audit/builtin/deep_nesting_php.json \
   src/audit/builtin/deep_nesting_python.json \
   src/audit/builtin/deep_nesting_rust.json \
   src/audit/builtin/deep_nesting_typescript.json
```

- [ ] **Step 3: Update pattern name in test assertions**

Replace all occurrences of `"excessive_nesting_depth"` with `"deep_nesting"` in `tests/audit_json_integration.rs`. Use the Edit tool with `replace_all: true`:

old_string: `"excessive_nesting_depth"`
new_string: `"deep_nesting"`

- [ ] **Step 4: Update pipeline name in test assertions**

The tests reference per-language pipeline names (e.g. `"deep_nesting_python"`, `"deep_nesting_rust"`). All must change to `"deep_nesting"` since there is now only one pipeline. Run this sed command to replace all variants in one pass:

```bash
sed -i '' 's/"deep_nesting_[a-z]*"/"deep_nesting"/g' tests/audit_json_integration.rs
```

Verify the replacement covered all cases:

```bash
grep -c 'deep_nesting_[a-z]' tests/audit_json_integration.rs
```

Expected output: `0`

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok.` — zero failures.

- [ ] **Step 6: Commit**

`git add -u` stages the 10 deletions from Step 2 automatically (they are tracked files that are now missing on disk):

```bash
git add src/audit/builtin/deep_nesting.json tests/audit_json_integration.rs
git add -u src/audit/builtin/
git commit -m "fix(audit): consolidate deep_nesting pipelines and rename pattern from excessive_nesting_depth"
```
