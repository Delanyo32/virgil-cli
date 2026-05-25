# test_to_function_map Query Optimisations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optimise `test_to_function_map.cozoql` by pre-materialising `call_edge` at build time, dropping its regex filter for a relation join, pushing the test-file filter ahead of the join, and adding the one missing index. Then bench end-to-end (build + query) on master vs feat across openclaw subdirs of increasing size.

**Architecture:** Adds one persistent relation (`*call_edge`) populated during `cozo::populate()`, and one new compound index (`symbol:by_name_kind`). Schema-version bumps from 8 to 9; existing caches wipe-and-rebuild on first open of the new binary. Bench harness is a bash script that rebuilds the binary from both branches and parses `/usr/bin/time -lp` output.

**Tech Stack:** Rust 2024, cozo-ce 0.7.13-alpha.3 (SQLite backend), tree-sitter, rayon. Bench in bash.

**Spec:** `docs/superpowers/specs/2026-05-25-test-to-function-map-optimisations-design.md`

---

## File map

- Create: `tests/integration_call_edge.rs` — schema + equivalence tests
- Create: `examples/bench_query_optimisations.sh` — bench harness
- Create: `examples/bench_query_optimisations.md` — bench README
- Create: `examples/test_to_function_map.baseline.cozoql` — original query (for baseline binary)
- Create: `examples/test_to_function_map.optimised.cozoql` — rewritten query (for optimised binary)
- Modify: `src/cozo/mod.rs` — bump `SCHEMA_VERSION: u32 = 8 → 9`
- Modify: `src/cozo/schema.rs` — add `:create call_edge` + `symbol:by_name_kind` index
- Modify: `src/cozo/writer.rs` — add `call_edge` row buffer + `push_call_edge()` + flush entry
- Modify: `src/cozo/from_code_graph.rs` — add `resolve_and_emit_call_edges()`, call from `populate()`

The orchestrator-side query rewrite (in the virgil-audit repo) is **out of scope for this branch**. We commit two query files under `examples/` here so the bench can pick the right one per binary.

---

### Task 1: Schema bump + new relation + new index

**Files:**
- Modify: `src/cozo/mod.rs` (around the `SCHEMA_VERSION` constant)
- Modify: `src/cozo/schema.rs:31-44` (graph-edges section) and `:140-143` (index list tail)

- [ ] **Step 1: Bump SCHEMA_VERSION**

In `src/cozo/mod.rs`, change:
```rust
pub const SCHEMA_VERSION: u32 = 8;
```
to:
```rust
pub const SCHEMA_VERSION: u32 = 9;
```
Add a new bullet at the bottom of the existing version-history doc comment:
```rust
/// 9: Added `call_edge {caller_id, callee_id => file_path}` relation
/// populated at build time by `from_code_graph::resolve_and_emit_call_edges`,
/// plus `symbol:by_name_kind {name, kind}` index. Lets queries that need
/// resolved call edges skip the per-query recursion.
```

- [ ] **Step 2: Add `:create call_edge` to schema**

In `src/cozo/schema.rs`, after the existing `:create call_site` block (~line 45), insert:
```rust
        // Resolved call edges, materialised at build time. Each row
        // corresponds to one call site whose callee_name resolved to a
        // single symbol id (intra-file by name+kind, cross-file via
        // *imports + exported=true). Schema v9.
        ":create call_edge {caller_id: String, callee_id: String => file_path: String}",
```

- [ ] **Step 3: Add the new compound index**

In `src/cozo/schema.rs::index_statements()`, append (after `call_site:by_name`):
```rust
        // call_edge resolution scans *symbol by (name, kind) on the
        // cross-file branch. The existing symbol:by_name index alone
        // forces a kind filter scan; the compound covers both.
        "::index create symbol:by_name_kind {name, kind}",
```

- [ ] **Step 4: Confirm it builds**

```bash
cargo build 2>&1 | tail -5
```
Expected: builds with no schema-related warnings.

- [ ] **Step 5: Confirm cargo test passes**

```bash
cargo test 2>&1 | tail -10
```
Expected: all tests pass (existing `populate_writes_symbols_and_call_edges_for_a_tiny_rust_workspace` will still pass because it queries `*call_site` directly, not `*call_edge`).

