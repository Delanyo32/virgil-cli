# Internal vs external dependencies (your code vs library code)

## The ask

Split a codebase's imports into **external** (libraries, stdlib, vendor)
vs **internal** (its own modules), and find which files are
library-facing glue versus pure internal logic — across all 9 languages,
using only virgil-cli's facts.

Two honest readings, and they answer different questions, so pick
deliberately:

- **Statement-level** — over `raw_import` (one row per `import`/`use`/
  `#include`/`using`). Tells you *which libraries* and *how heavily*. The
  external/internal split here is a **language-specific specifier
  heuristic** (relative path, module prefix, namespace shape).
- **Edge-level** — over `imports` (resolved file→file dependency edges).
  Resolution-aware and clean, and now populated for **all** languages, not
  just TS/JS. Use it for internal coupling and for "is this file
  library-facing." By construction it holds **only internal edges** — it
  can't tell you anything about external packages.

## The queries

### (1) External surface — `raw_import` (statement-level, per-language rule)

"External" at the statement level is the import's specifier shape, and the
rule differs by language (it's the same logic each resolver uses):

| language | internal specifier | external specifier |
|---|---|---|
| ts/js/jsx/tsx | `./`, `../`, `/` | bare (`mongoose`) |
| go | module-path prefix (from `go.mod`) | dotted domain = third-party; else stdlib |
| python | top package is a workspace module | everything else (`os`, `numpy`) |
| java | namespace maps to a workspace file | `java.*` / `javax.*` / `org.*` not in tree |
| php | PSR-4 namespace maps to a file | vendor (`Illuminate\…`) |
| c/c++ | quoted include resolvable under an include root | `<...>` system header |
| rust | `crate::`/`self::`/`super::`/bare path under `src/` | `std`, extern crate |
| c# | `using` namespace declared by a workspace file | `System.*` / `Microsoft.*` |

**TS/JS archetype** — relative path = internal, so external is the inverse:

```sql
SELECT raw_path, count(*) AS n
FROM raw_import
WHERE raw_path NOT LIKE './%'
  AND raw_path NOT LIKE '../%'
  AND raw_path NOT LIKE '/%'
GROUP BY raw_path
ORDER BY n DESC;
```

**Go archetype** — everything looks absolute, so internal is the module
prefix and external splits into third-party (has a dotted domain) vs
stdlib:

```sql
SELECT CASE WHEN raw_path LIKE '%.%/%' THEN 'thirdparty' ELSE 'stdlib' END AS cls,
       raw_path, count(*) AS n
FROM raw_import
WHERE raw_path NOT LIKE 'github.com/example/ordersvc/%'   -- the module path
GROUP BY cls, raw_path
ORDER BY n DESC;
```

There is **no universal SQL** for the external split — that workspace
dependence is the whole reason each language needs its own resolver. Swap
the `WHERE` per the table above.

### (2) Per-file dependency profile — resolution-aware, all languages

Raw import count vs resolved internal edges, per file. This one **is**
language-agnostic because it reads the resolved `imports` table:

```sql
SELECT ri.file_path,
       count(*) AS raw_imports,
       (SELECT count(*) FROM imports im
        WHERE im.importer_file_id = ri.file_path) AS internal_edges
FROM raw_import ri
GROUP BY ri.file_path
ORDER BY internal_edges DESC, raw_imports DESC;
```

High `internal_edges` = an internal hub wired into the rest of the app.
`raw_imports` well above `internal_edges` = a file that leans on libraries.

### (3) Project split — resolution-aware

```sql
SELECT (SELECT count(*) FROM raw_import) AS raw_imports,
       (SELECT count(*) FROM imports)    AS internal_edges;
```

The gap is **not** a clean external count (dedup + fan-out, see
Limitations) — read it as "how internally-wired is this codebase," not
"raw minus internal."

### (4) Library-facing leaves — files with deps but zero internal edges

The clean, all-language way to find pure library adapters (a file that
imports things but none of them are workspace modules — models wrapping an
ORM, a logger, a thin client):

```sql
SELECT ri.file_path, count(*) AS raw_imports
FROM raw_import ri
WHERE NOT EXISTS (
    SELECT 1 FROM imports im WHERE im.importer_file_id = ri.file_path
)
GROUP BY ri.file_path
ORDER BY raw_imports DESC;
```

Invert it (`EXISTS`) for the internally-wired files; the ones with the
highest `internal_edges` in (2) are your business-logic hubs.

### (cross-check) Does the heuristic agree with resolution?

For a relative-path language the statement heuristic and the resolved edge
count should match — a quick trust check that resolution is healthy:

```sql
SELECT (SELECT count(*) FROM raw_import
        WHERE raw_path LIKE './%' OR raw_path LIKE '../%') AS heuristic_internal,
       (SELECT count(*) FROM imports) AS resolved_edges;
```

Run any of them with `projects query <name> --file deps.sql` (or `--sql '…'`).

## What the output looks like

**(1-ts) external surface** — `express-api` (55 js): `mongoose` 12,
`express` 8, `bcryptjs` 4, `jsonwebtoken` 3, plus stdlib `fs`/`path` 3
each. The library coupling is concentrated in the ORM and auth.

**(1-go) external surface** — `http-service` (48 go): all top entries are
**stdlib** (`time` 31, `fmt` 27, `log` 18, `net/http` 11) with a single
third-party dep (`github.com/lib/pq`) in the whole tree. The numbers
invert vs JS: Go leans on its stdlib, not a package ecosystem.

**(2) per-file profile** — `express-api`:

```
file                                  raw_imports  internal_edges
src/controllers/postController.js     10           10     ← pure internal hub
src/app.js                            14            9     ← glue (libs + wiring)
src/controllers/authController.js      9            6
src/services/postService.js            5            5     ← pure internal
src/controllers/mediaController.js     6            4
```

**(3) project split** — resolution now works everywhere:

```
project          raw_imports  internal_edges
express-api      124          77
http-service     170          155
spring-api       351          75
technical-debt   569          58
dotnet-api       276          339   ← edges > raw (C# namespace fan-out)
systems-cli      111          30
laravel-store    161          70
```

**(4) library-facing leaves** — `express-api`: `services/mediaService.js`
(4 deps, 0 internal), `utils/fileUpload.js` (3/0), and every `models/*.js`
(1/0 — each wraps only `mongoose`). These are the adapter layer.

**(cross-check)** — `express-api`: `heuristic_internal=77`,
`resolved_edges=77`. They agree exactly, so the relative-path heuristic
and the resolver tell the same story.

## Limitations

- **The two readings answer different questions.** (1) is per-statement
  and tells you which libraries; (2)/(4) are per-edge and tell you which
  files are internally wired. A file heavy in raw imports but with zero
  internal edges is signal (a library adapter), not error.

- **No clean universal "external count."** `internal_edges` ≠ "internal
  imports" exactly, for two reasons:
  - **dedup** — `imports`' PK is `(importer_file_id, imported_id)`, so a
    file importing the same module twice collapses to one edge.
  - **fan-out** — a Go package import, a C# `using`, or a Java wildcard
    resolves to *every file* in the package/namespace, so one `raw_import`
    yields many edges. `dotnet-api` shows it: 276 raw → 339 edges. So
    **do not compute external = raw − internal_edges** for those
    languages. Use the (1) heuristic for an external count.

- **The (1) external split is language-specific.** There is no
  workspace-free SQL for it — that's exactly why each language has its own
  resolver. Use the rule table; the `WHERE` clause changes per language.

- **`imports.imported_id` is a file path, not a symbol id** — same gotcha
  as `references-ranking.md` (b1). Don't join it to `symbol.id`; it holds
  `src/models/Post.js`, not an id.

- **`imports` holds only internal edges.** External packages never produce
  a row (they don't resolve to a workspace file). You can't list libraries
  from `imports` — only from `raw_import`.

- **Stale stores under-report.** The cross-language resolver fix changed
  what lands in `imports` (it was empty for Go/Java/Python/C/C++/PHP/C#
  before, and partial for Rust). If (2)/(3)/(4) come back empty or
  TS/JS-only on an existing store, cold-rebuild with `--rebuild`.

- **Resolution is heuristic, not a compiler.** Go/Java/Python/PHP/Rust/C#
  match against the workspace file set (suffix/namespace/module rules), so
  a name collision can mis-link and unusual layouts (custom source roots,
  multi-module repos) can miss. TS/JS and the relative cases are the most
  precise. JS/TS call/import density is highest; the rest resolve but more
  sparsely.
