# Metric Pipeline Fix (start_line Indexing) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `deep_nesting`, `function_length`, and `cyclomatic_complexity` audit pipelines that produce zero findings by correcting an off-by-one error in how all language parsers store line numbers.

**Architecture:** All 10 language parsers store `start_position().row` (0-indexed) directly into `start_line`/`end_line`, but the rest of the system — including `execute_compute_metric` and user-facing output — treats line numbers as 1-indexed. Adding `+1` at the parser layer makes the system consistent. No changes needed in `executor.rs` or any pipeline JSON files.

**Tech Stack:** Rust, tree-sitter. All changes are in `src/languages/`. Run with `cargo test`.

---

## File Map

| File | Change |
|---|---|
| `src/languages/typescript.rs` | Add `+1` to 6 row sites + update 1 test assertion |
| `src/languages/rust_lang.rs` | Add `+1` to 5 row sites |
| `src/languages/go.rs` | Add `+1` to 5 row sites |
| `src/languages/java.rs` | Add `+1` to 4 row sites |
| `src/languages/python.rs` | Add `+1` to 6 row sites |
| `src/languages/c_lang.rs` | Add `+1` to 4 row sites |
| `src/languages/cpp.rs` | Add `+1` to 4 row sites |
| `src/languages/csharp.rs` | Add `+1` to 4 row sites |
| `src/languages/php.rs` | Add `+1` to 5 row sites |

No other files change. No new files.

---

### Task 1: Fix TypeScript parser (TDD anchor)

The TypeScript parser has an existing test that asserts `start_line == 0` for a first-line function. Update it to the correct expectation first, then fix the parser to make it pass.

**Files:**
- Modify: `src/languages/typescript.rs`

- [ ] **Step 1: Update the test assertion to the correct expected value**

In `src/languages/typescript.rs`, find the `positions_are_sane` test (around line 902) and change:

```rust
assert_eq!(syms[0].start_line, 0);
```
to:
```rust
assert_eq!(syms[0].start_line, 1);
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
cargo test --lib positions_are_sane 2>&1 | tail -20
```

Expected: FAIL — `assertion failed: (left == right) left: 0, right: 1`

- [ ] **Step 3: Fix all row conversions in `typescript.rs`**

Make the following six changes. Each is a mechanical `+1` addition:

**Line 177** (`start_line` for function/class symbols):
```rust
// Before:
start_line: def_node.start_position().row as u32,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32 + 1,
```

**Line 262** (`line` for static imports):
```rust
// Before:
let line = import_node.start_position().row as u32;

// After:
let line = import_node.start_position().row as u32 + 1;
```

**Line 299** (`line` for re-exports):
```rust
// Before:
let line = reexport_node.start_position().row as u32;

// After:
let line = reexport_node.start_position().row as u32 + 1;
```

**Line 340** (`line` for dynamic imports):
```rust
// Before:
line: dynamic_node.start_position().row as u32,

// After:
line: dynamic_node.start_position().row as u32 + 1,
```

**Line 360** (`line` for callsites):
```rust
// Before:
line: call_node.start_position().row as u32,

// After:
line: call_node.start_position().row as u32 + 1,
```

**Lines 562 and 564** (`start_line`/`end_line` for comment nodes):
```rust
// Before:
start_line: node.start_position().row as u32,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32 + 1,
```

- [ ] **Step 4: Run the test to confirm it passes**

```bash
cargo test --lib positions_are_sane 2>&1 | tail -10
```

Expected: `test languages::typescript::tests::positions_are_sane ... ok`

- [ ] **Step 5: Run the full test suite to catch regressions**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/languages/typescript.rs
git commit -m "fix(parser): make TypeScript start_line/end_line 1-indexed"
```

---

### Task 2: Fix Rust parser

**Files:**
- Modify: `src/languages/rust_lang.rs`

- [ ] **Step 1: Apply all row `+1` fixes in `rust_lang.rs`**

**Lines 125 and 127** (primary function symbol block):
```rust
// Before:
start_line: def_node.start_position().row as u32,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32 + 1,
```

**Line 213** (import line):
```rust
// Before:
let line = import_node.start_position().row as u32;