- [ ] **Step 6: Commit**

```bash
git add src/cozo/mod.rs src/cozo/schema.rs
git commit -m "feat(cozo): add call_edge relation + symbol:by_name_kind index (schema v9)

Declares the *call_edge relation but leaves it empty until the
resolver lands in a follow-up commit. Bumps SCHEMA_VERSION to 9 so
existing caches rebuild."
```

---

### Task 2: Writer support for `call_edge`

**Files:**
- Modify: `src/cozo/writer.rs` (field declaration around `:32`, append() around `:76`, push helper after `push_call_site` at `:213`, flush block after the `call_site` flush at `:638`)

- [ ] **Step 1: Add `call_edge` buffer field**

In `src/cozo/writer.rs`, find the existing fields block (`call_site: Vec<Vec<DataValue>>,` near line 32). Add after it:
```rust
    call_edge: Vec<Vec<DataValue>>,
```

In the `append()` method (near line 76, the `self.call_site.append(...)` line), add:
```rust
        self.call_edge.append(&mut other.call_edge);
```

In `CozoWriter::new()` (initialises `Default::default()` for each field), no change needed if it derives `Default` — verify by reading the struct definition. If the struct defines `new()` manually with explicit zero-inits, add `call_edge: Vec::new(),` to that init.

- [ ] **Step 2: Add `push_call_edge` helper**

In `src/cozo/writer.rs`, after `push_call_site` (around line 213), add:
```rust
    pub fn push_call_edge(&mut self, caller_id: &str, callee_id: &str, file_path: &str) {
        self.call_edge.push(vec![
            DataValue::from(caller_id),
            DataValue::from(callee_id),
            DataValue::from(file_path),
        ]);
    }
```

- [ ] **Step 3: Add the flush entry**

In the `flush()` method, after the `call_site` flush block (which ends around line 638), insert:
```rust
        flush(
            store,
            "?[caller_id, callee_id, file_path] <- $rows \
             :put call_edge {caller_id, callee_id => file_path}",
            std::mem::take(&mut self.call_edge),
        )?;
```

- [ ] **Step 4: Write the failing unit test**

In `src/cozo/writer.rs`, in the `mod tests {}` block at the bottom, add:
```rust
    #[test]
    fn flush_writes_call_edge_rows() {
        let store = CozoStore::open_in_memory().expect("open");
        let mut w = CozoWriter::new();
        w.push_call_edge("caller-id-1", "callee-id-1", "src/a.rs");
        w.push_call_edge("caller-id-2", "callee-id-2", "src/b.rs");
        w.flush(&store).expect("flush");

        let rows = store
            .run_query(
                "?[c, t, f] := *call_edge{caller_id: c, callee_id: t, file_path: f}",
                std::collections::BTreeMap::new(),
            )
            .expect("query");
        assert_eq!(rows.rows.len(), 2);
    }
```

- [ ] **Step 5: Run the test — expect FAIL**

```bash
cargo test --lib cozo::writer::tests::flush_writes_call_edge_rows 2>&1 | tail -15
```
Expected: FAIL with a missing-method error if push_call_edge isn't in scope yet, OR FAIL with row mismatch if the writer compiles but flush doesn't emit.

Note: If you completed steps 1–3 strictly before step 4, the test will PASS on first run. That's acceptable — the failing-test step in this task is a sanity check that the wiring is end-to-end. If it passes first time, proceed.

- [ ] **Step 6: Run full cargo test**

```bash
cargo test 2>&1 | tail -10
```
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/cozo/writer.rs
git commit -m "feat(cozo): add CozoWriter::push_call_edge + flush wiring

