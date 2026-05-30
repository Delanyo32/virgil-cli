# Finding Data Entry Points

## The ask

Find every place where external data enters a codebase — HTTP handlers,
DB reads, file/env/network input — using only virgil-cli's facts. No
language-specific source lists if we can avoid them.

virgil has **no taint analysis** (`parameter.is_taint_source` exists but
is always `false`). So "where does data enter" is answered structurally
from three fact tables: `call_site`, `symbol`, and `call_edge` / `occurrence`.

## The queries

### 1. Entry-point handlers — defined but never called internally

The core idea: an entry point is a function the app *defines* but nothing
in the app *calls*. The framework/runtime/OS invokes it. Its parameters
are the data-entry surface. No name list.

```sql
SELECT s.name, s.file_path
FROM symbol s
WHERE s.kind IN ('function','method') AND s.exported
  AND NOT EXISTS (SELECT 1 FROM call_edge ce  WHERE ce.callee_id = s.id)
  AND NOT EXISTS (SELECT 1 FROM occurrence o WHERE o.name = s.name
                                              AND o.occurrence_kind = 'call')
ORDER BY s.file_path;
```

Measure "called" with **both** `call_edge` and `occurrence` — they're
complementary (`call_edge` is dense for JS, `occurrence` for Java).

### 2. Boundary objects — fully list-free

What objects does the app talk to across its boundary? `receiver` (the
object a call is made on) comes from the grammar; "external" comes from
the app's own symbol table.

```sql
SELECT cs.receiver, cs.callee_name, count(*) n
FROM call_site cs
LEFT JOIN symbol s ON s.name = cs.callee_name   -- defined in app?
WHERE cs.receiver IS NOT NULL                    -- called on an object
  AND s.id IS NULL                               -- callee external = crosses boundary
GROUP BY cs.receiver, cs.callee_name
ORDER BY n DESC;
```

Ranks every boundary object: DB models (`Post`, `User`), DB/IO layers
(`db`, `fs`, `crypto`), HTTP wiring (`router`, `app`), response sinks (`res`).

### 3. Framework-tagged handlers — crisp where annotations exist

For Java/C#, the framework declares entry points via annotations. This is
the framework's own contract, not a guessed list, and it tags the
receiving parameter (`@RequestBody`, `[FromBody]`).

```sql
-- Java; swap java_attrs.annotations -> csharp_attrs.attributes for C#
SELECT s.name, a AS marker, s.file_path
FROM symbol s JOIN java_attrs j ON j.symbol_id = s.id, UNNEST(j.annotations) t(a)
WHERE regexp_matches(a, 'Mapping|RestController|RequestBody|RequestParam|PathVariable');
```

## Limitations

- **No source/sink direction.** Query 2 lists every boundary object but
  can't tell a *source* (`Post.find`, data in) from a *sink* (`res.json`,
  data out) from a *utility* (`Math.max`). virgil has no fact for "this
  external call returns data." Read it by the receiver name + ranking.

- **Recall depends on call resolution.** Query 1's precision rides on how
  well `call_edge` / `occurrence` capture calls. Dense for JS/TS, sparse
  for Java (overloads/generics) — so Java needs query 3 (annotations) to
  be crisp.

- **`exported` over-selects.** It catches all real handlers (good recall)
  but also exception classes, components, helpers (poor precision).
  Annotations (query 3) are precise; `exported` is the fallback.

- **Property-access sources are invisible.** `process.env` / `req.body`
  are not *calls*, so they never appear in `call_site`. Catch the
  request-style ones via parameter names (`req`, `event`, `ctx`).

- **Receiver coverage is grammar-dependent.** Captured for JS/TS, Python,
  Go, C#, Rust, C++, Java, PHP. C is ~0 (free-function calls, no
  receiver). Chained calls give the immediate receiver (`res.status(500)`),
  not the root.

- **Anonymous inline handlers are missed.** `app.get('/x', (req,res)=>…)`
  isn't a named exported symbol; query 1 won't see it. Fall back to
  receiver = `req`/`res` parameter names.
