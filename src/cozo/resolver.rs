//! Issue #16 ADR-0005 Cozoscript resolver. Materialises the
//! `references` relation from `occurrence` / `scope` / `binding` /
//! `imports` facts emitted by per-language extractors.
//!
//! See `docs/resolution.md` for the spec.
//!
//! The resolver runs as a sequence of small Cozo programs, each
//! writing its output into a temp stored relation (`rsv_*`). Each stage
//! is wrapped in its own `info_span!` with a row count, so a single
//! run produces a per-stage breakdown in the trace output.
//!
//! The final `match_index` assignment + write to `references` lives in
//! Rust: Cozo's optimiser can't simplify the `count(s) where s <= sym`
//! self-join, so for large workloads it costs orders of magnitude more
//! than reading `rsv_resolved` into Rust, grouping by `occ`, sorting
//! each group by `sym`, and batch-writing. The null-fallback (rows for
//! occurrences with no resolution candidate) is still emitted via
//! Cozoscript — that one's a single linear pass with no self-join.
//!
//! Note: Cozo silently drops relations whose names begin with `_`, so
//! the temp relations use a `rsv_` prefix instead.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use cozo::{DataValue, Num};
use tracing::{debug, info_span};

use super::CozoStore;

const STAGE_ANCESTOR: &str = r#"
sa[s, t] := *scope{id: s}, t = s
sa[s, t] := sa[s, mid], *scope{id: mid, parent_id: t}, t != null
?[scope_id, ancestor_id] := sa[scope_id, ancestor_id]
:replace rsv_ancestor {scope_id: String, ancestor_id: String}
"#;

const STAGE_INNERMOST: &str = r#"
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
"#;

// Two-stage chain. The original single-pass chain query was an 8-way
// join; even with restructuring, Cozo's planner kept generating huge
// intermediate sets because it expanded occurrence × null_binding
// before pruning by `imports`. On real workloads `imports` is by far
// the most selective input (often <1% of bindings), so we fold it into
// the pre-stage.
const STAGE_CHAIN_ELIGIBLE: &str = r#"
?[file_path, name, target_file] :=
    *imports{importer_file_id: file_path, imported_id: target_file},
    *scope{id: sid, file_path},
    *binding{scope_id: sid, name, symbol_id: cnb, binding_kind: cbk},
    cnb == null,
    cbk != "wildcard_import"
:replace rsv_chain_eligible {file_path: String, name: String, target_file: String}
"#;

const STAGE_CHAIN: &str = r#"
?[occ, sym] :=
    *rsv_chain_eligible{file_path: cof, name: cn, target_file: tf},
    *occurrence{id: occ, name: cn, file_path: cof},
    *scope{id: ts, file_path: tf},
    *binding{scope_id: ts, name: cn, symbol_id: sym, binding_kind: tbk},
    sym != null,
    tbk != "wildcard_import",
    *symbol{id: sym}
:replace rsv_chain {occ: String, sym: String}
"#;

// Wildcard takes the same imports-first restructure as chain.
const STAGE_WILDCARD_ELIGIBLE: &str = r#"
?[file_path, target_file] :=
    *imports{importer_file_id: file_path, imported_id: target_file},
    *scope{id: sid, file_path},
    *binding{scope_id: sid, binding_kind: "wildcard_import"}
:replace rsv_wildcard_eligible {file_path: String, target_file: String}
"#;

const STAGE_WILDCARD: &str = r#"
has_scoped[occ] := *rsv_innermost{occ}
?[occ, sym] :=
    *rsv_wildcard_eligible{file_path: of, target_file: tf},
    *occurrence{id: occ, name: n, file_path: of},
    *symbol{id: sym, name: n, file_path: tf, exported: true},
    not has_scoped[occ]
:replace rsv_wildcard {occ: String, sym: String}
"#;

const STAGE_RESOLVED: &str = r#"
res[occ, sym] := *rsv_innermost{occ, sym}
res[occ, sym] := *rsv_chain{occ, sym}
res[occ, sym] := *rsv_wildcard{occ, sym}
?[occ, sym] := res[occ, sym]
:replace rsv_resolved {occ: String, sym: String}
"#;

// Join resolved with occurrence metadata so the Rust finalize step can
// pull a single row per (occ, sym) candidate carrying everything it
// needs to write into `references`. Excludes rows without an
// enclosing referrer — those wouldn't survive the write filter anyway.
const STAGE_RESOLVED_JOINED: &str = r#"
?[occ, sym, referrer_id, site_file, site_start_byte, ref_kind] :=
    *rsv_resolved{occ, sym},
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null
:replace rsv_resolved_joined {occ: String, sym: String =>
    referrer_id: String, site_file: String,
    site_start_byte: Int, ref_kind: String}