Buffer + flush path for the new *call_edge relation. Empty until the
resolver populates it."
```

---

### Task 3: Resolver — `resolve_and_emit_call_edges`

**Files:**
- Modify: `src/cozo/from_code_graph.rs` (new function after `populate`, called from inside `populate`)

- [ ] **Step 1: Add the resolver function**

In `src/cozo/from_code_graph.rs`, after the closing brace of `populate()` (around line 66), add:
```rust
/// Resolve every `*call_site` to a target `*symbol.id` and emit one
/// `*call_edge{caller_id, callee_id => file_path}` row per resolution.
///
/// Algorithm mirrors the rules that lived inline in the old
/// `test_to_function_map.cozoql`:
///   1. Intra-file: callee_name matches a *symbol{name, file_path, kind}
///      where kind in (function, method, arrow_function, macro) and the
///      callee is not the caller itself.
///   2. Cross-file: caller's file imports a file via *imports; that
///      imported file exports a *symbol{name = callee_name, kind in (...),
///      exported = true}.
///
/// Cost shifts from per-query to once-per-build. Read by any future query
/// that needs resolved call edges.
fn resolve_and_emit_call_edges(store: &CozoStore, writer: &mut CozoWriter) -> Result<()> {
    let _s = info_span!("cozo.populate.call_edge").entered();

    // Single Cozo query that emits both intra-file and cross-file
    // edges. Bind into the writer so the flush path is the same as
    // every other relation.
    let rows = store.run_query(
        "edge[caller_id, callee_id, file] := \
            *call_site{caller_id, callee_name, file_path: file}, \
            *symbol{id: callee_id, name: callee_name, file_path: file, kind: k}, \
            k in ['function', 'method', 'arrow_function', 'macro'], \
            caller_id != callee_id \
         edge[caller_id, callee_id, file] := \
            *call_site{caller_id, callee_name, file_path: file}, \
            *imports{importer_file_id: file, imported_id: callee_file}, \
            *symbol{id: callee_id, name: callee_name, file_path: callee_file, \
                    kind: k, exported: true}, \
            k in ['function', 'method', 'arrow_function', 'macro'], \
            caller_id != callee_id \
         ?[caller_id, callee_id, file] := edge[caller_id, callee_id, file]",
        BTreeMap::new(),
    )?;

    let mut count = 0usize;
    for row in &rows.rows {
        let caller_id = row[0].get_str().unwrap_or("");
        let callee_id = row[1].get_str().unwrap_or("");
        let file = row[2].get_str().unwrap_or("");
        if caller_id.is_empty() || callee_id.is_empty() {
            continue;
        }
        writer.push_call_edge(caller_id, callee_id, file);
        count += 1;
    }
    eprintln!("[bench] call_edge_count={count}");
    info!(call_edges = count, "cozo call_edge resolution complete");
    Ok(())
}
```

Note on `DataValue::get_str()`: that helper exists on `cozo::DataValue` for the String variant. If the method name differs in this version, replace with the equivalent pattern match:
```rust
let caller_id = match &row[0] {
    cozo::DataValue::Str(s) => s.as_str(),
    _ => continue,
};
```
Use whichever compiles cleanly — both shapes are present elsewhere in `from_code_graph.rs`; mirror the pattern used there.

- [ ] **Step 2: Wire it into `populate`**

In `src/cozo/from_code_graph.rs::populate()`, after the final `writer.flush(store)?` inside the `cozo.populate.flush` span block (around line 63), add:
```rust
    {
        let _r = info_span!("cozo.populate.call_edge_flush").entered();
        let mut writer = CozoWriter::new();
        resolve_and_emit_call_edges(store, &mut writer)?;
        writer.flush(store)?;
    }
```

This runs **after** the tail flush, so `*call_site`, `*symbol`, and `*imports` are all guaranteed to be queryable by the resolver.

- [ ] **Step 3: Confirm build**

```bash
cargo build 2>&1 | tail -5
```

- [ ] **Step 4: Run the existing call-edge test against the new path**

```bash
cargo test --lib cozo::from_code_graph::tests::populate_writes_symbols_and_call_edges_for_a_tiny_rust_workspace 2>&1 | tail -10
```
Expected: PASS — the existing test still queries via `*call_site` × `*symbol`, which is unchanged.

- [ ] **Step 5: Run full cargo test**

```bash
cargo test 2>&1 | tail -10
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/cozo/from_code_graph.rs
git commit -m "feat(cozo): resolve and persist call_edge at build time

After populate's tail flush, run the two-rule (intra-file + cross-file)
call resolution that test_to_function_map.cozoql was doing inline at
query time. Emits *call_edge rows so downstream queries can skip the
recursion."
```

---

### Task 4: Integration test — schema migration + call_edge population

**Files:**
- Create: `tests/integration_call_edge.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/integration_call_edge.rs`:
```rust
//! Confirms the schema v9 migration: *call_edge is populated at build
//! time with both intra-file and cross-file resolutions, and *call_site
//! is unchanged.

