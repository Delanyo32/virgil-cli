# Cozoscript examples — on-demand reference resolution

Virgil's build path no longer materialises a `references` relation. The
raw `occurrence`, `scope`, `binding`, and `imports` facts are still
emitted, so callers who need resolved references compute them at query
time over those facts.

This folder collects example scripts you can run through
`virgil projects query <name> --file …`, plus a longer-form walkthrough
of the staged algorithm we used to run eagerly.

| File | What it does | Cost shape |
|---|---|---|
| `find_writers_of.cozoql` | Lists every byte where `$name` is written, resolving each occurrence to its target symbol through the lexical scope chain. Demand-scoped — only the scopes that contain a `$name` occurrence get walked. | Cheap on any repo. |
| `unused_symbols.cozoql` | Workspace-wide: exported symbols with no scope-chain reference. Inlines the full ancestor closure over every scope. | **Expensive on big repos.** See "scaling" below. |
| `calls_at_query_time.cozoql` | Derives caller→callee edges from `*occurrence` + `*imports` + `*symbol`. Paste the `call_edge` rules as a prelude in your own queries to get a `*calls`-equivalent view. Filter `$name` constrains to outgoing calls from any symbol with that name. | Demand-scoped is cheap; workspace-wide (no filter) materialises one row per call site, like the old `*calls` relation. |
| `resolve_references_full.md` | The original 8-stage staged resolver, written as a series of programs you can run sequentially. Use when you want a `references_ad_hoc` relation materialised once and queried many times within a session. | Workspace-wide, mostly fixed cost — same as the old eager build. |

## Running an example

Demand-scoped (fast):

```bash
virgil projects query myproject \
  --file examples/cozoscript/find_writers_of.cozoql \
  --param name=login
```

Workspace-wide (potentially slow):

```bash
virgil projects query myproject --file examples/cozoscript/unused_symbols.cozoql
```

## Why on-demand instead of eager

The eager Cozoscript resolver computed the full transitive closure of
every scope's parent chain at build time (`rsv_ancestor` in the
historical `src/cozo/resolver.rs`). On a 5.5k-file repo it OOMed; on
django it sat at 4.6 GB for nearly 6 minutes. Most queries never used
the resulting `references` relation, and the ones that did (template
`find_writers_of`, template `unused_symbols`) had small demand sets.

Moving resolution to query time:
- Build memory dropped 67–90% across the bench matrix.
- Build time dropped 78–98%.
- The 5.5k repo now builds in ~27 seconds with ~580 MB.

The trade-off is that any query needing references pays the resolution
cost itself. For demand-scoped queries (single name, single file), this
is cheap. For workspace-wide queries, it's the same cost as before —
but now you pay it only when you ask.

## Writing your own

Two patterns:

**1. Inline (one program).** Express resolution as Cozo rules in the
prelude of your query. See `find_writers_of.cozoql` for the shape. This
works when the demand set is small enough that the ancestor closure
stays tractable.

**2. Staged (multiple programs).** Run a sequence of programs that
write into temp stored relations (`:replace foo {...}`), then a final
program that reads them. This is what `resolve_references_full.md`
documents. Use when one resolution result will be queried many times
in a session and the workspace is large.

## Caveats

- Cozo silently drops relations whose names start with `_`. Use a
  prefix like `rsv_` or `tmp_` for scratch tables.
- Cozo's planner can't push some filters through aggregation or
  negation. Pre-stage your most selective input (often `imports`) into
  a smaller eligibility set before the big join.
- These examples target the lexical-scope + import resolution model
  from `docs/resolution.md`. If you've changed the schema, adjust the
  rules accordingly.
