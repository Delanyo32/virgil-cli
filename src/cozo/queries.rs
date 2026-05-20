//! Typed constructors for parameterised Cozoscript queries.
//!
//! The point of this module is to keep user-supplied values out of the
//! script text — every parameter goes through `BTreeMap<String, DataValue>`,
//! never through string formatting. The surface here grows in issue 05
//! when the user-facing query interface lands.

use std::collections::BTreeMap;

use cozo::DataValue;

pub const FIND_SYMBOL_BY_NAME: &str = "?[id, name, file_path, start_line, end_line] := \
     *symbol{id, name, file_path, start_line, end_line}, name = $name";

pub fn find_symbol_by_name_params(name: &str) -> BTreeMap<String, DataValue> {
    let mut p = BTreeMap::new();
    p.insert("name".to_string(), DataValue::from(name));
    p
}

pub const COUNT_SYMBOLS: &str = "?[c] := c := count(s), *symbol{id: s}";
pub const COUNT_FILES: &str = "?[c] := c := count(p), *file{path: p}";
pub const COUNT_CALL_EDGES: &str = "?[c] := c := count(x), *edge_calls{caller_id: x}";