use std::collections::BTreeMap;

use tempfile::tempdir;

use virgil_cli::cozo::{CozoStore, populate};
use virgil_cli::graph::builder::GraphBuilder;
use virgil_cli::language::Language;
use virgil_cli::storage::workspace::Workspace;

#[test]
fn call_edge_is_populated_with_intra_and_cross_file_edges() {
    let dir = tempdir().expect("tempdir");

    // File a.rs defines beta + alpha (alpha calls beta — intra-file).
    // File b.rs imports beta from a, calls it (cross-file).
    std::fs::write(
        dir.path().join("a.rs"),
        "pub fn beta() {}\nfn alpha() { beta(); }\n",
    )
    .expect("write a.rs");
    std::fs::write(
        dir.path().join("b.rs"),
        "use crate::a::beta;\nfn gamma() { beta(); }\n",
    )
    .expect("write b.rs");

    let workspace =
        Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
    let store = CozoStore::open_in_memory().expect("open store");
    let graph = GraphBuilder::new(&workspace, &[Language::Rust])
        .build(&store)
        .expect("build graph");
    populate(&store, &graph, Some(&workspace)).expect("populate");

    let edges = store
        .run_query(
            "?[caller, callee, file] := \
             *call_edge{caller_id, callee_id, file_path: file}, \
             *symbol{id: caller_id, name: caller}, \
             *symbol{id: callee_id, name: callee}",
            BTreeMap::new(),
        )
        .expect("call_edge query");

    let pairs: Vec<(String, String)> = edges
        .rows
        .iter()
        .map(|r| (
            r[0].get_str().unwrap_or("").to_string(),
            r[1].get_str().unwrap_or("").to_string(),
        ))
        .collect();

    assert!(
        pairs.iter().any(|(a, b)| a == "alpha" && b == "beta"),
        "expected intra-file edge alpha -> beta in call_edge, got {pairs:?}"
    );
    // Cross-file edge depends on the Rust extractor's import handling.
    // If it's not yet wired, the assertion below will fail and reveal
    // that gap — fix in a follow-up, do NOT relax the test.
    assert!(
        pairs.iter().any(|(a, b)| a == "gamma" && b == "beta"),
        "expected cross-file edge gamma -> beta in call_edge, got {pairs:?}"
    );

    // *call_site is unchanged: every call expression still emits a row.
    let call_sites = store
        .run_query(
            "?[count(id)] := *call_site{id}",
            BTreeMap::new(),
        )
        .expect("call_site count");
    let n = match &call_sites.rows[0][0] {
        cozo::DataValue::Num(cozo::Num::Int(i)) => *i,
        other => panic!("expected int, got {other:?}"),
    };
    assert!(n >= 2, "expected at least 2 call_site rows, got {n}");
}
```

- [ ] **Step 2: Run the test — expect PASS if Task 3 is correct**

```bash
cargo test --test integration_call_edge 2>&1 | tail -20
```
Expected: PASS. If the cross-file assertion fails, the Rust extractor's import wiring is missing for this fixture. **Do not weaken the assertion.** Investigate via:
```bash
cargo test --test integration_call_edge -- --nocapture 2>&1 | tail -40
```
The `[bench] call_edge_count=N` line printed by the resolver tells you whether the resolver ran. If the cross-file row is genuinely missing because the Rust extractor doesn't synthesise the `*imports` row from `use crate::a::beta;`, change the cross-file fixture to use a less ambiguous import shape and document why in a code comment.

- [ ] **Step 3: Run full cargo test**

```bash
cargo test 2>&1 | tail -10
```

- [ ] **Step 4: Commit**

```bash
git add tests/integration_call_edge.rs
git commit -m "test(cozo): assert call_edge is populated with intra and cross-file edges

Tiny Rust fixture; checks both branches of the resolver and confirms
*call_site is unchanged."
```

---

### Task 5: Equivalence test — old vs new query shape

**Files:**
- Create: `examples/test_to_function_map.baseline.cozoql` — the original
- Create: `examples/test_to_function_map.optimised.cozoql` — the rewrite
- Modify: `tests/integration_call_edge.rs` (append a new `#[test]`)