"#;

// Null fallback for occurrences that no resolution path matched. Pure
// Cozoscript — single linear pass, no self-join.
const STAGE_WRITE_NULL: &str = r#"
has_any[occ] := *rsv_resolved{occ}
?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] :=
    *occurrence{id: occ, file_path: site_file, start_byte: site_start_byte,
                enclosing_symbol_id: referrer_id, occurrence_kind: ref_kind},
    referrer_id != null,
    not has_any[occ],
    match_index = 0,
    referent_id = null
:put references {referrer_id, site_file, site_start_byte, match_index => referent_id, ref_kind}
"#;

const CLEANUP_REMOVE: &[&str] = &[
    "::remove rsv_ancestor",
    "::remove rsv_innermost",
    "::remove rsv_chain_eligible",
    "::remove rsv_chain",
    "::remove rsv_wildcard_eligible",
    "::remove rsv_wildcard",
    "::remove rsv_resolved",
    "::remove rsv_resolved_joined",
];

/// Batch size for the Rust-side `:put references` chunked write.
/// Matches the size used by [`super::writer`].
const FINALIZE_BATCH: usize = 10_000;

/// Materialise the `references` relation by running the staged
/// Cozoscript resolver against the current fact relations.
///
/// No-ops cheaply when the `occurrence` relation is empty.
pub fn resolve_references(store: &CozoStore) -> Result<()> {
    let n = {
        let _s = info_span!("cozo.resolver.count_probe").entered();
        let occ = count_relation(store, "occurrence", "id");
        let scopes = count_relation(store, "scope", "id");
        let bindings = count_relation(store, "binding", "name");
        let imports = count_relation(store, "imports", "importer_file_id");
        debug!(
            occurrences = occ,
            scopes, bindings, imports, "resolver input cardinality"
        );
        occ
    };
    if n == 0 {
        return Ok(());
    }

    {
        let _s = info_span!("cozo.resolver.run", occurrences = n).entered();
        run_stage(
            store,
            "ancestor",
            STAGE_ANCESTOR,
            "rsv_ancestor",
            "scope_id",
        )?;
        run_stage(store, "innermost", STAGE_INNERMOST, "rsv_innermost", "occ")?;
        run_stage(
            store,
            "chain_eligible",
            STAGE_CHAIN_ELIGIBLE,
            "rsv_chain_eligible",
            "file_path",
        )?;
        run_stage(store, "chain", STAGE_CHAIN, "rsv_chain", "occ")?;
        run_stage(
            store,
            "wildcard_eligible",
            STAGE_WILDCARD_ELIGIBLE,
            "rsv_wildcard_eligible",
            "file_path",
        )?;
        run_stage(store, "wildcard", STAGE_WILDCARD, "rsv_wildcard", "occ")?;
        run_stage(store, "resolved", STAGE_RESOLVED, "rsv_resolved", "occ")?;
        run_stage(
            store,
            "resolved_joined",
            STAGE_RESOLVED_JOINED,
            "rsv_resolved_joined",
            "occ",
        )?;
        finalize_resolved(store)?;
        {
            let _w = info_span!("cozo.resolver.stage", stage = "write_null").entered();
            store
                .run_script(STAGE_WRITE_NULL, BTreeMap::new())
                .map_err(|e| anyhow::anyhow!("resolver write_null stage failed: {e}"))?;
            debug!(
                references_rows = count_relation(store, "references", "referrer_id"),
                "write_null stage complete"
            );
        }
    }

    // Best-effort cleanup of the temp relations. A failure here doesn't
    // invalidate the run — the references relation is already populated.
    {
        let _s = info_span!("cozo.resolver.cleanup").entered();
        for stmt in CLEANUP_REMOVE {
            let _ = store.run_script(stmt, BTreeMap::new());
        }
    }

    Ok(())
}

