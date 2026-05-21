//! Issue #16 ADR-0005 Cozoscript resolver. Materialises the
//! `references` relation from `occurrence` / `scope` / `binding` /
//! `imports` facts emitted by per-language extractors.
//!
//! See `docs/resolution.md` for the spec.

use anyhow::Result;

use super::CozoStore;

/// Cozoscript program that produces `references` rows.
///
/// A Cozo program has exactly one terminal `?[...]` query head; we
/// build supporting rules (`scope_ancestor`, `resolved`, etc.) and
/// funnel everything through `refs_out` then the final `?[...]`.
/// `:put references {...}` writes the result back.
const RESOLVED_SCRIPT: &str = r#"
scope_ancestor[s, t] := *scope{id: s}, t = s
scope_ancestor[s, t] := scope_ancestor[s, mid], *scope{id: mid, parent_id: t}, t != null

# #18.2a + 18.2b: innermost-scope pick (max scope.start_byte among
# scopes that have a matching name binding) — preserves overload
# candidates that all live in the same scope.
scope_with_binding[occ, sid, ssb] :=
    *occurrence{id: occ, name: n, enclosing_scope_id: occ_scope},
    scope_ancestor[occ_scope, sid],
    *binding{scope_id: sid, name: n, symbol_id: sym, binding_kind: bk},
    bk != "wildcard_import",
    sym != null,
    *scope{id: sid, start_byte: ssb}

occ_max_scope_sb[occ, max(ssb)] := scope_with_binding[occ, _, ssb]

innermost_scope[occ, sid] :=
    scope_with_binding[occ, sid, ssb],
    occ_max_scope_sb[occ, ssb]

innermost_binding[occ, sym] :=
    *occurrence{id: occ, name: n},
    innermost_scope[occ, isid],
    *binding{scope_id: isid, name: n, symbol_id: sym, binding_kind: bk},
    bk != "wildcard_import",
    sym != null

has_scoped[occ] := innermost_binding[occ, _]

# #18.2e: cross-file chain (single hop). An `import` /
# `import_alias` binding with `symbol_id = null` means "the extractor
# didn't follow"; the resolver follows the `imports` relation +
# matching-name binding in the target file. Multi-hop transitive
# re-exports require a recursive `file_resolves` rule which cozo's
# analyzer currently rejects in this form — tracked as a follow-up.
chain_resolved[occ, sym] :=
    *occurrence{id: occ, name: cn, file_path: cof},
    *scope{id: csc, file_path: cof},
    *binding{scope_id: csc, name: cn, symbol_id: cnb, binding_kind: cbk},
    cnb == null,
    cbk != "wildcard_import",
    *imports{importer_file_id: cof, imported_id: tf},
    *scope{id: ts, file_path: tf},
    *binding{scope_id: ts, name: cn, symbol_id: sym, binding_kind: tbk},
    sym != null,
    tbk != "wildcard_import",
    *symbol{id: sym}

# #18.2c: wildcard import expansion. When no scoped binding hits, the
# occurrence's file may have wildcard_import bindings.
wildcard_target[occ, sym] :=
    *occurrence{id: occ, name: n, file_path: of},
    *scope{id: wscope, file_path: of},
    *binding{scope_id: wscope, binding_kind: "wildcard_import"},
    *imports{importer_file_id: of, imported_id: tf},
    *symbol{id: sym, name: n, file_path: tf, exported: true},
    not has_scoped[occ]

resolved[occ, sym] := innermost_binding[occ, sym]
resolved[occ, sym] := chain_resolved[occ, sym]
resolved[occ, sym] := wildcard_target[occ, sym]

# #18.2b: deterministic match_index per occurrence. Count of
# candidates whose sym is lex ≤ this one. Singleton → count = 1 → mi = 0.
# Two candidates A<B → A: count=1 → mi=0; B: count=2 → mi=1.
match_index_count[occ, sym, count(s)] :=
    resolved[occ, sym],
    resolved[occ, s],
    s <= sym

?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] :=
    match_index_count[occ, referent_id, c],
    match_index = c - 1,
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null

:put references {referrer_id, site_file, site_start_byte, match_index => referent_id, ref_kind}
"#;

const UNRESOLVED_SCRIPT: &str = r#"
scope_ancestor[s, t] := *scope{id: s}, t = s
scope_ancestor[s, t] := scope_ancestor[s, mid], *scope{id: mid, parent_id: t}, t != null

has_scoped_resolution[occ] :=
    *occurrence{id: occ, name: n, enclosing_scope_id: occ_scope},
    scope_ancestor[occ_scope, anc_scope],
    *binding{scope_id: anc_scope, name: n, symbol_id: sym, binding_kind: bk},
    bk != "wildcard_import",
    sym != null

# #18.2c: wildcard-import resolutions also count as resolved — don't
# emit a null fallback when a wildcard would have matched.
has_wildcard_resolution[occ] :=
    *occurrence{id: occ, name: n, file_path: of},
    *scope{id: wscope, file_path: of},
    *binding{scope_id: wscope, binding_kind: "wildcard_import"},
    *imports{importer_file_id: of, imported_id: tf},
    *symbol{id: sym, name: n, file_path: tf, exported: true}

# #18.2e: chain-resolved occurrences (null-sym binding + single-hop
# imports lookup). Don't emit null fallback when a chain would have
# matched.
has_chain_resolution[occ] :=
    *occurrence{id: occ, name: cn, file_path: cof},
    *scope{id: csc, file_path: cof},
    *binding{scope_id: csc, name: cn, symbol_id: cnb, binding_kind: cbk},
    cnb == null,
    cbk != "wildcard_import",
    *imports{importer_file_id: cof, imported_id: tf},
    *scope{id: ts, file_path: tf},
    *binding{scope_id: ts, name: cn, symbol_id: csym, binding_kind: tbk},
    csym != null,
    tbk != "wildcard_import",
    *symbol{id: csym}

has_resolution[occ] := has_scoped_resolution[occ]
has_resolution[occ] := has_wildcard_resolution[occ]
has_resolution[occ] := has_chain_resolution[occ]

?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] :=
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null,
    not has_resolution[occ],
    match_index = 0,
    referent_id = null

:put references {referrer_id, site_file, site_start_byte, match_index => referent_id, ref_kind}
"#;

/// Materialise the `references` relation by running the Cozoscript
/// resolver against the current `occurrence` / `scope` / `binding` /
/// `imports` / `symbol` facts.
///
/// Called after all fact-emission flushes. No-ops cheaply when the
/// `occurrence` relation is empty.
pub fn resolve_references(store: &CozoStore) -> Result<()> {
    // Cheap short-circuit: skip the heavy resolver if no occurrences.
    let count = store
        .run_query(
            "?[count(id)] := *occurrence{id}",
            std::collections::BTreeMap::new(),
        )
        .map_err(|e| anyhow::anyhow!("occurrence-count probe failed: {e}"))?;
    let n = count
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| match v {
            cozo::DataValue::Num(cozo::Num::Int(i)) => Some(*i),
            _ => None,
        })
        .unwrap_or(0);
    if n == 0 {
        return Ok(());
    }
    store
        .run_script(RESOLVED_SCRIPT, std::collections::BTreeMap::new())
        .map_err(|e| anyhow::anyhow!("references resolver (resolved) failed: {e}"))?;
    store
        .run_script(UNRESOLVED_SCRIPT, std::collections::BTreeMap::new())
        .map_err(|e| anyhow::anyhow!("references resolver (unresolved) failed: {e}"))?;
    Ok(())
}
