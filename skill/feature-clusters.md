# Feature Clusters (vertical slices from the import graph)

## The ask

Find a codebase's **feature clusters** — the groups of files that belong
to one feature (cart, auth, payments) rather than one folder — using only
virgil-cli's facts.

A feature cluster is a **community** in the file→file import graph: files
that import each other densely but reach the rest of the app only through a
few shared files. The trap is that "community" is **not** the same as
"connected component", and the naive query conflates them:

- **Connected component (WCC)** — everything transitively reachable over
  imports. On any layered app this is **one giant blob**, because the
  entrypoint and shared base modules bridge every feature into one.
- **Community (what you want)** — dense subgroups *after* the bridges are
  removed. The bridge set is itself meaningful: it's the **shared kernel**
  (entrypoint, base models, utils, auth).

So feature clustering is: delete the bridge files, then run WCC on what's
left. Each surviving component is a feature; the deleted set is the kernel.

The graph is `imports` directly — `importer_file_id` → `imported_id`, **both
file paths** (see Limitations b1). No join to `symbol` is needed or correct.

## The queries

### 1. Feature clusters — strip bridges, then WCC

```sql
WITH RECURSIVE
fa     AS (SELECT importer_file_id f, COUNT(*) n FROM imports GROUP BY 1),
fi     AS (SELECT imported_id f, COUNT(DISTINCT importer_file_id) n FROM imports GROUP BY 1),
bridge AS (SELECT f FROM fi WHERE n >= 3 UNION SELECT f FROM fa WHERE n >= 4),
edges  AS (
  SELECT importer_file_id a, imported_id b FROM imports
  WHERE importer_file_id NOT IN (SELECT f FROM bridge) AND imported_id NOT IN (SELECT f FROM bridge)
  UNION
  SELECT imported_id, importer_file_id FROM imports
  WHERE importer_file_id NOT IN (SELECT f FROM bridge) AND imported_id NOT IN (SELECT f FROM bridge)
),
nodes  AS (SELECT a f FROM edges UNION SELECT b f FROM edges),
reach(seed, node) AS (
  SELECT f, f FROM nodes
  UNION
  SELECT r.seed, e.b FROM reach r JOIN edges e ON e.a = r.node
),
comp AS (SELECT node file, MIN(seed) feature FROM reach GROUP BY node)
SELECT feature, COUNT(*) files, string_agg(file, ', ') members
FROM comp GROUP BY feature ORDER BY files DESC;
```

`fa`/`fi` are fan-out / fan-in per file. `bridge` is the shared kernel.
`edges` makes the remaining graph undirected (both directions) so a feature
is found regardless of which file imports which. `reach` is a transitive
closure; the component label is `MIN(seed)` — files sharing a label are one
feature. (`cluster` is a DuckDB reserved word — the label column is named
`feature`.)

### 2. The shared kernel — what got removed

```sql
WITH fa AS (SELECT importer_file_id f, COUNT(*) n FROM imports GROUP BY 1),
     fi AS (SELECT imported_id f, COUNT(DISTINCT importer_file_id) n FROM imports GROUP BY 1)
SELECT f, 'fan-in' why FROM fi WHERE n >= 3
UNION SELECT f, 'fan-out' FROM fa WHERE n >= 4
ORDER BY f;
```

The bridge set is the codebase's spine — base models, auth, utils, the app
entrypoint. Read it as "what every feature depends on."

Run either with `projects query <name> --file feature_clusters.sql` (or
`--sql '…'`). The two thresholds (`fi >= 3`, `fa >= 4`) are the only knobs:
**lower** them → more files treated as kernel → tighter, smaller clusters;
**raise** them → fewer bridges removed → clusters merge back toward the blob.

## What the output looks like

The progression on `express-api` (55 js) is the whole lesson — same graph,
three clusterings:

```
naive WCC                       1 blob   (44 files)   ← useless
remove fan-in hubs only         1 blob   (34 files)   ← entrypoint still bridges
remove fan-in AND fan-out      8 features              ← the slices appear
```

**(1) feature clusters** — `express-api`:

```
feature                  files  members
commentController.js     4      comments.js, commentController.js,
                                notificationService.js, emailService.js
categoryController.js    3      categories.js, categoryController.js, Category.js
userController.js        3      users.js, userController.js, formatters.js
tagController.js         2      tags.js, tagController.js
Session.js               2      Session.js, authService.js
```

Each row is a **vertical slice**: route + controller + the service/model it
owns. "Comments" pulling in `notificationService` + `emailService` is a real
domain fact — commenting triggers notifications — that the folder tree
(`controllers/`, `services/`) never shows.

**(1) `laravel-store`** (70 php) — PHP resolves cleanly into textbook
verticals:

```
CartController.php     4   Cart.php, CartItem.php, CartService.php, CartController.php
PaymentController.php  3   Payment.php, PaymentService.php, PaymentController.php
CouponController.php   2   CouponService.php, CouponController.php
SearchController.php   2   SearchService.php, SearchController.php
ProductController.php  2   ProductRepository.php, ProductController.php
```

Cart, Payment, Coupon, Search, Product — the domain, read straight off the
import graph.

**(2) shared kernel** — `express-api`: `app.js` (fan-out), every
`models/{Post,User,Comment,Tag}.js` + `middleware/auth.js` +
`config/{auth,constants}.js` + `utils/{pagination,slugify}.js` (fan-in), plus
the fan-out controllers/services. That's the spine the 8 features hang off.

## Limitations

- **Thresholds are the result.** This is a heuristic, not a principled
  community-detection algorithm. The `fi >= 3` / `fa >= 4` cut decides
  everything — different thresholds give different clusters. Tune per
  codebase and read the kernel (query 2) to sanity-check the cut. For
  threshold-free results you'd want modularity-based detection (Louvain),
  which isn't expressible in plain SQL — it'd need a Rust template calling a
  graph lib, like `complexity_hotspots` escapes SQL for metrics.

- **WCC over the full graph is always one blob on layered code.** Don't ship
  the naive component query as a "feature finder" — for MVC / layered apps
  layering *is* connection, so WCC can't split it. The bridge removal is the
  point, not an optimisation.

- **`imports.imported_id` is a file path, not a symbol id** (b1) — same
  gotcha as `references-ranking.md` and `internal-vs-external-deps.md`.
  `imports` *is* the file→file edge; don't join it to `symbol.id` (matches
  nothing). `SELECT imported_id FROM imports LIMIT 3` confirms it holds
  `src/models/Post.js`.

- **`imports` holds only internal edges.** External packages never resolve
  to a workspace file, so they never bridge — which is correct here (you
  want internal cohesion), but it means clustering quality rides entirely on
  internal-import resolution. Dense for JS/TS/PHP; sparser for
  Go/Java/C#/Rust (see `internal-vs-external-deps.md`), so clusters there are
  coarser and a real feature can fragment.

- **Singletons drop out.** A file with no surviving edge after bridge
  removal (it only ever imported kernel files) produces no row — it isn't
  "featureless", it's *pure glue*. Cross-check against the kernel set rather
  than reading absence as a bug.

- **Undirected by design.** `edges` unions both directions, so the cluster
  is a co-membership grouping, not a dependency direction. For "who depends
  on whom inside a feature", keep the directed `imports` and read it per
  cluster.

- **Stale stores under-report.** Same as the other import skills: the
  cross-language resolver fix changed what lands in `imports` (empty for
  Go/Java/Python/C/C++/PHP/C# before). Empty or TS/JS-only clusters on an
  existing store → cold-rebuild with `--rebuild`.
