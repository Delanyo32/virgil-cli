//! Issue #18.2e — transitive re-export test (single-hop chain).
//!
//! file_a has `use file_b::foo;` recorded as a binding with
//! `symbol_id = null` (the extractor doesn't follow). file_b
//! re-exports file_c's foo with a binding pointing at file_c's
//! symbol. The resolver chases file_a → file_b's binding → file_c's
//! symbol via the `imports` relation.
//!
//! Multi-hop (file_a → file_b → file_c → file_d) chains require a
//! recursive `file_resolves` rule which cozo currently rejects in
//! the form we tried; that's deferred to a follow-up.

use std::collections::BTreeMap;

use cozo::DataValue;
use virgil_cli::cozo::{CozoStore, CozoWriter, resolver};

#[test]
fn single_hop_reexport_chains_through_imports() {
    let store = CozoStore::open_in_memory().expect("open");
    let mut w = CozoWriter::new();

    // file_c defines `foo`.
    w.push_symbol(
        "file_c.rs|1|0|foo|function",
        "function",
        "foo",
        "foo",
        "rust",
        "public",
        "file_c.rs",
        None,
        false,
        false,
        false,
        false,
        true,
    );
    // Enclosing function in file_a where the occurrence lives.
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

    // Scopes for all three files.
    w.push_scope("file_a.rs|0|file", None, "file_a.rs", "file", 0, 200);
    w.push_scope("file_b.rs|0|file", None, "file_b.rs", "file", 0, 200);
    w.push_scope("file_c.rs|0|file", None, "file_c.rs", "file", 0, 200);

    // file_a: `use file_b::foo;` → binding(name=foo, symbol_id=null)
    w.push_binding("file_a.rs|0|file", "foo", 10, None, "import");
    // file_b: `pub use file_c::foo;` → binding(name=foo, symbol_id=file_c::foo)
    w.push_binding(
        "file_b.rs|0|file",
        "foo",
        20,
        Some("file_c.rs|1|0|foo|function"),
        "import_alias",
    );

    // imports: file_a → file_b → file_c (single chain).
    w.push_imports("file_a.rs", "file_b.rs");
    w.push_imports("file_b.rs", "file_c.rs");

    // Occurrence of `foo` in file_a.
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
    // Chains through to file_c's foo, NOT file_b (which has no foo Symbol).
    assert_eq!(row[1], DataValue::from("file_c.rs|1|0|foo|function"));
}
