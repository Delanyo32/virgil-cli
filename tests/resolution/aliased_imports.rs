//! Issue #18.2d — aliased import test.
//!
//! `import { foo as bar }` (or any language's equivalent) creates an
//! `import_alias` binding with `name = "bar"` and `symbol_id =
//! foo's_id`. An occurrence of `bar` in the importing file should
//! resolve to foo's original symbol.
//!
//! No resolver code change required — the existing
//! `innermost_binding` rule treats `import_alias` like any other
//! non-wildcard binding. This test pins that behavior.

use std::collections::BTreeMap;

use cozo::DataValue;
use virgil_cli::cozo::{CozoStore, CozoWriter, resolver};

#[test]
fn import_alias_resolves_to_original_symbol() {
    let store = CozoStore::open_in_memory().expect("open");
    let mut w = CozoWriter::new();

    // The original symbol — `foo` in file_b.
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

    // file_a's scope and the alias binding: `use file_b::foo as bar`
    // → binding(name=bar, symbol_id=foo's_id, kind=import_alias).
    w.push_scope("file_a.rs|0|file", None, "file_a.rs", "file", 0, 200);
    w.push_binding(
        "file_a.rs|0|file",
        "bar",
        10,
        Some("file_b.rs|1|0|foo|function"),
        "import_alias",
    );

    // Occurrence of `bar` in file_a.
    w.push_occurrence(
        "file_a.rs|100|bar|call",
        "bar",
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
    // The alias resolves to the original `foo` symbol — NOT to a
    // synthetic "bar" symbol.
    assert_eq!(row[1], DataValue::from("file_b.rs|1|0|foo|function"));
}

#[test]
fn alias_shadows_same_name_import() {
    // file_a imports both `foo` (as itself) AND `something as foo`
    // (alias to a different target). The alias binding sits at a
    // higher start_byte, so per the innermost-binding rule its
    // start_byte wins over the prior import — but per #18.2b's
    // innermost-SCOPE rule, both candidates live in the same scope
    // and survive as overloads.
    //
    // This test pins the overload outcome: two references rows,
    // match_index 0 + 1, deterministic lex order on referent_id.
    let store = CozoStore::open_in_memory().expect("open");
    let mut w = CozoWriter::new();

    for (id, file) in [
        ("file_b.rs|1|0|foo|function", "file_b.rs"),
        ("file_c.rs|1|0|something|function", "file_c.rs"),
    ] {
        w.push_symbol(
            id,
            "function",
            if id.contains("something") { "something" } else { "foo" },
            if id.contains("something") { "something" } else { "foo" },
            "rust",
            "public",
            file,
            None,
            false,
            false,
            false,
            false,
            true,
        );
    }
    w.push_symbol(
        "file_a.rs|10|0|caller|function",
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
    // Regular import: name = "foo" → file_b's foo.
    w.push_binding(
        "file_a.rs|0|file",
        "foo",
        5,
        Some("file_b.rs|1|0|foo|function"),
        "import",
    );
    // Alias import: also name = "foo" (shadowing!) → file_c's something.
    w.push_binding(
        "file_a.rs|0|file",
        "foo",
        20,
        Some("file_c.rs|1|0|something|function"),
        "import_alias",
    );

    w.push_occurrence(
        "file_a.rs|100|foo|call",
        "foo",
        "file_a.rs",
        100,
        103,
        Some("file_a.rs|10|0|caller|function"),
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

    let mut pairs: Vec<(i64, String)> = rows
        .rows
        .iter()
        .map(|r| {
            let mi = match &r[0] {
                DataValue::Num(cozo::Num::Int(i)) => *i,
                _ => panic!("expected int"),
            };
            let s = match &r[1] {
                DataValue::Str(s) => s.to_string(),
                _ => panic!("expected str"),
            };
            (mi, s)
        })
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            (0, "file_b.rs|1|0|foo|function".to_string()),
            (1, "file_c.rs|1|0|something|function".to_string()),
        ],
        "both candidates survive at mi 0 and 1 lex-order on referent_id"
    );
}
