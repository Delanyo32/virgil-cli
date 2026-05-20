//! Built-in template discovery.
//!
//! Phase 1 of the Datalog-model migration: the previous 7 pure-Cozoscript
//! templates referenced the old `symbol` / `edge_*` shapes and have been
//! removed (per [ADR-0004]). Templates are rebuilt against the new
//! schema in Phase 7 (issue #17). Until then, this module returns an
//! empty surface — `cozoscript_template_names()` returns `[]` and
//! `load_cozoscript_template(_)` returns `None` for every name.
//!
//! The `include_dir!` machinery is kept so adding a new template back is
//! a one-file drop without re-wiring the module.
//!
//! [ADR-0004]: docs/adr/0004-templates-dark-during-migration.md

use include_dir::{Dir, include_dir};

static BUILTIN_TEMPLATES_DIR: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/src/queries/builtin");

/// Pure-Cozoscript template names (one `.cozoql` file each).
pub fn cozoscript_template_names() -> Vec<String> {
    BUILTIN_TEMPLATES_DIR
        .files()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("cozoql"))
        .filter_map(|f| f.path().file_stem().and_then(|s| s.to_str()))
        .map(|s| s.to_string())
        .collect()
}

/// Returns the Cozoscript body for a built-in template, or `None` if no
/// `.cozoql` file by that name is embedded.
pub fn load_cozoscript_template(name: &str) -> Option<&'static str> {
    let path = format!("{name}.cozoql");
    BUILTIN_TEMPLATES_DIR
        .get_file(&path)
        .and_then(|f| f.contents_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pure_templates_during_migration() {
        let names = cozoscript_template_names();
        assert!(
            names.is_empty(),
            "expected zero templates during migration (ADR-0004), got {names:?}"
        );
    }

    #[test]
    fn load_returns_none_for_any_name() {
        assert!(load_cozoscript_template("find_function_by_name").is_none());
        assert!(load_cozoscript_template("nonexistent").is_none());
    }
}
