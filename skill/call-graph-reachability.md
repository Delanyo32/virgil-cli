# Call-Graph Reachability (who depends on what)

## The ask

Given a function name, find (a) everything that depends on it — directly
or indirectly — and (b) everything it depends on — directly or
indirectly. Two transitive-closure walks over the call graph, using only
virgil-cli's facts.

Both reduce to reachability over the materialised `call_edge` table
(`caller_id → callee_id`, populated by
`from_code_graph::resolve_and_emit_call_edges`):

- **(a) dependents = transitive callers.** Walk *up*: `callee_id →
  caller_id`. "Who reaches this function?"
- **(b) dependencies = transitive callees.** Walk *down*: `caller_id →
  callee_id`. "What does this function reach?"

## The queries

### (a) Transitive callers — everything that depends on `$name`

```sql
WITH RECURSIVE
seed AS (SELECT id FROM symbol WHERE name = $name AND kind IN ('function','method')),
up AS (
  SELECT e.caller_id AS fn, [e.callee_id, e.caller_id] AS path, 1 AS depth
  FROM call_edge e WHERE e.callee_id IN (SELECT id FROM seed)
  UNION ALL
  SELECT e.caller_id, list_append(u.path, e.caller_id), u.depth + 1
  FROM up u JOIN call_edge e ON e.callee_id = u.fn
  WHERE NOT list_contains(u.path, e.caller_id)   -- cycle guard
)
SELECT s.name, s.file_path, min(u.depth) AS nearest_depth
FROM up u JOIN symbol s ON s.id = u.fn
GROUP BY s.name, s.file_path
ORDER BY nearest_depth, s.name;
```

### (b) Transitive callees — everything `$name` depends on

Same walk, edges reversed:

```sql
WITH RECURSIVE
seed AS (SELECT id FROM symbol WHERE name = $name AND kind IN ('function','method')),
down AS (
  SELECT e.callee_id AS fn, [e.caller_id, e.callee_id] AS path, 1 AS depth
  FROM call_edge e WHERE e.caller_id IN (SELECT id FROM seed)
  UNION ALL
  SELECT e.callee_id, list_append(d.path, e.callee_id), d.depth + 1
  FROM down d JOIN call_edge e ON e.caller_id = d.fn
  WHERE NOT list_contains(d.path, e.callee_id)   -- cycle guard
)
SELECT s.name, s.file_path, min(d.depth) AS nearest_depth
FROM down d JOIN symbol s ON s.id = d.fn
GROUP BY s.name, s.file_path
ORDER BY nearest_depth, s.name;
```

Run: `projects query <name> --file dependents.sql --param name=getAccessToken`.

- **`path` is a list `[id]`, not a delimited string** — symbol ids are
  `path|line|col|name|kind` and already contain `|`, so a pipe-delimited
  cycle guard gives false matches. `list_contains` / `list_append` carry
  the visited set cleanly. Same choice the longest-data-paths skill makes.
  To *surface* a chain (swap `path` into the SELECT), render it in SQL the
  same way: `list_reduce(list_transform(u.path, x -> split_part(x,'|',4)),
  (a,b) -> a || ' -> ' || b)` → `getAccessToken -> isAuthenticated -> useAuth`.
- **`nearest_depth`** is the shortest hop distance; a node reachable by
  several paths is reported once at its closest, collapsed by `GROUP BY`.

## What the output looks like

(a) dependents of `getAccessToken` on `nextjs-dashboard`:

```
1  isAuthenticated   src/lib/auth.ts
2  useAuth           src/hooks/useAuth.ts
2  Home              src/pages/index.tsx
3  DashboardPage     src/pages/dashboard.tsx
3  Header / ReportsPage / SettingsPage / UsersPage   (depth 3)
```

Reads bottom-up: pages reach `getAccessToken` at depth 3, through
`useAuth → isAuthenticated`. Direction (b) on `useAuth` bottoms out at
`getAccessToken` (depth 2) — the same chain walked the other way.

## Limitations

- **Quality rides entirely on `call_edge`.** These queries add no
  resolution of their own — they inherit whatever the build-time resolver
  produced. Two facts about that resolver matter here:
  - **Caller attribution must be correct.** A call's `caller_id` is the
    enclosing function/method. (A fixed extractor bug used to pin it to a
    parameter on the signature line — on param-heavy corpora like Java that
    made >50% of edges originate at parameter nodes the walk can't traverse,
    silently fragmenting the graph. Re-run a cold build if results look
    empty on an old store.)
  - **Self-receiver calls are precise; named receivers depend on type info.**
    `this`/`self`/`$this->m()` resolve to the caller's own class. A named
    receiver like `order.save()` resolves precisely *when the receiver's type
    is known* — the schema-v4 type/parent funnel uses `local_type` (locals),
    parameter types, and field types to keep only candidates whose parent
    class is that type, so `category.getId()` no longer fans out to all 14
    `getId` methods. That type resolution is populated for **C#/Java/Python**
    only; for Go/Rust/JS/TS/PHP, or any receiver whose type can't be inferred,
    it still falls back to name-alone matching and over-counts. No edges are
    ever dropped below the name-based recall — the funnel only narrows.

- **Name-based resolution under-links too.** Dynamic dispatch, callbacks,
  and unresolved cross-file imports produce no edge — so the closure is an
  *under*-estimate in those directions and an *over*-estimate through name
  collisions. Both at once.

- **`$name` may be ambiguous.** Two functions can share a name; `seed`
  picks up all of them and the closures merge. Add `AND file_path = $file`
  to disambiguate, or seed on the stringly id directly.

- **Duplicate-looking rows are real.** Same method name in two
  classes/files appears as two rows (grouped by `(name, file_path)`).
  Correct, just noisy — group by `s.id` to merge.

- **Cycles are cut, not reported.** The visited-list guard stops at the
  first repeat, so a recursive sub-path contributes its acyclic prefix
  only. Use `find_cycles` to enumerate the cycles themselves.

- **Sparse for Java/C#, dense for JS/TS.** `call_edge` recall tracks call
  resolution quality — pair with `occurrence` (`occurrence_kind='call'`) if
  you need higher recall on overload-heavy languages. See
  `data-entry-points.md`.