- [ ] **Step 1: Save the baseline query**

Create `examples/test_to_function_map.baseline.cozoql`. Body (the exact query the orchestrator runs, with `//` headers stripped):

```
call_edge[caller_id, callee_id, file] :=
    *call_site{caller_id, callee_name, file_path: file},
    *symbol{id: callee_id, name: callee_name, file_path: file, kind: k},
    k in ['function', 'method', 'arrow_function', 'macro'],
    caller_id != callee_id

call_edge[caller_id, callee_id, file] :=
    *call_site{caller_id, callee_name, file_path: file},
    *imports{importer_file_id: file, imported_id: callee_file},
    *symbol{id: callee_id, name: callee_name, file_path: callee_file,
            kind: k, exported: true},
    k in ['function', 'method', 'arrow_function', 'macro'],
    caller_id != callee_id

?[file, line, severity, pattern, message] :=
    call_edge[c, t, file],
    *symbol{id: c, name: caller_name},
    *symbol{id: t, name: callee_name},
    regex_matches(file, "(?i)(test|spec|__tests__|\\.test\\.|\\.spec\\.)"),
    *span{entity_id: c, file_path: file, start_line: line},
    severity = "info",
    pattern = "test_call",
    message = concat("test=", caller_name, "|callee=", callee_name)
```

- [ ] **Step 2: Save the optimised query**

Create `examples/test_to_function_map.optimised.cozoql`:

```
?[file, line, severity, pattern, message] :=
    *call_edge{caller_id: c, callee_id: t, file_path: file},
    *file_classification{path: file, is_test: true},
    *symbol{id: c, name: caller_name},
    *symbol{id: t, name: callee_name},
    *span{entity_id: c, file_path: file, start_line: line},
    severity = "info",
    pattern = "test_call",
    message = concat("test=", caller_name, "|callee=", callee_name)
```

Schema confirmed: `file_classification` is keyed by `path: String`, not `file_id`.

- [ ] **Step 3: Add the equivalence test**

Append to `tests/integration_call_edge.rs`:
```rust
#[test]
fn baseline_and_optimised_queries_return_identical_rows() {
    let dir = tempdir().expect("tempdir");

    // Two test files (so file_classification.is_test = true) and one
    // non-test file. Each contains a call.
    std::fs::write(
        dir.path().join("helper.rs"),
        "pub fn target() {}\n",
    )
    .expect("write helper");
    std::fs::write(
        dir.path().join("widget_test.rs"),
        "use crate::helper::target;\nfn test_widget() { target(); }\n",
    )
    .expect("write widget_test");
    std::fs::write(
        dir.path().join("plain.rs"),
        "use crate::helper::target;\nfn unused() { target(); }\n",
    )
    .expect("write plain");

    let workspace =
        Workspace::load(dir.path(), &[Language::Rust], None).expect("load workspace");
    let store = CozoStore::open_in_memory().expect("open store");
    let graph = GraphBuilder::new(&workspace, &[Language::Rust])
        .build(&store)
        .expect("build graph");
    populate(&store, &graph, Some(&workspace)).expect("populate");

    let baseline = std::fs::read_to_string("examples/test_to_function_map.baseline.cozoql")
        .expect("read baseline");
    let optimised = std::fs::read_to_string("examples/test_to_function_map.optimised.cozoql")
        .expect("read optimised");

    let b_rows = store.run_query(&baseline, BTreeMap::new()).expect("baseline");
    let o_rows = store.run_query(&optimised, BTreeMap::new()).expect("optimised");

    let mut b: Vec<Vec<String>> = b_rows
        .rows
        .iter()
        .map(|r| r.iter().map(|c| format!("{c:?}")).collect())
        .collect();
    let mut o: Vec<Vec<String>> = o_rows
        .rows
        .iter()
        .map(|r| r.iter().map(|c| format!("{c:?}")).collect())
        .collect();
    b.sort();
    o.sort();

    assert_eq!(
        b, o,
        "baseline and optimised queries returned different row sets"
    );
    // Sanity: we should have rows for the test file only, not for plain.rs.
    assert!(!b.is_empty(), "expected at least one row (test_widget -> target)");
    for row in &o {
        assert!(
            row[0].contains("widget_test.rs"),
            "expected only test-file rows, found row from {:?}",
            row[0]
        );
    }
}
```

