//! Issue #18.2a — innermost-binding (max start_byte) shadowing test.
//!
//! Hand-built factbase: a file-scope binding of `x` to symbol_A is
//! shadowed inside a function scope by a parameter binding of `x` to
//! symbol_B. An occurrence of `x` inside the function must resolve to
//! symbol_B — NOT symbol_A.
//!
//! Without the innermost-binding rule, the resolver emits two
//! references rows (one per candidate). With the rule, exactly one
//! row whose `referent_id = symbol_B`.

use std::collections::BTreeMap;

use cozo::DataValue;
use virgil_cli::cozo::{CozoStore, CozoWriter, resolver};

#[test]
fn inner_binding_shadows_outer() {
    let store = CozoStore::open_in_memory().expect("open store");
    let mut w = CozoWriter::new();

    // Two symbols. The outer `x` (file-scope variable) and the inner
    // `x` (function parameter).
    w.push_symbol(
        "f.rs|1|0|x|variable",
        "variable",
        "x",
        "x",
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
        "f.rs|3|6|x|parameter",
        "parameter",
        "x",
        "x",
        "rust",
        "private",
        "f.rs",
        Some("f.rs|2|0|f|function"),
        false,
        false,
        false,
        false,
        false,
    );
    // The enclosing function symbol — referrer_id of the occurrence.
    w.push_symbol(
        "f.rs|2|0|f|function",
        "function",
        "f",
        "f",
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

    // Scopes: file + function (function is nested in file).
    w.push_scope("f.rs|0|file", None, "f.rs", "file", 0, 100);
    w.push_scope(
        "f.rs|20|function",
        Some("f.rs|0|file"),
        "f.rs",
        "function",
        20,
        90,
    );

    // Bindings. File-scope `x` binding at start_byte 0; function-scope
    // parameter binding at start_byte 25 (inside the function body).
    w.push_binding(
        "f.rs|0|file",
        "x",
        0,
        Some("f.rs|1|0|x|variable"),
        "definition",
    );
    w.push_binding(
        "f.rs|20|function",
        "x",
        25,
        Some("f.rs|3|6|x|parameter"),
        "parameter",
    );

    // Occurrence of `x` inside the function body (start_byte 50).
    w.push_occurrence(
        "f.rs|50|x|read",
        "x",
        "f.rs",
        50,
        51,
        Some("f.rs|2|0|f|function"),
        "f.rs|20|function",
        "read",
    );

    w.flush(&store).expect("flush facts");

    // Run the resolver.
    resolver::resolve_references(&store).expect("resolve");

    // Assert: exactly one references row, pointing at the parameter
    // binding (symbol_B), not the file-scope variable (symbol_A).
    let rows = store
        .run_query(
            "?[r, sb, ref] := *references{referrer_id: r, site_start_byte: sb, referent_id: ref}",
            BTreeMap::new(),
        )
        .expect("query");
    assert_eq!(
        rows.rows.len(),
        1,
        "expected one resolved row, got {:?}",
        rows.rows
    );
    let row = &rows.rows[0];
    assert_eq!(row[0], DataValue::from("f.rs|2|0|f|function"));
    assert_eq!(row[1], DataValue::from(50));
    assert_eq!(row[2], DataValue::from("f.rs|3|6|x|parameter"));
}