// After:
let line = import_node.start_position().row as u32 + 1;
```

**Lines 329 and 331** (secondary symbol block — macro/const/etc.):
```rust
// Before:
start_line: node.start_position().row as u32,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32 + 1,
```

- [ ] **Step 2: Run tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/languages/rust_lang.rs
git commit -m "fix(parser): make Rust start_line/end_line 1-indexed"
```

---

### Task 3: Fix Go and Java parsers

**Files:**
- Modify: `src/languages/go.rs`
- Modify: `src/languages/java.rs`

- [ ] **Step 1: Apply `+1` fixes in `go.rs`**

**Lines 116 and 118** (function/method symbols):
```rust
// Before:
start_line: def_node.start_position().row as u32,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32 + 1,
```

**Line 207** (import line):
```rust
// Before:
line: import_node.start_position().row as u32,

// After:
line: import_node.start_position().row as u32 + 1,
```

**Lines 249 and 251** (secondary symbol block):
```rust
// Before:
start_line: node.start_position().row as u32,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32 + 1,
```

- [ ] **Step 2: Apply `+1` fixes in `java.rs`**

**Line 119** (`start_line` for method/class symbols):
```rust
// Before:
start_line: def_node.start_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
```

Also fix `end_line` on the adjacent line if present (same pattern).

**Line 203** (import line):
```rust
// Before:
line: node.start_position().row as u32,

// After:
line: node.start_position().row as u32 + 1,
```

**Line 272** (secondary symbol block):
```rust
// Before:
start_line: node.start_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
```

Also fix adjacent `end_line` if present.

- [ ] **Step 3: Run tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/languages/go.rs src/languages/java.rs
git commit -m "fix(parser): make Go and Java start_line/end_line 1-indexed"
```

---

### Task 4: Fix Python, C, and C++ parsers

**Files:**
- Modify: `src/languages/python.rs`
- Modify: `src/languages/c_lang.rs`
- Modify: `src/languages/cpp.rs`

- [ ] **Step 1: Apply `+1` fixes in `python.rs`**

**Lines 136 and 138** (primary function/class symbol block):
```rust
// Before:
start_line: def_node.start_position().row as u32,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
start_column: def_node.start_position().column as u32,
end_line: def_node.end_position().row as u32 + 1,
```

**Line 227** (import line):
```rust
// Before:
let line = import_node.start_position().row as u32;

// After:
let line = import_node.start_position().row as u32 + 1;
```

**Lines 415 and 417** (second symbol block):
```rust
// Before:
start_line: node.start_position().row as u32,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32 + 1,
```

**Lines 446 and 448** (third symbol block):
```rust
// Before:
start_line: node.start_position().row as u32,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
start_column: node.start_position().column as u32,
end_line: node.end_position().row as u32 + 1,
```

- [ ] **Step 2: Apply `+1` fixes in `c_lang.rs`**

**Line 133** (primary function symbol):
```rust
// Before:
start_line: def_node.start_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
```

Fix adjacent `end_line` if present.

**Line 234** (include/import line):
```rust
// Before:
line: include_node.start_position().row as u32,

// After:
line: include_node.start_position().row as u32 + 1,
```

**Line 285** (secondary symbol block):
```rust
// Before:
start_line: node.start_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
```

Fix adjacent `end_line` if present.

- [ ] **Step 3: Apply `+1` fixes in `cpp.rs`**

Same three-site pattern as `c_lang.rs` — lines 141, 245, 296:

```rust
// Before (each site):
start_line: def_node.start_position().row as u32,   // line 141
line: include_node.start_position().row as u32,      // line 245
start_line: node.start_position().row as u32,        // line 296