/// Pull the joined resolved rows into Rust, group by `occ`, sort each
/// group by `sym` to assign `match_index`, and batch-write to
/// `references`. Replaces the Cozoscript `count(s) where s <= sym`
/// self-join — that one scaled quadratically in `|rsv_resolved|` on the
/// 5k-file workload (4 minutes on ext), whereas this is linear after a
/// per-group sort.
fn finalize_resolved(store: &CozoStore) -> Result<()> {
    let _s = info_span!("cozo.resolver.stage", stage = "finalize").entered();

    let rows = {
        let _r = info_span!("cozo.resolver.finalize.read").entered();
        store
            .run_query(
                "?[occ, sym, referrer_id, site_file, site_start_byte, ref_kind] := \
                 *rsv_resolved_joined{occ, sym, referrer_id, site_file, site_start_byte, ref_kind}",
                BTreeMap::new(),
            )
            .map_err(|e| anyhow::anyhow!("finalize: read rsv_resolved_joined: {e}"))?
            .rows
    };
    debug!(rows_in = rows.len(), "finalize read");

    // Group (sym, referrer, file, byte, kind) tuples per occ.
    type Candidate = (String, String, String, i64, String);
    let mut by_occ: HashMap<String, Vec<Candidate>> = HashMap::with_capacity(rows.len() / 2);
    for row in rows {
        let occ = take_str(&row, 0)?;
        let sym = take_str(&row, 1)?;
        let referrer = take_str(&row, 2)?;
        let file = take_str(&row, 3)?;
        let byte = take_int(&row, 4)?;
        let kind = take_str(&row, 5)?;
        by_occ
            .entry(occ)
            .or_default()
            .push((sym, referrer, file, byte, kind));
    }

    // Each candidate becomes one references row: (referrer_id,
    // site_file, site_start_byte, match_index, referent_id, ref_kind).
    let mut out: Vec<Vec<DataValue>> = Vec::with_capacity(by_occ.values().map(Vec::len).sum());
    for (_occ, mut group) in by_occ {
        group.sort_by(|a, b| a.0.cmp(&b.0));
        for (mi, (sym, referrer, file, byte, kind)) in group.into_iter().enumerate() {
            out.push(vec![
                DataValue::from(referrer),
                DataValue::from(file),
                DataValue::Num(Num::Int(byte)),
                DataValue::Num(Num::Int(mi as i64)),
                DataValue::from(sym),
                DataValue::from(kind),
            ]);
        }
    }
    let total = out.len();
    debug!(rows_out = total, "finalize grouped");

    {
        let _w = info_span!("cozo.resolver.finalize.write").entered();
        for chunk in out.chunks(FINALIZE_BATCH) {
            let batch: Vec<DataValue> = chunk.iter().map(|r| DataValue::List(r.clone())).collect();
            let mut params = BTreeMap::new();
            params.insert("rows".to_string(), DataValue::List(batch));
            store
                .run_script(
                    "?[referrer_id, site_file, site_start_byte, match_index, referent_id, ref_kind] <- $rows \
                     :put references {referrer_id, site_file, site_start_byte, match_index => referent_id, ref_kind}",
                    params,
                )
                .map_err(|e| anyhow::anyhow!("finalize: put references: {e}"))?;
        }
    }
    debug!(
        references_rows = count_relation(store, "references", "referrer_id"),
        rows_written = total,
        "finalize stage complete"
    );

    Ok(())
}

fn take_str(row: &[DataValue], i: usize) -> Result<String> {
    match row.get(i) {
        Some(DataValue::Str(s)) => Ok(s.to_string()),
        Some(v) => Err(anyhow::anyhow!(
            "finalize: expected string at col {i}, got {v:?}"
        )),
        None => Err(anyhow::anyhow!("finalize: missing col {i}")),
    }
}

fn take_int(row: &[DataValue], i: usize) -> Result<i64> {
    match row.get(i) {
        Some(DataValue::Num(Num::Int(n))) => Ok(*n),
        Some(v) => Err(anyhow::anyhow!(
            "finalize: expected int at col {i}, got {v:?}"
        )),
        None => Err(anyhow::anyhow!("finalize: missing col {i}")),
    }
}

fn run_stage(
    store: &CozoStore,
    name: &'static str,
    script: &str,
    output: &str,
    count_col: &str,
) -> Result<()> {
    let _s = info_span!("cozo.resolver.stage", stage = name).entered();
    store
        .run_script(script, BTreeMap::new())
        .map_err(|e| anyhow::anyhow!("resolver stage `{name}` failed: {e}"))?;
    debug!(
        stage = name,
        rows = count_relation(store, output, count_col),
        "stage complete"
    );
    Ok(())
}

/// Best-effort row count via `?[count(<col>)] := *<rel>{<col>}`. Returns
/// `-1` on any error so a missing relation never trips the resolver.
fn count_relation(store: &CozoStore, rel: &str, col: &str) -> i64 {
    let script = format!("?[count({col})] := *{rel}{{{col}}}");
    store
        .run_query(&script, BTreeMap::new())
        .ok()
        .and_then(|r| r.rows.into_iter().next())
        .and_then(|r| r.into_iter().next())
        .and_then(|v| match v {
            DataValue::Num(Num::Int(i)) => Some(i),
            _ => None,
        })
        .unwrap_or(-1)
}