- [ ] **Step 4: Run the equivalence test**

```bash
cargo test --test integration_call_edge baseline_and_optimised_queries_return_identical_rows 2>&1 | tail -25
```
Expected: PASS. If the row sets differ, the optimisation is not semantically equivalent — STOP and investigate. Do not relax the assertion. Common causes: `*file_classification` key mismatch (use the `path:` field, not anything else), or the rewrite drops a join column.

- [ ] **Step 5: Run full cargo test**

```bash
cargo test 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
git add examples/test_to_function_map.baseline.cozoql \
        examples/test_to_function_map.optimised.cozoql \
        tests/integration_call_edge.rs
git commit -m "test: pin baseline vs optimised test_to_function_map equivalence

Two checked-in query files (the orchestrator's original and the
rewrite) plus a test that runs both against the same fixture store and
asserts identical row sets."
```

---

### Task 6: Bench harness script

**Files:**
- Create: `examples/bench_query_optimisations.sh`

- [ ] **Step 1: Write the bench script**

Create `examples/bench_query_optimisations.sh`:
```bash
#!/usr/bin/env bash
# Bench: master vs feat/test-to-function-map-optimisations on openclaw subdirs.
# Outputs bench-results.csv with one row per (binary, subdir) pair.
#
# Usage:
#   ./examples/bench_query_optimisations.sh <openclaw-clone-path> <subdir1> [<subdir2> ...]
#
# Example:
#   ./examples/bench_query_optimisations.sh /tmp/openclaw \
#       Source/ActorComponent Source/ActorController Source Build
#
# Each <subdir> is benched at its full file count. The script does not
# slice files itself — pick subdirs whose file counts span the range you
# want (~50 / ~500 / ~2000 / ~5000).

set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <openclaw-clone-path> <subdir1> [<subdir2> ...]" >&2
  exit 1
fi

OPENCLAW="$1"; shift
SUBDIRS=("$@")

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE_DIR="$HOME/Library/Caches/virgil"   # macOS; adjust for linux
BASELINE_BIN="$REPO_ROOT/target/release/virgil-cli-baseline"
OPTIMISED_BIN="$REPO_ROOT/target/release/virgil-cli-optimised"
RESULTS="$REPO_ROOT/bench-results.csv"

echo "binary,subdir,files,wall_s,user_s,sys_s,max_rss_mb,call_edge_count" > "$RESULTS"

build_baseline() {
  echo "[bench] building master binary..."
  git -C "$REPO_ROOT" stash push --include-untracked -m "bench-stash" >/dev/null
  trap 'git -C "$REPO_ROOT" stash pop >/dev/null 2>&1 || true' EXIT
  local cur_branch
  cur_branch=$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD)
  git -C "$REPO_ROOT" checkout master
  (cd "$REPO_ROOT" && cargo build --release)
  cp "$REPO_ROOT/target/release/virgil-cli" "$BASELINE_BIN"
  git -C "$REPO_ROOT" checkout "$cur_branch"
  trap - EXIT
  git -C "$REPO_ROOT" stash pop >/dev/null 2>&1 || true
}

build_optimised() {
  echo "[bench] building feat binary..."
  (cd "$REPO_ROOT" && cargo build --release)
  cp "$REPO_ROOT/target/release/virgil-cli" "$OPTIMISED_BIN"
}

run_one() {
  local label="$1" binary="$2" subdir="$3" query_file="$4"
  local target="$OPENCLAW/$subdir"
  local files
  files=$(find "$target" -type f | wc -l | tr -d ' ')

  # Cold start: wipe the project's cache.
  rm -rf "$CACHE_DIR"/*.sqlite 2>/dev/null || true

  # Capture full stderr (time -lp is on stderr; resolver's call_edge_count too).
  local time_out
  time_out=$(mktemp)
  # `--cozoscript` would require shell escaping; --file is cleaner.
  /usr/bin/time -lp "$binary" projects query bench-$$-$label-$RANDOM \
    --path "$target" --file "$query_file" >/dev/null 2>"$time_out" || true

  local wall user sys rss_kb call_edges
  wall=$(awk '/^real / {print $2}' "$time_out")
  user=$(awk '/^user / {print $2}' "$time_out")
  sys=$(awk '/^sys / {print $2}' "$time_out")
  rss_kb=$(awk '/maximum resident set size/ {print $1}' "$time_out")
  call_edges=$(awk '/^\[bench\] call_edge_count=/ {gsub("[^0-9]","",$0); print}' "$time_out")
  call_edges="${call_edges:-NA}"

  local rss_mb
  if [[ -n "${rss_kb:-}" ]]; then
    # On macOS the time -lp RSS is in bytes; on linux it's in KB. Detect:
    # values >1e8 are almost certainly bytes (≥100MB).
    if (( rss_kb > 100000000 )); then
      rss_mb=$(awk "BEGIN{printf \"%.1f\", $rss_kb / 1048576}")
    else
      rss_mb=$(awk "BEGIN{printf \"%.1f\", $rss_kb / 1024}")
    fi
  else
    rss_mb="NA"
  fi

  echo "$label,$subdir,$files,$wall,$user,$sys,$rss_mb,$call_edges" >> "$RESULTS"
  echo "[bench] $label $subdir files=$files wall=${wall}s rss=${rss_mb}MB call_edges=$call_edges"
  rm -f "$time_out"
}

build_baseline
build_optimised

for subdir in "${SUBDIRS[@]}"; do
  run_one baseline  "$BASELINE_BIN"  "$subdir" "$REPO_ROOT/examples/test_to_function_map.baseline.cozoql"
done
for subdir in "${SUBDIRS[@]}"; do
  run_one optimised "$OPTIMISED_BIN" "$subdir" "$REPO_ROOT/examples/test_to_function_map.optimised.cozoql"
done

echo
echo "[bench] done. Results in $RESULTS"
column -ts, "$RESULTS"
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x examples/bench_query_optimisations.sh
```

