# Directory Coupling (layer flow and violations)

## The ask

Show how a codebase's **directories depend on each other** — the layered
architecture, its flow direction, and any edges that break the layering —
using only virgil-cli's facts.

This is the **directed** counterpart to `feature-clusters.md`. Same source
graph (`imports`, file→file), but rolled up to the directory and kept
directional:

- **Feature clusters** answer "which files belong together" — undirected
  communities, folder-agnostic.
- **Directory coupling** answers "which layer calls which" — directed,
  folder-aware. It surfaces the `routes → controllers → services → models`
  shape and the edges that violate it.

The roll-up is a string op: a file's directory is its path with the last
segment stripped, `regexp_replace(path, '/[^/]+$', '')`. No join to `symbol`
(see Limitations b1 — `imported_id` is already a path).

## The queries

### 1. Cross-dir coupling matrix — the layer flow

```sql
SELECT regexp_replace(importer_file_id, '/[^/]+$', '') src_dir,
       regexp_replace(imported_id,      '/[^/]+$', '') dst_dir,
       COUNT(*) edges
FROM imports
WHERE regexp_replace(importer_file_id, '/[^/]+$', '')
   <> regexp_replace(imported_id,      '/[^/]+$', '')   -- cross-dir only
GROUP BY 1, 2
ORDER BY edges DESC;
```

Each row is a directed `src_dir → dst_dir` edge weighted by how many file
imports cross it. Read top-down it *is* the architecture diagram: which
layer leans on which, and how hard.

### 2. Intra-dir cohesion — edges that stay inside one directory

```sql
SELECT regexp_replace(importer_file_id, '/[^/]+$', '') dir,
       COUNT(*) internal_edges
FROM imports
WHERE regexp_replace(importer_file_id, '/[^/]+$', '')
    = regexp_replace(imported_id,      '/[^/]+$', '')
GROUP BY 1
ORDER BY internal_edges DESC;
```

High intra-dir edges = a directory whose files actually collaborate (a real
module). Near-zero = a **bucket-of-siblings** — files grouped by kind
(`controllers/`, `models/`) that don't talk to each other and only fan out
to other layers. Both are normal; the ratio tells you whether a folder is a
module or a category.

### 3. Layer-violation detection — edges going *up* the layer order

Assign each directory a rank (higher = closer to the entrypoint), then flag
any edge whose source rank is **below** its destination — a lower layer
reaching up into a higher one. The `CASE` ladder is the only
project-specific part; set it to your layer names.

```sql
WITH ranked AS (
  SELECT importer_file_id src, imported_id dst,
    CASE WHEN importer_file_id LIKE '%/routes/%'      THEN 5
         WHEN importer_file_id LIKE '%/controllers/%' THEN 4
         WHEN importer_file_id LIKE '%/services/%'    THEN 3
         WHEN importer_file_id LIKE '%/models/%'      THEN 2 ELSE 1 END sr,
    CASE WHEN imported_id LIKE '%/routes/%'      THEN 5
         WHEN imported_id LIKE '%/controllers/%' THEN 4
         WHEN imported_id LIKE '%/services/%'    THEN 3
         WHEN imported_id LIKE '%/models/%'      THEN 2 ELSE 1 END dr
  FROM imports)
SELECT src, dst, sr src_layer, dr dst_layer
FROM ranked
WHERE sr < dr AND sr > 1 AND dr > 1   -- exclude the shared rank-1 bucket
ORDER BY dr - sr DESC;
```

A hit is an inverted dependency — `models/User.js → controllers/…` means a
model knows about a controller, the textbook layering smell. `sr > 1 AND
dr > 1` keeps shared utils/config (rank 1) out of the judgement, since
everything depends on those legitimately.

Run any with `projects query <name> --file dir_coupling.sql` (or `--sql '…'`).

## What the output looks like

**(1) coupling matrix** — `express-api` (55 js):

```
src_dir            dst_dir          edges
src/controllers -> src/models        14
src/routes      -> src/middleware     9
src/services    -> src/models         9
src/controllers -> src/utils          8
src/routes      -> src/controllers    7
src             -> src/routes         7
src/controllers -> src/services       7
src/middleware  -> src/config         3
```

The whole MVC flow falls out top-down: the entrypoint (`src`) → `routes` →
`controllers` → `services` → `models`, with `middleware`/`utils`/`config` as
shared sinks. `controllers → models` (14) is the heaviest seam — the place a
schema change ripples furthest.

**(2) intra-dir cohesion** — `express-api`:

```
dir            internal_edges
src/services   2
src            1
```

Almost everything is cross-dir. `controllers/`, `models/`, `routes/` have
**zero** internal edges — they're buckets-of-siblings (grouped by kind, not
collaborating). Only `services/` shows a hint of intra-module wiring. That's
the structural signature of layer-first (not feature-first) organisation.

**(3) layer violations** — `express-api` **and** `laravel-store`: **0 rows**.
Both corpora are cleanly layered — every cross-layer edge points *down*, so
the matrix in (1) never inverts. An empty result here is the **pass**
condition, not an error: it's the query confirming the layering holds. A
violation, when present, reads:

```
src                          dst                            src_layer  dst_layer
app/Models/User.php          app/Http/Controllers/Auth.php  1          4   ← model → controller
```

— a lower layer importing a higher one, the smell to chase.

## Limitations

- **The directory is the layer — only if the codebase agrees.** The roll-up
  assumes one-directory-per-concern. Flat repos (the `technical-debt` bench
  extract is just `app/` + `tests/`) collapse to a trivial matrix; deeply
  nested or feature-first trees (`features/cart/{controller,model}.ts`) put
  every layer in *one* dir, so the cross-dir matrix goes quiet and you want
  `feature-clusters.md` instead. Check the dir list first:
  `SELECT DISTINCT regexp_replace(importer_file_id,'/[^/]+$','') FROM imports`.

- **The layer ranking in (3) is hand-supplied.** virgil has no notion of
  "layer" — the `CASE` ladder encodes *your* intended order. Wrong names →
  wrong violations. There's no facts-only way to infer layer order; the
  matrix in (1) is the input you read it off.

- **Roll-up is last-segment only.** `regexp_replace(.,'/[^/]+$','')` groups
  by the immediate parent dir, so `a/b/x.js` and `a/c/y.js` are different
  groups. For coarser grouping (top-level only) swap in a prefix expression
  like `split_part(path,'/',1)`.

- **`imports.imported_id` is a file path, not a symbol id** (b1) — same
  gotcha as `references-ranking.md` and `internal-vs-external-deps.md`.
  `imports` *is* the file→file edge; don't join it to `symbol.id`.

- **`imports` holds only internal edges.** External packages never resolve
  to a workspace file, so the matrix is internal-coupling only — it can't
  show "which layer pulls in the most libraries" (use `raw_import` and
  `internal-vs-external-deps.md` for that). Coupling density also rides on
  resolution quality: dense for JS/TS/PHP, sparser for Go/Java/C#/Rust, so
  the matrix under-weights cross-dir seams there.

- **Stale stores under-report.** Same as the other import skills: empty or
  TS/JS-only matrices on an existing store → cold-rebuild with `--rebuild`.
