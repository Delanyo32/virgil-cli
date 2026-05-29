# Cataloguing Big-O Candidates

## The ask

Catalogue the algorithmic complexity (Big-O) of functions in a codebase
and surface the heaviest ones — using only virgil-cli's facts.

virgil has **no Big-O computation** and can't have one: Big-O needs loop
*bounds*, recursion depth, and data-structure semantics that static facts
don't carry. What it can give is a **structural proxy** — loop-nesting
depth — which approximates the polynomial degree: depth 1 ≈ O(n), depth 2
≈ O(n²), depth 3 ≈ O(n³). The query produces a *candidate list*; a human
confirms the actual bound by reading the top hits.

Two false starts worth recording, because both are tempting and both are
wrong:

- **Cyclomatic complexity** (`complexity_hotspots` template) ranks by
  branch count, not loops. On a real C++ corpus its #1 hit (`run_pipeline`,
  cyclomatic 31) is **O(n)** — a flat arg-parser — while the true O(n³)
  triple-loop sat mid-list. High branchiness ≠ high Big-O.
- **Block-nesting** (counting any `{}` scope) conflates `if` nesting with
  loop nesting. It ranked a single-loop O(n) function *above* a
  triple-nested-loop O(n³) one, because the former was wrapped in 6 `if`s.

The fix that made this query possible: `scope.kind` for body blocks now
holds the owning tree-sitter construct (`for_statement`, `while_statement`,
…) instead of a generic `"block"`, so loops are distinguishable from
branches with **no Rust-side analysis** — all logic stays in SQL.

## The queries

### 1. Loop-nesting rank — the candidate finder

Walk the `scope.parent_id` chain from each function, counting only loop
constructs. `MAX(depth)` per function is its deepest loop nest.

```sql
WITH RECURSIVE walk AS (
  SELECT id AS fn, id AS node, file_path, 0 AS depth FROM scope WHERE kind='function'
  UNION ALL
  SELECT w.fn, c.id, c.file_path,
    w.depth + CASE WHEN c.kind IN
      ('for_statement','for_in_statement','for_of_statement','while_statement',
       'do_statement','for_range_loop','foreach_statement','enhanced_for_statement',
       'for_expression','while_expression','loop_expression') THEN 1 ELSE 0 END
  FROM walk w JOIN scope c ON c.parent_id = w.node
),
d AS (SELECT fn, file_path, MAX(depth) AS loop_nesting FROM walk GROUP BY fn, file_path),
fs AS (SELECT id, file_path, start_byte, end_byte FROM scope WHERE kind='function')
SELECT s.name, d.file_path, sp.start_line, d.loop_nesting
FROM d JOIN fs ON fs.id = d.fn
JOIN span sp ON sp.file_path = d.file_path
  AND fs.start_byte >= sp.start_byte AND fs.end_byte <= sp.end_byte
JOIN symbol s ON s.id = sp.entity_id AND s.kind IN ('function','method')
WHERE d.loop_nesting > 0
ORDER BY d.loop_nesting DESC LIMIT 20;
```

The `IN (...)` list is the loop vocabulary across all 10 grammars — the
one piece of per-language knowledge, and it lives in the *query*, not the
app. Branches (`if_statement`, `else_clause`, `switch_*`, `catch_clause`)
are deliberately excluded, so they don't inflate the count.

### 2. What loops exist, and where — sanity / breakdown

Confirms the corpus actually parsed loops before trusting query 1, and
shows the per-construct mix.

```sql
SELECT kind, COUNT(*) n FROM scope
WHERE kind LIKE '%for%' OR kind LIKE '%while%' OR kind LIKE '%loop%'
GROUP BY kind ORDER BY n DESC;
```

### 3. Confirm a hit is a real nested loop, not an artifact

The function→span join can be fuzzy; before reporting an O(n²)+ finding,
read it. Get its location from query 1, then open the source. A genuine
hit looks like `for (...) { for (...) { ... } }`; a false one is a single
loop the depth walk mis-attributed.

## Limitations

- **Bounded loops still count.** `for (j=0; j<256; j++)` is O(1) but adds
  a nesting level. Depth is a *structural* signal; the loop bound is not a
  fact. Confirm bounds by reading the top hits — a depth-3 function whose
  inner loop is a fixed 256 is really O(n²).

- **Recursion is invisible.** An O(2ⁿ) recursive function has loop_nesting
  0. Recursive complexity lives in the call graph (`call_edge` self-loops /
  cycles), not in scope nesting. This query will not find it.

- **Cross-function loops are split.** A loop calling a helper that itself
  loops (O(n²) spread across two functions) reads as depth 1 in each.
  Inlining via `call_edge` would be needed to see the composite.

- **Brace-less single-statement loops emit no scope.** `for (...) g();`
  with no `{}` produces no body block in C/C++/TS, so it's invisible to
  this query. Pre-existing extractor behaviour. (Python/Rust/Go always
  brace, so unaffected.)

- **TS/JS loops are counted at the construct, not the body.** `for_*`
  opens its own scope (it holds the loop variable); its body block is kept
  neutral (`kind='block'`) to avoid double-counting. `while`/`do` aren't
  scoped at the construct, so their body block carries the kind. Both paths
  yield exactly +1 per loop — verified on a real nested TS loop reporting
  depth 2, single loops depth 1.

- **Function-body blocks carry an odd kind.** In most languages the body
  block of a function reports its parent verbatim (e.g.
  `kind='function_definition'`) rather than `'function'`. Harmless here —
  it isn't a loop kind — but don't mistake it for control flow.

- **Loop vocabulary is grammar-specific.** The construct names differ per
  language (`for_range_loop` C++, `enhanced_for_statement` Java,
  `for_in_statement` TS for both `in`/`of`, `loop_expression` Rust). The
  query's `IN (...)` list must stay in sync with the grammars; build a
  normalised SQL view if you want a portable `loop`/`branch` vocabulary.