- [ ] **Step 3: Smoke-test the script with `--help`-equivalent (no args)**

```bash
./examples/bench_query_optimisations.sh 2>&1 | head -3
```
Expected: prints usage and exits 1.

- [ ] **Step 4: Commit**

```bash
git add examples/bench_query_optimisations.sh
git commit -m "bench: harness to compare master vs feat on openclaw subdirs

Cold-start runs through /usr/bin/time -lp; writes bench-results.csv
with wall, user, sys, RSS, and resolver call_edge_count per run.
Builds baseline binary by checking out master, then restores the
working branch."
```

---

### Task 7: Bench README

**Files:**
- Create: `examples/bench_query_optimisations.md`

- [ ] **Step 1: Write the README**

Create `examples/bench_query_optimisations.md`:
```markdown
# Bench: `test_to_function_map` query optimisations

Compares `master` vs `feat/test-to-function-map-optimisations` end-to-end
(cold build + query) over progressively larger openclaw subdirectories.

## Setup

1. Clone openclaw somewhere outside this repo:
   ```bash
   git clone --depth 1 https://github.com/openclaw/openclaw.git /tmp/openclaw
   ```
2. Survey subdir sizes to pick your 4 datapoints:
   ```bash
   find /tmp/openclaw -mindepth 1 -maxdepth 3 -type d \
     -exec sh -c 'printf "%6d  %s\n" "$(find "$1" -type f | wc -l)" "$1"' _ {} \; \
     | sort -n | tail -30
   ```
   Pick four subdirs whose file counts roughly hit 50 / 500 / 2000 / 5000.

## Run

From the repo root, on the feat branch:
```bash
./examples/bench_query_optimisations.sh /tmp/openclaw \
  Source/Foo Source/Bar Source/Baz Source
