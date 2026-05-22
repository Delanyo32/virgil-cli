//! Issue #18.2c — wildcard import expansion test.
//!
//! File A has `use file_b::*;` (modelled as a wildcard_import binding
//! in A's file scope). File B exports a symbol `foo`. An occurrence
//! of `foo` in A — with NO scoped binding — must resolve to B's foo
//! via the wildcard + imports + exported-symbol join.

use std::collections::BTreeMap;

use cozo::DataValue;
use virgil_cli::cozo::{CozoStore, CozoWriter, resolver};

#[test]
fn wildcard_import_resolves_to_exported_symbol() {
    let store = CozoStore::open_in_memory().expect("open");
    let mut w = CozoWriter::new();

    // file_b exports `foo`.
    w.push_symbol(
        "file_b.rs|1|0|foo|function",
        "function",
        "foo",
        "foo",
        "rust",
        "public",
        "file_b.rs",
        None,
        false,
        false,
        false,
        false,
        true, // exported
    );

    // Enclosing function symbol in file_a where the occurrence lives.
    w.push_symbol(
        "file_a.rs|5|0|caller|function",
        "function",
        "caller",
        "caller",
        "rust",
        "private",
        "file_a.rs",
        None,
        false,
        false,
        false,
        false,
        false,
    );

    // file_a scope (the wildcard binding lives here).
    w.push_scope("file_a.rs|0|file", None, "file_a.rs", "file", 0, 200);
    // file_b scope (where `foo` actually lives — needed for completeness;
    // resolver doesn't require it but populate() would emit it).
    w.push_scope("file_b.rs|0|file", None, "file_b.rs", "file", 0, 200);

    // Wildcard binding: `use file_b::*` records one row with
    // binding_kind = wildcard_import. name = "*" by convention.
    w.push_binding("file_a.rs|0|file", "*", 10, None, "wildcard_import");

    // imports relation: file_a imports file_b.
    w.push_imports("file_a.rs", "file_b.rs");

    // Occurrence of `foo` in file_a (no local binding for foo).
    w.push_occurrence(
        "file_a.rs|100|foo|call",
        "foo",
        "file_a.rs",
        100,
        103,
        Some("file_a.rs|5|0|caller|function"),
        "file_a.rs|0|file",
        "call",
    );

    w.flush(&store).expect("flush");
    resolver::resolve_references(&store).expect("resolve");

    let rows = store
        .run_query(
            "?[mi, ref] := *references{site_start_byte: 100, match_index: mi, referent_id: ref}",
            BTreeMap::new(),
        )
        .expect("query");
    assert_eq!(rows.rows.len(), 1, "expected one row, got {:?}", rows.rows);
    let row = &rows.rows[0];
    assert_eq!(row[0], DataValue::from(0));
    assert_eq!(row[1], DataValue::from("file_b.rs|1|0|foo|function"));
}

#[test]
fn wildcard_skipped_when_scoped_binding_present() {
    // file_a wildcard-imports file_b, but ALSO has its own `foo`
    // binding in file scope. The scoped binding must win; wildcard
    // does not contribute.
    let store = CozoStore::open_in_memory().expect("open");
    let mut w = CozoWriter::new();

    // file_b exports `foo`.
    w.push_symbol(
        "file_b.rs|1|0|foo|function",
        "function",
        "foo",
        "foo",
        "rust",
        "public",
        "file_b.rs",
        None,
        false,
        false,
        false,
        false,
        true,
    );
    // file_a's own `foo`.
    w.push_symbol(
        "file_a.rs|2|0|foo|function",
        "function",
        "foo",
        "foo",
        "rust",
        "private",
        "file_a.rs",
        None,
        false,
        false,
        false,
        false,
        false,
    );
    w.push_symbol(
        "file_a.rs|5|0|caller|function",
        "function",
        "caller",
        "caller",
        "rust",
        "private",
        "file_a.rs",
        None,
        false,
        false,
        false,
        false,
        false,
    );

    w.push_scope("file_a.rs|0|file", None, "file_a.rs", "file", 0, 200);
    w.push_scope("file_b.rs|0|file", None, "file_b.rs", "file", 0, 200);

    // Local scoped binding for foo.
    w.push_binding(
        "file_a.rs|0|file",
        "foo",
        50,
        Some("file_a.rs|2|0|foo|function"),
        "definition",
    );
    // Wildcard binding for file_b::*.
    w.push_binding("file_a.rs|0|file", "*", 10, None, "wildcard_import");
    w.push_imports("file_a.rs", "file_b.rs");

    w.push_occurrence(
        "file_a.rs|100|foo|call",
        "foo",
        "file_a.rs",
        100,
        103,
        Some("file_a.rs|5|0|caller|function"),
        "file_a.rs|0|file",
        "call",
    );

    w.flush(&store).expect("flush");
    resolver::resolve_references(&store).expect("resolve");

    let rows = store
        .run_query(
            "?[ref] := *references{site_start_byte: 100, referent_id: ref}",
            BTreeMap::new(),
        )
        .expect("query");
    assert_eq!(rows.rows.len(), 1, "expected one row, got {:?}", rows.rows);
    // Local foo wins, not the wildcard-imported one.
    assert_eq!(
        rows.rows[0][0],
        DataValue::from("file_a.rs|2|0|foo|function")
    );
}
