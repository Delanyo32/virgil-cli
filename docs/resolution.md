# Symbol resolution — Cozoscript rules

Per [ADR-0005](adr/0005-datalog-resolution.md), resolution lives in Cozoscript rules over the `occurrence` / `scope` / `binding` / `imports` factbase. This document is the spec the rules implement.

> **Build-time vs query-time, current state (schema v7):**
>
> | Eager at build? | Relation |
> |---|---|
> | yes | `*imports{importer_file_id, imported_id}` — module-resolution is per-language Rust, hard to port to Cozoscript. |
> | **no (since v7)** | `*calls{caller_id, callee_id, ...}` — relation still exists in the schema but is **empty after build**. Derive call edges at query time over `*occurrence{occurrence_kind: 'call'}` + `*imports` + `*symbol`. See [Calls at query time](#calls-at-query-time) below. |
> | no (since v6) | `*references{...}` — same model: raw facts only, callers resolve at query time. See the rest of this document for the algorithm.|
>
> The v7 change was driven by container OOM on large C++ repos: openclaw (14,852 files) peaked at 3.26 GiB → SIGKILL under a 4 GiB cap. The `deferred_calls` Vec + `file_symbols_by_name`/`file_exports_by_name` HashMaps in `GraphBuilder::build` were the dominant RAM term. After the change, the same workload peaks at ~800 MiB. Imports were left eager because they're small (~30 MiB) and per-language module resolution is impractical to express in Cozoscript.

## Calls at query time

The replacement pattern lives in [`examples/cozoscript/calls_at_query_time.cozoql`](../examples/cozoscript/calls_at_query_time.cozoql); the built-in templates `find_callers`, `find_callees`, and `find_cycles` inline the same rules. Shape:

```cozo
# Same-file: callee is a symbol named N in the caller's own file.
call_edge[caller_id, callee_id, call_site_file, call_site_start_byte] :=
    *occurrence{name: callee_name, file_path: call_site_file,
                start_byte: call_site_start_byte,
                enclosing_symbol_id: caller_id,
                occurrence_kind: 'call'},
    *symbol{id: callee_id, name: callee_name,
            file_path: call_site_file, kind: callee_kind},
    callee_kind in ['function', 'method', 'arrow_function', 'macro'],
    caller_id != callee_id

# Cross-file: callee is an *exported* symbol named N in a file the
# caller's file imports.
call_edge[caller_id, callee_id, call_site_file, call_site_start_byte] :=
    *occurrence{name: callee_name, file_path: call_site_file,
                start_byte: call_site_start_byte,
                enclosing_symbol_id: caller_id,
                occurrence_kind: 'call'},
    *imports{importer_file_id: call_site_file, imported_id: callee_file},
    *symbol{id: callee_id, name: callee_name, file_path: callee_file,
            kind: callee_kind, exported: true},
    callee_kind in ['function', 'method', 'arrow_function', 'macro'],
    caller_id != callee_id
```

Accuracy matches the build-time resolver (same algorithm); the cost moves to the caller. Demand-scoped queries (e.g. "all callers of `foo`") are cheap because Cozo's planner can push the `$name` filter into the `*symbol` lookup before the join. Workspace-wide call enumeration costs roughly what it cost to populate `*calls` at build under v6.

---

## References (legacy section)

Below: the (now also query-time) `*references` algorithm. Same model as `call_edge` above — emit raw facts at build, resolve in Cozoscript on demand. The rules below were the spec for the v5-and-earlier eager resolver; today they are the reference implementation for callers who want to materialise references in their own scratch relation.

## Inputs

| Relation | Provided by |
|---|---|
| `occurrence` | Per-language extractor (every identifier occurrence) |
| `scope` | Per-language extractor (lexical scope tree per file) |
| `binding` | Per-language extractor (definition, parameter, import_alias, wildcard_import) |
| `imports` | Per-language extractor (resolved file-to-file imports) |
| `symbol` | Per-language extractor (definition sites) |

## Outputs

| Relation | Shape | Populated by |
|---|---|---|
| `references` | `(referrer_id, site_file, site_start_byte, match_index) => (referent_id, ref_kind)` | This document's resolver rules |

## Resolution algorithm (informal)

For each `occurrence{id, name, enclosing_scope_id, occurrence_kind}`:

1. **Walk scopes outward** from `enclosing_scope_id` toward the file scope. At each scope, look up `binding{scope_id, name}`. The first hit wins.
   - For `binding_kind = "definition"` or `"parameter"`: the `symbol_id` is the resolved target.
   - For `binding_kind = "import"` or `"import_alias"`: the binding's `symbol_id` is a target in another file; treat as resolved.
2. **If no scoped binding matches**, look for `wildcard_import` bindings in the chain. Each wildcard expands to every exported symbol in the imported file matching `name`.
3. **If still nothing**, the occurrence is unresolved: emit one `references` row with `referent_id = null`, `match_index = 0`.
4. **Overload disambiguator.** When step 1 or 2 produces multiple candidate `symbol_id`s at the same scope level (overloads, re-exports of the same name), emit one row per candidate at `match_index = 0, 1, 2, …` per [ADR-0003](adr/0003-level-3-types-and-references.md).

`ref_kind` carries over from `occurrence.occurrence_kind` 1-to-1.

## Cozoscript rules

```cozo
# ─── scope walking ─────────────────────────────────────────────────────

# scope_ancestor[child, ancestor] — child is `ancestor` itself OR
# transitively inside it. Base case + recursive step.
scope_ancestor[s, s] := *scope{id: s}
scope_ancestor[s, a] := *scope{id: s, parent_id: p}, p != null,
                       scope_ancestor[p, a]

# innermost_binding[occ, sym, bk, sb] — for occurrence `occ`, the
# scoped binding for its name closest to `occ`'s scope. Picks the
# binding with the largest `start_byte` among all candidate scopes
# (largest start_byte = nearest enclosing in source order).
candidate_binding[occ, sym, bk, sb] :=
    *occurrence{id: occ, name, enclosing_scope_id: occ_scope},
    scope_ancestor[occ_scope, anc_scope],
    *binding{scope_id: anc_scope, name, start_byte: sb, symbol_id: sym, binding_kind: bk},
    bk != 'wildcard_import',
    sym != null

innermost_binding[occ, sym, bk] :=
    candidate_binding[occ, sym, bk, sb],
    max_sb = max(sb),
    candidate_binding[occ, sym, bk, max_sb]

# ─── wildcard import expansion ─────────────────────────────────────────

wildcard_target[occ, sym] :=
    *occurrence{id: occ, name, file_path: of},
    *binding{scope_id: ws, binding_kind: 'wildcard_import', symbol_id: _},
    *scope{id: ws, file_path: of},
    *imports{importer_file_id: of, imported_id: tf},
    *symbol{id: sym, name, file_path: tf, exported: true}

# ─── final references view ─────────────────────────────────────────────

# Resolved (scoped binding hit, possibly multi-candidate for overloads).
resolved[occ, sym] := innermost_binding[occ, sym, _]
# Resolved via wildcard, only when no scoped binding matched.
resolved[occ, sym] := wildcard_target[occ, sym],
                     not innermost_binding[occ, _, _]

# Numbering candidates 0, 1, 2, … per occurrence (lexicographic on sym).
match_index[occ, sym, mi] :=
    resolved[occ, sym],
    mi = count_filter(s, resolved[occ, s], s < sym)

# Resolved references.
references_resolved[occ, sym, mi, file, sb, kind] :=
    match_index[occ, sym, mi],
    *occurrence{id: occ, file_path: file, start_byte: sb,
                enclosing_symbol_id: ref_id, occurrence_kind: kind},
    ref_id != null
    # referrer_id is the occurrence's enclosing_symbol_id; occurrences
    # outside any symbol (file-level expressions) are skipped per
    # ADR-0002's name-required referrer convention.

# Unresolved (no candidate). One null row at match_index 0.
references_unresolved[occ, file, sb, kind, ref_id] :=
    *occurrence{id: occ, file_path: file, start_byte: sb,
                enclosing_symbol_id: ref_id, occurrence_kind: kind},
    ref_id != null,
    not resolved[occ, _]

# Materialise.
?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] :=
    references_resolved[occ, sym, mi, file, sb, kind],
    *occurrence{id: occ, enclosing_symbol_id: referrer_id},
    site_file = file, site_start_byte = sb, match_index = mi,
    referent_id = sym, ref_kind = kind
    :put references {referrer_id, site_file, site_start_byte, match_index
                     => referent_id, ref_kind}

?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] :=
    references_unresolved[occ, file, sb, kind, referrer_id],
    site_file = file, site_start_byte = sb, match_index = 0,
    referent_id = null, ref_kind = kind
    :put references {referrer_id, site_file, site_start_byte, match_index
                     => referent_id, ref_kind}
```

The Cozoscript above is the *spec*; the actual implementation may split it into smaller stratified blocks for performance and may use `:create` of intermediate stored relations rather than ephemeral rules. Differences between the implementation and this spec are bugs against the spec.

## Edge cases the rules cover

- **Re-exports** (`pub use foo::bar`, `export { foo } from './foo'`) — the extractor emits a `binding` of kind `import_alias` with `symbol_id` pointing at the original definition's id. Transitive re-exports require either the extractor to chase the chain, or an additional recursive rule here; the current spec assumes the extractor resolves transitive re-exports during import resolution.
- **Aliased imports** (`import { foo as bar }`) — `binding{name: "bar", binding_kind: "import_alias", symbol_id: <foo's id>}`. No special resolver code needed; bindings handle it.
- **Wildcard imports** (`use foo::*`) — emitted as `binding{name: "*", binding_kind: "wildcard_import"}`. The `wildcard_target` rule expands at resolution time.
- **Shadowing** (Rust `let x = 1; let x = 2;`) — multiple `binding{scope_id, name: "x"}` rows; `innermost_binding` picks the one with the largest `start_byte` smaller than the occurrence's position. (Spec note: this requires `start_byte < occ.start_byte` to be filtered in `candidate_binding` — TODO when the rule is implemented.)
- **`this` / `self` references** — extractors emit them as `read` occurrences with `name = "this"` or `"self"`; bindings of kind `parameter` cover them at the class/function scope.

## Edge cases the rules deliberately don't cover

- **Method dispatch** (`obj.method()`). Without type info, we can't know which `method` is meant. Extractors emit an `occurrence` with `name = "method"` and `occurrence_kind = "call"`, but no `binding` matches at any enclosing scope, so it resolves to `null`. The `obj` part *does* resolve (it's a `read` occurrence).
- **Dynamic dispatch** (`getattr(obj, name)()`, `$$dynamic` in PHP, etc.) — same as method dispatch: extractors emit nothing and the resolver produces no row.

## Versioning the resolver

The Cozoscript rules ship in `src/queries/resolution/` (folder TBD when implemented). Bumping `SCHEMA_VERSION` is required when the resolver semantics change in a way that would produce different `references` rows from the same `occurrence` + `binding` facts.

## Tests

The reference test suite for the resolver lives in `tests/resolution/` (folder TBD). Each test fixes:
1. A small synthetic `occurrence` / `scope` / `binding` / `imports` factbase.
2. The exact `references` rows the resolver must materialise.

Per-language extractor tests (Issue #16a) feed real source files into the full pipeline and assert the resulting `references` rows. The resolver tests above cover the rules themselves in isolation.