```

The script:
- Checks out master, builds, saves binary as `target/release/virgil-cli-baseline`
- Returns to your current branch, builds, saves as `target/release/virgil-cli-optimised`
- For each subdir × binary: wipes the project's SQLite cache, runs the
  query end-to-end under `/usr/bin/time -lp`, parses wall/user/sys/RSS,
  greps the resolver's `[bench] call_edge_count=N` line.

Output: `bench-results.csv` at the repo root.

## Reading the results

Columns: `binary, subdir, files, wall_s, user_s, sys_s, max_rss_mb, call_edge_count`.

- **Speedup** = baseline_wall / optimised_wall, per subdir.
- **CPU spread** = user_s / wall_s. >1.0 means the work used multiple
  cores; ≈1.0 means single-threaded.
- **call_edge_count** is your sanity check. If it's 0 on either binary,
  the comparison is invalid (likely a schema or extractor mismatch).

## Optional: warm-query-only run

Each scripted run is cold (cache wiped) and includes the build. To
isolate the query speedup from the build cost:

1. Comment out the `rm -rf "$CACHE_DIR"/*.sqlite` line in `run_one`.
2. Run the script twice — second run uses the warm cache.

The second run's wall time is roughly query-only.

## Why these particular changes?

See `docs/superpowers/specs/2026-05-25-test-to-function-map-optimisations-design.md`.
```

- [ ] **Step 2: Commit**

```bash
git add examples/bench_query_optimisations.md
git commit -m "docs: README for the test_to_function_map bench harness"
```

---

### Task 8: Run the bench and capture results

**Files:**
- Updated: `bench-results.csv` (gitignored — do not commit)

- [ ] **Step 1: Clone openclaw if not already present**

```bash
test -d /tmp/openclaw || git clone --depth 1 https://github.com/openclaw/openclaw.git /tmp/openclaw
```

- [ ] **Step 2: Survey subdir sizes**

```bash
find /tmp/openclaw -mindepth 1 -maxdepth 3 -type d \
  -exec sh -c 'printf "%6d  %s\n" "$(find "$1" -type f | wc -l)" "$1"' _ {} \; \
  | sort -n | tail -30
```

Pick 4 subdirs hitting ~50 / ~500 / ~2000 / ~5000 files. Record the
chosen paths in a comment in `bench-results.csv` (the script doesn't
record them).

- [ ] **Step 3: Run the bench**

```bash
./examples/bench_query_optimisations.sh /tmp/openclaw \
  <subdir-50> <subdir-500> <subdir-2000> <subdir-5000>
```

Expected: prints per-run progress lines, then a table dump from
`column -ts,`. Total runtime depends on the 5000-file subdir build —
expect 5–60 minutes on the largest point.

- [ ] **Step 4: Verify call_edge_count > 0 for every row**

```bash
awk -F, 'NR>1 && ($8=="0" || $8=="NA") {print "INVALID:", $0}' bench-results.csv
```
Expected: no output (every row has a valid count). If anything prints,
the bench is invalid for that point — investigate before reporting
speedups.

- [ ] **Step 5: Compute the speedup column**

```bash
awk -F, 'NR==1{print $0",speedup"; next} \
         $1=="baseline"{b[$2]=$4; print $0",1.00"; next} \
         $1=="optimised"{printf "%s,%.2f\n", $0, b[$2]/$4}' bench-results.csv \
  > bench-results-with-speedup.csv
column -ts, bench-results-with-speedup.csv
```

- [ ] **Step 6: Save the results into the spec folder**

```bash
cp bench-results-with-speedup.csv \
   docs/superpowers/specs/2026-05-25-test-to-function-map-optimisations-results.csv
git add docs/superpowers/specs/2026-05-25-test-to-function-map-optimisations-results.csv
git commit -m "data: bench results for test_to_function_map optimisations

Wall/user/sys/RSS for master vs feat across openclaw subdirs of
increasing file count. Speedup column added inline."
```

---

## Self-review

- **Spec coverage:** all four changes (regex→join, push filter ahead, persist call_edge at build, new index) are in Tasks 1, 3, and 5. Bench harness in Tasks 6–8. Equivalence + schema tests in Tasks 4 and 5. Branch/baseline strategy is followed (script checks out master, builds, returns).
- **Placeholders:** none. The `<subdir-N>` paths in Task 8 are intentional — they depend on the actual openclaw layout at clone time, which the script's survey step (8.2) reveals.
- **Type consistency:** `push_call_edge(caller_id, callee_id, file_path)` in Task 2 matches the resolver in Task 3 and the schema in Task 1. `file_classification{path: file, is_test: true}` in Task 5 matches the schema (`schema.rs:112`).
- **Known unknown flagged in plan:** the cross-file assertion in Task 4 may surface a gap in the Rust extractor's import handling for `use crate::...` syntax. Task 4 documents the fallback (switch fixture import shape) rather than silently relaxing the test.
