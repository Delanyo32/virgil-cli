//! Issue #18.2f — unresolved occurrence emits exactly one null
//! references row.
//!
//! An occurrence with NO matching scoped binding, NO wildcard match,
//! and NO chain hit should emit exactly one row with
//! `referent_id = null` at `match_index = 0`. This pins the
//! fallback branch of the resolver.

use std::collections::BTreeMap;

use cozo::DataValue;
use virgil_cli::cozo::{CozoStore, CozoWriter, resolver};

#[test]
fn unresolved_occurrence_emits_one_null_row() {
    let store = CozoStore::open_in_memory().expect("open");
    let mut w = CozoWriter::new();

    // Enclosing function (the referrer_id of the occurrence).
    w.push_symbol(
        "f.rs|1|0|caller|function",
        "function",
        "caller",
        "caller",
        "rust",
        "private",
        "f.rs",
        None,
        false,
        false,
        false,
        false,
        false,
    );

    // File scope. No bindings (no definitions, no imports, no
    // wildcards) for the name we'll reference.
    w.push_scope("f.rs|0|file", None, "f.rs", "file", 0, 200);

    // Occurrence of `mystery` inside the caller. Nothing in the
    // factbase resolves it.
    w.push_occurrence(
        "f.rs|50|mystery|read",
        "mystery",
        "f.rs",
        50,
        57,
        Some("f.rs|1|0|caller|function"),
        "f.rs|0|file",
        "read",
    );

    w.flush(&store).expect("flush");
    resolver::resolve_references(&store).expect("resolve");

    let rows = store
        .run_query(
            "?[mi, ref] := *references{site_start_byte: 50, match_index: mi, referent_id: ref}",
            BTreeMap::new(),
        )
        .expect("query");
    assert_eq!(
        rows.rows.len(),
        1,
        "expected exactly one row, got {:?}",
        rows.rows
    );
    let row = &rows.rows[0];
    assert_eq!(row[0], DataValue::from(0), "match_index = 0");
    assert!(
        matches!(row[1], DataValue::Null),
        "referent_id is null, got {:?}",
        row[1]
    );
}

#[test]
fn resolved_occurrence_does_not_emit_null_row() {
    // Inverse pin: when an occurrence DOES resolve, the unresolved
    // branch must NOT also emit a null row.
    let store = CozoStore::open_in_memory().expect("open");
    let mut w = CozoWriter::new();

    w.push_symbol(
        "f.rs|2|0|foo|function",
        "function",
        "foo",
        "foo",
        "rust",
        "private",
        "f.rs",
        None,
        false,
        false,
        false,
        false,
        false,
    );
    w.push_symbol(
        "f.rs|10|0|caller|function",
        "function",
        "caller",
        "caller",
        "rust",
        "private",
        "f.rs",
        None,
        false,
        false,
        false,
        false,
        false,
    );

    w.push_scope("f.rs|0|file", None, "f.rs", "file", 0, 200);
    w.push_binding(
        "f.rs|0|file",
        "foo",
        2,
        Some("f.rs|2|0|foo|function"),
        "definition",
    );

    w.push_occurrence(
        "f.rs|50|foo|call",
        "foo",
        "f.rs",
        50,
        53,
        Some("f.rs|10|0|caller|function"),
        "f.rs|0|file",
        "call",
    );

    w.flush(&store).expect("flush");
    resolver::resolve_references(&store).expect("resolve");

    let rows = store
        .run_query(
            "?[ref] := *references{site_start_byte: 50, referent_id: ref}",
            BTreeMap::new(),
        )
        .expect("query");
    // Exactly one row — the resolved one. No additional null fallback.
    assert_eq!(
        rows.rows.len(),
        1,
        "expected exactly one row, got {:?}",
        rows.rows
    );
    assert_eq!(rows.rows[0][0], DataValue::from("f.rs|2|0|foo|function"));
}
