//! Issue #18.2b — overload `match_index` numbering test.
//!
//! Two bindings of `foo` exist in the SAME (file) scope, each mapping
//! to a different symbol. An occurrence of `foo` should resolve to
//! BOTH candidates, with `match_index = 0` and `match_index = 1`
//! (lexicographic on referent_id per ADR-0003).

use std::collections::BTreeMap;

use cozo::DataValue;
use virgil_cli::cozo::{CozoStore, CozoWriter, resolver};

#[test]
fn two_overload_candidates_emit_match_indices() {
    let store = CozoStore::open_in_memory().expect("open store");
    let mut w = CozoWriter::new();

    // Two overload symbols sym_A and sym_B (sym_A < sym_B lex).
    for (id, kind) in [
        ("f.rs|10|0|foo|method", "method"),
        ("f.rs|20|0|foo|method", "method"),
    ] {
        w.push_symbol(
            id,
            kind,
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
    }
    // Enclosing function symbol for the occurrence.
    w.push_symbol(
        "f.rs|30|0|caller|function",
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

    // Single file scope.
    w.push_scope("f.rs|0|file", None, "f.rs", "file", 0, 200);

    // Two bindings to the same name `foo` in the same scope, each
    // mapping to a different symbol. Different start_byte values
    // (each binding has its own location).
    w.push_binding(
        "f.rs|0|file",
        "foo",
        10,
        Some("f.rs|10|0|foo|method"),
        "definition",
    );
    w.push_binding(
        "f.rs|0|file",
        "foo",
        20,
        Some("f.rs|20|0|foo|method"),
        "definition",
    );

    // Occurrence of `foo` somewhere in the file (start_byte 50).
    w.push_occurrence(
        "f.rs|50|foo|call",
        "foo",
        "f.rs",
        50,
        53,
        Some("f.rs|30|0|caller|function"),
        "f.rs|0|file",
        "call",
    );

    w.flush(&store).expect("flush");
    resolver::resolve_references(&store).expect("resolve");

    // Expect two references rows, match_index 0 and 1.
    let rows = store
        .run_query(
            "?[mi, ref] := \
             *references{site_start_byte: 50, match_index: mi, referent_id: ref}",
            BTreeMap::new(),
        )
        .expect("query");

    let mut pairs: Vec<(i64, String)> = rows
        .rows
        .iter()
        .map(|r| {
            let mi = match &r[0] {
                DataValue::Num(cozo::Num::Int(i)) => *i,
                other => panic!("int expected, got {other:?}"),
            };
            let ref_s = match &r[1] {
                DataValue::Str(s) => s.to_string(),
                other => panic!("str expected, got {other:?}"),
            };
            (mi, ref_s)
        })
        .collect();
    pairs.sort();

    assert_eq!(
        pairs,
        vec![
            (0, "f.rs|10|0|foo|method".to_string()),
            (1, "f.rs|20|0|foo|method".to_string()),
        ],
        "expected match_index 0 → first overload, 1 → second"
    );
}
