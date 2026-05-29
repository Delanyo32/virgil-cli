# Ranking by References (hubs and hot spots)

## The ask

Rank (a) functions and (b) files by how heavily they're referenced —
the dependency hubs of a codebase — using only virgil-cli's facts.

"Reference" has two honest readings, and they disagree, so pick
deliberately:

- **Distinct callers** — how many *different* sites reach it. Id-resolved,
  clean, but a caller that calls a function 5× counts once.
- **Total uses** — every call expression, repeats included. Higher recall,
  but name-based, so name collisions over-count.

Functions rank over the call graph (`call_edge` / `occurrence`); files
rank over imports (`imports`) or cross-file calls (`call_edge`).

## The queries

### (a1) Functions by distinct callers — `call_edge`

```sql
SELECT s.name, s.file_path, count(*) AS refs
FROM symbol s
JOIN call_edge e ON e.callee_id = s.id
WHERE s.kind IN ('function','method')
GROUP BY s.id, s.name, s.file_path
ORDER BY refs DESC;
```

`call_edge`'s PK is `(caller_id, callee_id)`, so each row is one
*distinct* caller→callee pair — `count(*)` is distinct callers, not total
call sites. Id-resolved: no name-collision noise.

### (a2) Functions by total call sites — `occurrence`

```sql
SELECT s.name, s.file_path, count(*) AS refs
FROM symbol s
JOIN occurrence o ON o.name = s.name AND o.occurrence_kind = 'call'
WHERE s.kind IN ('function','method')
GROUP BY s.id, s.name, s.file_path
ORDER BY refs DESC;
```

Counts every call expression. But `occurrence` has no `callee_id` — the
join is **by name only**, so two same-named methods in different classes
share a count. Trade precision for recall. Same complementarity the
`data-entry-points.md` skill relies on (`call_edge` dense for JS/TS,
`occurrence` higher recall elsewhere).

### (b1) Files by imported-by count — `imports`

```sql
SELECT imported_id AS file, count(DISTINCT importer_file_id) AS importers
FROM imports
GROUP BY imported_id
ORDER BY importers DESC;
```

**No join to `symbol`.** Despite the PGQ DDL declaring
`imports DESTINATION KEY (imported_id) REFERENCES symbol (id)`, the
column actually holds a **file path** (`src/lib/auth.ts`), not a symbol
id. Joining `imported_id = symbol.id` matches nothing and silently
returns zero rows. `SELECT imported_id FROM imports LIMIT 3` confirms it.
Drop `DISTINCT` to count total import edges instead of distinct importers.

### (b2) Files by incoming cross-file calls — `call_edge`

```sql
SELECT callee.file_path AS file, count(*) AS incoming_calls
FROM call_edge e
JOIN symbol callee ON callee.id = e.callee_id
JOIN symbol caller ON caller.id = e.caller_id
WHERE caller.file_path <> callee.file_path   -- only cross-file
GROUP BY callee.file_path
ORDER BY incoming_calls DESC;
```

Counts call edges landing on symbols defined in the file, from a *different*
file. A behaviour-level hub measure, complementary to (b1)'s import-level one.

Run any of them with `projects query <name> --file refs.sql` (or `--sql '…'`).

## What the output looks like

On `nextjs-dashboard` (55 ts/tsx files):

**(a1) distinct callers** — `cacheSet` 6, `useAuth` 5, `get` 5,
`apiRequest` 4, `createEntry` 4, `formatCurrency` 2.

**(a2) call sites** — same shape, counts climb: `get` jumps to 8,
`checkPermission` (4) surfaces where (a1) had it lower. Recall up,
collision risk up.

**(b1) imported-by** and **(b2) cross-file calls**:

```
b1 importers          b2 incoming_calls
6  src/lib/api.ts      10  src/lib/api.ts
5  src/types/api.ts     6  src/lib/auth.ts
5  src/types/common.ts  5  src/lib/validators.ts
5  src/hooks/useAuth.ts 5  src/hooks/useAuth.ts
4  src/components/Layout.tsx
```

`src/lib/api.ts` tops both lists — the real hub. Note the disagreement:
type-only files (`src/types/api.ts`) rank high on imports but are absent
from cross-file calls — they're imported for types, never *called*. Run
both to tell a config/type hub from a behaviour hub.

## Limitations

- **Two readings disagree on purpose.** (a1)/(a2) and (b1)/(b2) measure
  different things. A symbol high in one and low in the other is signal,
  not error — read the gap (type import vs runtime call).

- **`occurrence` and the cross-file-call variant over-link on names.**
  (a2) joins by name, so same-named methods merge. `call_edge` (a1) and
  `imports` (b1) are id/path-resolved and clean. Prefer the resolved ones
  for a ranking; use the name-based ones for raw usage volume.

- **Rankings ride on call/import resolution quality.** Dense for JS/TS,
  sparse for Java/C# (overloads, generics, dynamic dispatch produce no
  edge) — under-counts there. Same caveat as every call-graph skill here.

- **`imports.imported_id` is a file path, not a symbol id** — see (b1).
  The DDL's `REFERENCES symbol (id)` is misleading; trust the data, not
  the declaration.

- **Empty results on an old store?** Cold-rebuild (`--rebuild`). The
  caller-attribution fix changed `call_edge` shape; stale stores
  under-report.