// After (each site):
start_line: def_node.start_position().row as u32 + 1,
line: include_node.start_position().row as u32 + 1,
start_line: node.start_position().row as u32 + 1,
```

Fix adjacent `end_line` entries wherever found.

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/languages/python.rs src/languages/c_lang.rs src/languages/cpp.rs
git commit -m "fix(parser): make Python, C, and C++ start_line/end_line 1-indexed"
```

---

### Task 5: Fix C# and PHP parsers

**Files:**
- Modify: `src/languages/csharp.rs`
- Modify: `src/languages/php.rs`

- [ ] **Step 1: Apply `+1` fixes in `csharp.rs`**

**Line 128** (primary symbol):
```rust
// Before:
start_line: def_node.start_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
```

Fix adjacent `end_line` (line 130).

**Line 213** (secondary line field):
```rust
// Before:
line: node.start_position().row as u32,

// After:
line: node.start_position().row as u32 + 1,
```

**Line 270** (tertiary symbol block):
```rust
// Before:
start_line: node.start_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
```

Fix adjacent `end_line` (line 272).

- [ ] **Step 2: Apply `+1` fixes in `php.rs`**

**Line 138** (primary function symbol):
```rust
// Before:
start_line: def_node.start_position().row as u32,

// After:
start_line: def_node.start_position().row as u32 + 1,
```

Fix adjacent `end_line` (line 140).

**Lines 256, 277, 299** (import/use statement lines):
```rust
// Before (each):
node.start_position().row as u32,

// After (each):
node.start_position().row as u32 + 1,
```

**Line 444** (secondary symbol block):
```rust
// Before:
start_line: node.start_position().row as u32,

// After:
start_line: node.start_position().row as u32 + 1,
```

Fix adjacent `end_line` (line 446).

- [ ] **Step 3: Run tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/languages/csharp.rs src/languages/php.rs
git commit -m "fix(parser): make C# and PHP start_line/end_line 1-indexed"
```

---

### Task 6: Integration verification

Confirm the three metric pipelines now produce findings on real code. The benchmark codebases from the report are at `../virgil-skills/benchmarks/` — use one to verify.

**Files:** Read-only verification — no code changes.

- [ ] **Step 1: Build release binary**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: `Finished release [optimized]`

- [ ] **Step 2: Verify `deep_nesting` fires on a Rust codebase**

```bash
cargo run -- audit --dir ../virgil-skills/benchmarks/rust/systems-cli \
  --language rs --pipeline deep_nesting_rust 2>&1 | head -30
```

Expected: findings at `src/core/pipeline.rs` around line 106 (previously produced 0 findings).

- [ ] **Step 3: Verify `function_length` fires**

```bash
cargo run -- audit --dir ../virgil-skills/benchmarks/rust/systems-cli \
  --language rs --pipeline function_length 2>&1 | head -20
```

Expected: finding for `main()` spanning ~202 lines in `src/main.rs`.

- [ ] **Step 4: Verify `cyclomatic_complexity` fires**

```bash
cargo run -- audit --dir ../virgil-skills/benchmarks/javascript/express-api \
  --language js --pipeline cyclomatic_complexity 2>&1 | head -20
```

Expected: finding for `searchPosts()` with high CC in `src/services/postService.js`.

- [ ] **Step 5: Verify `match_pattern` line numbers are unchanged**

Confirm that a known-working pipeline still reports the same line numbers (no regression):

```bash
cargo run -- audit --dir ../virgil-skills/benchmarks/rust/systems-cli \
  --language rs --pipeline panic_prone_calls_rust 2>&1 | head -20
```

Expected: same findings as before — line numbers should be identical to pre-fix output since `match_pattern` already added `+1` independently.

- [ ] **Step 6: Commit verification note**

No code changes — nothing to commit.

---

## Done

All three metric pipelines (`deep_nesting`, `function_length`, `cyclomatic_complexity`) should now produce findings across all supported languages. The root cause (0-indexed `start_line` in parsers vs. 1-indexed assumption in the executor) is resolved consistently across all 9 language parser files.
