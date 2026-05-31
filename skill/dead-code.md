# Dead Code (functions defined but never used)

## The ask

Find functions/methods that are **defined but reached from nowhere** — dead
code — using only virgil-cli's facts.

The naive query is a trap, and it's the one everyone writes first:

```sql
-- DON'T: flags ~86% of a codebase as "dead"
SELECT name FROM symbol s WHERE kind IN ('function','method')
  AND NOT EXISTS (SELECT 1 FROM call_edge e WHERE e.callee_id = s.id);
```

On `express-api` (213 functions) this returns **183**. It's useless, for two
reasons that define the whole problem:

1. **`call_edge` is the *resolved* layer, not the *used* layer.** A call only
   becomes a `call_edge` row if the resolver linked it to a workspace symbol.
   Unresolved-but-real calls (and every builtin/library call) have no edge, so
   "no incoming edge" ≠ "unused". You must measure *use*, not *resolution*.
2. **"Used" has three shapes, and they live in three tables:**
   - **called** — `call_site.callee_name` (covers `foo()` *and* `obj.m()` /
     `self.m()`; this is the one `occurrence` misses).
   - **referenced** — `occurrence` of the name (covers being *passed*, e.g.
     `app.use(handler)`, `router.post('/x', ctrl.login)` — a read, not a call).
   - **an entry point** — invoked from *outside* the workspace graph
     (frameworks, the runtime, the test harness, the OS). These are never
     "called" in-repo and must be **excluded**, not flagged.

So dead = defined, **not called** (`call_site`), **not referenced outside its
own body** (`occurrence` + span), and **not an entry point** (exclusions).

## The queries

### 1. Dead code — the three-signal query

```sql
WITH defspan AS (
  SELECT s.id, s.name, sp.file_path, sp.start_byte, sp.end_byte
  FROM symbol s JOIN span sp ON sp.entity_id = s.id
  WHERE s.kind IN ('function','arrow_function','method')
    AND NOT s.exported                              -- exported = undecidable (see Limitations)
    AND NOT starts_with(s.name, '__')               -- dunders (__init__, __repr__) are runtime-called
    AND NOT starts_with(s.name, 'test_')            -- test harness invokes these
    AND s.name NOT IN ('main','configure','doFilterInternal','setUp','tearDown','run')
)
SELECT d.name AS dead_function, d.file_path
FROM defspan d
LEFT JOIN file_classification fc ON fc.path = d.file_path
WHERE coalesce(fc.is_test, false) = false           -- skip test files
  AND NOT EXISTS (                                   -- not called (incl. obj.m() / self.m())
    SELECT 1 FROM call_site cs WHERE cs.callee_name = d.name)
  AND NOT EXISTS (                                   -- not referenced outside its own def span
    SELECT 1 FROM occurrence o
    WHERE o.name = d.name
      AND NOT (o.file_path = d.file_path
               AND o.start_byte >= d.start_byte AND o.start_byte <= d.end_byte))
ORDER BY d.file_path, d.name;
```

Run: `projects query <name> --file dead_code.sql`.

The three `NOT EXISTS` are the whole design:
- **`call_site`** catches calls — including `obj.method()` / `self.method()`,
  which `occurrence` does **not** record. Drop this clause and live methods
  like `self._recalculate()` or `s.sendEmail()` get false-flagged.
- **`occurrence` … outside the def span** catches *reference-passing* — a
  handler passed to `app.use(...)` is a `read` of the name, not a call. The
  span guard excludes the symbol's own definition occurrence (and self-recursion)
  so a function isn't kept alive by merely existing.
- The `defspan`/`fc` filters strip the **entry-point classes** the runtime
  calls from outside the graph.

### 2. Same, but only the *confident* core — private + zero references

For the lowest false-positive rate, judge **only non-exported** symbols and
require **zero** references of any kind outside their own body:

