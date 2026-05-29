# Longest Data Paths (input → output)

## The ask

List the longest paths from where data enters a codebase to where it
leaves — ordered by length — using only virgil-cli's facts.

"Longest path from input to output" reduces to the longest **call chain**
over the `call_edge` table:

- **Source (data in):** a function nothing internal calls but which calls
  out — a call-graph root. The framework/runtime invokes it (HTTP handler,
  React page, `main`). Same definition as `data-entry-points.md` query 1.
- **Sink (data out):** a function that calls nothing internal — a leaf.
  That's where the chain bottoms out into a boundary call (`res.json`,
  `fetch`, a DB driver).
- **Path length:** number of internal call hops between them.

So it's a longest-path walk: seed at every source, follow `call_edge`,
keep only paths ending at a sink, order by hops.

## The query

### 1. Longest source→sink call chains — the path finder

```sql
WITH RECURSIVE
src AS (   -- call-graph roots that actually call out (data-entry points)
  SELECT s.id
  FROM symbol s
  WHERE s.kind IN ('function','method')
    AND NOT EXISTS (SELECT 1 FROM call_edge e WHERE e.callee_id = s.id)
    AND EXISTS     (SELECT 1 FROM call_edge e WHERE e.caller_id = s.id)
),
walk AS (
  SELECT id AS src, id AS node, 0 AS hops, [id] AS path
  FROM src
  UNION ALL
  SELECT w.src, e.callee_id, w.hops + 1, list_append(w.path, e.callee_id)
  FROM walk w
  JOIN call_edge e ON e.caller_id = w.node
  WHERE NOT list_contains(w.path, e.callee_id)   -- cycle guard
    AND w.hops < 50                              -- runaway cap
)
SELECT src_s.name AS source, sink_s.name AS sink, w.hops, w.path
FROM walk w
JOIN symbol src_s  ON src_s.id  = w.src
JOIN symbol sink_s ON sink_s.id = w.node
WHERE NOT EXISTS (SELECT 1 FROM call_edge e WHERE e.caller_id = w.node)  -- node is a sink
ORDER BY w.hops DESC
LIMIT 15;
```

Run with `projects query <name> --file longest_data_paths.sql`. Reverse
direction (output→input) is the same walk with `caller_id`/`callee_id`
swapped.

`path` is a list of stringly symbol ids (`path|line|col|name|kind`).
Resolve to readable names by splitting on `|` and taking field 4, e.g.
`Header → useAuth → isAuthenticated → getAccessToken`.

### 2. Two implementation choices that bite — keep them

- **Carry the path as a list `[id]`, not a delimited string.** Symbol ids
  are `path|line|col|name|kind` — they already contain `|`, so a
  pipe-delimited cycle guard gives false matches. `list_contains` /
  `list_append` sidestep it.
- **Recursive CTE, not PGQ.** duckpgq's `GRAPH_TABLE` can't be wrapped in
  `WITH` (crashes), and unbounded `->*` needs an explicit `ACYCLIC` mode
  anyway — `find_cycles` already chose the recursive-CTE route for the same
  reason.

## What the output looks like

On `nextjs-dashboard` (a bench extract), top hits:

```
hops=3  Header      -> useAuth -> isAuthenticated -> getAccessToken
hops=3  SettingsPage-> useAuth -> isAuthenticated -> getAccessToken
hops=3  DashboardPage-> useAuth -> isAuthenticated -> getAccessToken
hops=2  DashboardPage-> computeStats -> calculatePercentage
```

The top hits all funnel into the same auth chain — every page reaches
`getAccessToken` through `useAuth → isAuthenticated`, crossing
`Header.tsx → useAuth.ts → auth.ts`. That fan-in is the real signal: the
longest data paths in this app all run through auth resolution.

## Limitations

- **Chains are short because resolution is sparse.** Max depth was 3 on
  the bench corpora (call graphs of 24–89 edges). Name-based call
  resolution (no type info) misses dynamic dispatch, callbacks, and method
  calls it can't bind — so the true longest data path is an *under*-estimate.

- **"Source" ≠ guaranteed input, "sink" ≠ guaranteed output.** virgil has
  no taint facts (`parameter.is_taint_source` is always `false`). A leaf
  might be a pure helper, not an I/O sink; confirm the top hits by reading
  them, like the Big-O skill says. See `data-entry-points.md` for why
  direction (source vs sink) can't be told from facts alone.

- **Cross-file / anonymous calls split the chain.** Inline
  `(req,res)=>…` handlers and unresolved calls break a long path into
  fragments — each reads shorter than reality.

- **Cycles are cut, not reported.** The guard stops at the first repeat, so
  a recursive sub-path contributes its acyclic prefix only. Use
  `find_cycles` to enumerate the cycles themselves.

- **All simple paths are enumerated.** The walk emits every acyclic
  source→node path, not just the longest per pair — the `hops < 50` cap and
  `LIMIT` bound it. On a dense, deep call graph this can blow up; tighten
  the cap or filter `src`/sink sets first.
