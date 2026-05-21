//! Issue #16 ADR-0005 Cozoscript resolver. Materialises the
//! `references` relation from `occurrence` / `scope` / `binding` /
//! `imports` facts emitted by per-language extractors.
//!
//! See `docs/resolution.md` for the spec.

use anyhow::Result;
use tracing::{debug, info_span};

use super::CozoStore;

/// Single Cozoscript program that materialises both resolved and null-
/// fallback `references` rows in one pass.
///
/// The previous implementation ran two separate Cozo programs — one for
/// resolved refs, one for the null fallback. The fallback program had to
/// re-derive `has_scoped_resolution` / `has_wildcard_resolution` /
/// `has_chain_resolution` from scratch in order to negate them, costing
/// roughly as much as the main resolver. Consolidating lets `has_any`
/// reuse the already-materialised `resolved[occ, _]` set.
///
/// Two `?[...]` heads with identical column shapes feed into the same
/// `:put references` action via disjunction.
const RESOLVER_SCRIPT: &str = r#"
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
# matching-name binding in the target file.
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

# `resolved` is the union of all three resolution paths — the null
# fallback can negate against it directly, no need to re-derive the
# per-path `has_*` predicates.
has_any_resolution[occ] := resolved[occ, _]

# #18.2b: deterministic match_index per occurrence. Count of
# candidates whose sym is lex ≤ this one. Singleton → count = 1 → mi = 0.
# Two candidates A<B → A: count=1 → mi=0; B: count=2 → mi=1.
match_index_count[occ, sym, count(s)] :=
    resolved[occ, sym],
    resolved[occ, s],
    s <= sym

# Resolved rows.
?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] :=
    match_index_count[occ, referent_id, c],
    match_index = c - 1,
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null

# Null-fallback rows for occurrences that no resolution path matched.
?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] :=
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null,
    not has_any_resolution[occ],
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
    let n = {
        let _s = info_span!("cozo.resolver.count_probe").entered();
        // Cheap short-circuit: skip the heavy resolver if no occurrences.
        let count = store
            .run_query(
                "?[count(id)] := *occurrence{id}",
                std::collections::BTreeMap::new(),
            )
            .map_err(|e| anyhow::anyhow!("occurrence-count probe failed: {e}"))?;
        count
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(|v| match v {
                cozo::DataValue::Num(cozo::Num::Int(i)) => Some(*i),
                _ => None,
            })
            .unwrap_or(0)
    };
    debug!(occurrences = n, "resolver input cardinality");
    if n == 0 {
        return Ok(());
    }
    {
        let _s = info_span!("cozo.resolver.run", occurrences = n).entered();
        store
            .run_script(RESOLVER_SCRIPT, std::collections::BTreeMap::new())
            .map_err(|e| anyhow::anyhow!("references resolver failed: {e}"))?;
        debug!(references_rows = count_references(store), "resolver pass complete");
    }
    Ok(())
}

/// Best-effort `references` row count for logging. Returns `-1` on any
/// error so a missing relation never trips the resolver itself.
fn count_references(store: &CozoStore) -> i64 {
    store
        .run_query(
            "?[count(referrer_id)] := *references{referrer_id}",
            std::collections::BTreeMap::new(),
        )
        .ok()
        .and_then(|r| r.rows.into_iter().next())
        .and_then(|r| r.into_iter().next())
        .and_then(|v| match v {
            cozo::DataValue::Num(cozo::Num::Int(i)) => Some(i),
            _ => None,
        })
        .unwrap_or(-1)
}