```sql
WITH defspan AS (
  SELECT s.id, s.name, sp.file_path, sp.start_byte, sp.end_byte
  FROM symbol s JOIN span sp ON sp.entity_id = s.id
  WHERE s.kind IN ('function','arrow_function','method') AND NOT s.exported
)
SELECT d.name, d.file_path
FROM defspan d
WHERE NOT EXISTS (SELECT 1 FROM call_site cs WHERE cs.callee_name = d.name)
  AND NOT EXISTS (
    SELECT 1 FROM occurrence o WHERE o.name = d.name
      AND NOT (o.file_path = d.file_path
               AND o.start_byte >= d.start_byte AND o.start_byte <= d.end_byte));
```

This is query 1 without the entry-point name/test exclusions — use it when you
want to *see* the entry-point noise (overrides, `__init__`, `main`) and tune
the exclusion list per framework, rather than trust a pre-baked one.

## What the output looks like

The progression on `express-api` (55 js) is the lesson — same codebase, three
queries:

```
naive call_edge-only        183 / 213   ← useless (measures resolution, not use)
+ call_site name check        2          ← corsMiddleware (dead) + errorHandler (false)
+ occurrence/span + exclude   small, mostly-real candidate lists
```

**(1) three-signal query**, across corpora (candidate counts):

```
spring-api   (Java)   0
systems-cli  (Rust)   0
laravel-store(PHP)    1   redirectTo
technical-debt(Py)    2   _get_models, _worker_loop
http-service (Go)     5   getEnvBool, calculateDiscount, retryPayment,
                          collectResults, logQueueEvent
dotnet-api   (C#)    18   incl. EF migration Up/Down (false — see Limitations)
```

Source-verified true positives on `http-service`: `calculateDiscount`,
`retryPayment`, `getEnvBool` — each appears **only** in its own definition and
a doc comment, nowhere else. Real dead code.

The remaining `dotnet-api` noise (`Up`/`Down` in `Data/Migrations/*.cs`) is an
EF Core entry-point class the framework invokes — add it to the exclusion list
and the list tightens.

## Limitations

- **Exported symbols are undecidable.** An exported function may be called by
  an external consumer the workspace can't see, so `NOT s.exported` is required
  — and it means the query says **nothing** about a library's public API. On an
  app with everything `module.exports`-ed (`express-api`), almost nothing is
  judged, which is honest, not a bug. For a closed app, treat exported
  entry-handlers via `data-entry-points.md` instead.

- **Entry points are an open-ended exclusion list.** Every framework calls code
  from outside the graph in its own way — Spring `@Override` lifecycle
  (`doFilterInternal`), EF migrations (`Up`/`Down`), route handlers, Python
  dunders, `main`, `test_*`. The baked-in list catches the common ones; a new
  framework needs a new exclusion. Read query 2's raw output to find the class
  before trusting query 1.

- **`occurrence` does not record `obj.method()` call names — `call_site` does.**
  This is why both clauses are mandatory. An `occurrence`-only query
  false-flags every method reached through a receiver (`self.m()`, `s.m()`);
  verified on `sendEmail` (5 uses) and `_recalculate` (3 uses), both wrongly
  flagged until `call_site` was added.

- **Name-based, so collisions hide deaths.** Both checks match on `name`, not
  id. A dead `save` is kept alive by a live `save` in another class. Under-reports
  (false negatives), never the reverse — acceptable for a "candidate" list.

- **Dynamic dispatch is invisible.** PHP `$this->$method()`, Python `getattr`,
  reflection, and string-keyed dispatch produce no `call_site`/`occurrence` of
  the name → real uses missed → occasional false positives (e.g. PHP service
  methods called via a resolver map).

- **It's a candidate list, not ground truth.** The output is sized for a human
  to eyeball (≤ ~20 rows per corpus here), not to feed an automated deleter.
  Confirm each against source — the verified Go three took one `grep` each.

- **Stale stores under-report.** The schema-v4 call-resolution changes (local
  types, type/parent funnel, CommonJS object-exports) changed what lands in
  `call_edge`/`call_site`. On an old store the lists will be noisier —
  cold-rebuild with `--rebuild`.
