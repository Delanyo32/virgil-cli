# Workspace-wide reference resolution — staged

This is the original 8-stage resolver that used to run automatically at
the end of every build. We removed it because most queries never ask
for references, but the staging trick is still the right way to resolve
references *workspace-wide* on a large repo: each stage writes into a
temp stored relation so Cozo's planner can drive joins from the
smallest input (`imports`, often <0.1% of bindings).

Run each stage as a separate query (`virgil projects query <name>
--cozoscript '…'` or `--file <stage>.cozoql`). The temp relations
persist for the lifetime of the cache file, so a later session can read
the result without re-running the resolver.

## Why staged

Cozo's planner cannot push `imports` filters through `count(s) where
s <= sym` aggregations or through `not has_*` negation. Without
pre-staging the eligibility set, the `chain` join expands
`occurrence × scope × binding` for the entire workspace before the
`imports` filter applies — 20+ minutes on a 5k-file workload.

The chain and wildcard stages each have an `_eligible` pre-stage that
folds `imports` in first, dropping the working set by 100×.

## Stage 1: ancestor closure (all scopes)

Reflexive-transitive closure of `scope.parent_id`. This is the
expensive one — O(scopes × avg depth) rows. On django you'll see
millions of rows in `rsv_ancestor`. If you can pre-filter to scopes
that actually contain occurrences, do — see
`find_writers_of.cozoql` for the demand-scoped pattern.

```cozoql
sa[s, t] := *scope{id: s}, t = s
sa[s, t] := sa[s, mid], *scope{id: mid, parent_id: t}, t != null
?[scope_id, ancestor_id] := sa[scope_id, ancestor_id]
:replace rsv_ancestor {scope_id: String, ancestor_id: String}
```

## Stage 2: innermost binding per occurrence

For each occurrence, find the binding in the closest enclosing scope
that has a non-null `symbol_id`.

```cozoql
swb[occ, sid, ssb] :=
    *occurrence{id: occ, name: n, enclosing_scope_id: occ_scope},
    *rsv_ancestor{scope_id: occ_scope, ancestor_id: sid},
    *binding{scope_id: sid, name: n, symbol_id: sym, binding_kind: bk},
    bk != "wildcard_import",
    sym != null,
    *scope{id: sid, start_byte: ssb}

mss[occ, max(ssb)] := swb[occ, _, ssb]

?[occ, sym] :=
    swb[occ, sid, ssb],
    mss[occ, ssb],
    *occurrence{id: occ, name: n},
    *binding{scope_id: sid, name: n, symbol_id: sym, binding_kind: bk},
    bk != "wildcard_import",
    sym != null
:replace rsv_innermost {occ: String, sym: String}
```

## Stage 3: chain eligibility (imports-first)

Pre-filter cross-file resolution to the `(file, name, target_file)`
triples that could possibly succeed. Drives the chain stage from the
smallest input.

```cozoql
?[file_path, name, target_file] :=
    *imports{importer_file_id: file_path, imported_id: target_file},
    *scope{id: sid, file_path},
    *binding{scope_id: sid, name, symbol_id: cnb, binding_kind: cbk},
    cnb == null,
    cbk != "wildcard_import"
:replace rsv_chain_eligible {file_path: String, name: String, target_file: String}
```

## Stage 4: chain resolution

For each eligible triple, find an exported symbol with the matching
name in the imported file.

```cozoql
?[occ, sym] :=
    *rsv_chain_eligible{file_path: cof, name: cn, target_file: tf},
    *occurrence{id: occ, name: cn, file_path: cof},
    *scope{id: ts, file_path: tf},
    *binding{scope_id: ts, name: cn, symbol_id: sym, binding_kind: tbk},
    sym != null,
    tbk != "wildcard_import",
    *symbol{id: sym}
:replace rsv_chain {occ: String, sym: String}
```

## Stage 5: wildcard eligibility (imports-first)

Same restructure as chain: pre-filter `(file, target_file)` pairs where
the importer's file has a wildcard binding.

```cozoql
?[file_path, target_file] :=
    *imports{importer_file_id: file_path, imported_id: target_file},
    *scope{id: sid, file_path},
    *binding{scope_id: sid, binding_kind: "wildcard_import"}
:replace rsv_wildcard_eligible {file_path: String, target_file: String}
```

## Stage 6: wildcard resolution

For occurrences that didn't already get a scoped binding, resolve
through wildcard imports to any exported symbol with the matching name
in the imported file.

```cozoql
has_scoped[occ] := *rsv_innermost{occ}
?[occ, sym] :=
    *rsv_wildcard_eligible{file_path: of, target_file: tf},
    *occurrence{id: occ, name: n, file_path: of},
    *symbol{id: sym, name: n, file_path: tf, exported: true},
    not has_scoped[occ]
:replace rsv_wildcard {occ: String, sym: String}
```

## Stage 7: union of resolved (innermost ∪ chain ∪ wildcard)

```cozoql
res[occ, sym] := *rsv_innermost{occ, sym}
res[occ, sym] := *rsv_chain{occ, sym}
res[occ, sym] := *rsv_wildcard{occ, sym}
?[occ, sym] := res[occ, sym]
:replace rsv_resolved {occ: String, sym: String}
```

## Stage 8: materialise into an ad-hoc references relation

Pick whatever final shape you want. Below is the shape the old
built-in `references` relation used (key by referrer/site/match_index,
nullable referent for unresolved occurrences).

```cozoql
# Resolved rows joined with occurrence metadata.
?[occ, sym, referrer_id, site_file, site_start_byte, ref_kind] :=
    *rsv_resolved{occ, sym},
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null
:replace rsv_resolved_joined {occ: String, sym: String =>
    referrer_id: String, site_file: String,
    site_start_byte: Int, ref_kind: String}
```

To produce the final `references_ad_hoc` relation, you have a choice
about `match_index` (the per-occurrence overload disambiguator):

- **Skip it.** Drop the column from your schema if you don't need it.
- **Assign sequentially in Rust.** The original code did this because
  Cozo's planner can't simplify the `count(s) where s <= sym` self-join.
  See historic `src/cozo/resolver.rs::finalize_resolved` for the
  reference implementation — read sorted by `occ, sym`, assign
  `match_index = 0..n-1` per `occ` group, batch-write.
- **Live with the self-join.** Tolerable on small workspaces; fine
  for occasional one-off queries.

A simple shape without `match_index`:

```cozoql
?[referrer_id, site_file, site_start_byte, referent_id, ref_kind] :=
    *rsv_resolved_joined{referrer_id, site_file, site_start_byte,
                         sym: referent_id, ref_kind}
:replace references_ad_hoc {referrer_id: String, site_file: String,
                            site_start_byte: Int =>
                            referent_id: String, ref_kind: String}
```

## Optional: null fallback for unresolved occurrences

Adds one row per occurrence that no stage matched, with `referent_id =
null`. Useful for "show me everything the resolver couldn't reach"
queries.

```cozoql
has_any[occ] := *rsv_resolved{occ}
?[referrer_id, site_file, site_start_byte, referent_id, ref_kind] :=
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null,
    not has_any[occ],
    referent_id = null
:put references_ad_hoc {referrer_id, site_file, site_start_byte =>
                        referent_id, ref_kind}
```

## Cleanup

Drop the temp relations when you're done (or leave them for next
session; they persist in the cache file until the schema version
changes).

```cozoql
::remove rsv_ancestor
::remove rsv_innermost
::remove rsv_chain_eligible
::remove rsv_chain
::remove rsv_wildcard_eligible
::remove rsv_wildcard
::remove rsv_resolved
::remove rsv_resolved_joined
```
