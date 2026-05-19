//! Built-in template discovery. Templates live in `src/queries/builtin/`
//! and are embedded into the binary at compile time via `include_dir`.

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
    fn the_seven_pure_templates_are_discoverable() {
        let names = cozoscript_template_names();
        for expected in [
            "find_callers",
            "find_callees",
            "find_cycles",
            "find_function_by_name",
            "export_surface",
            "import_depth",
            "unused_symbols",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing template {expected}; have: {names:?}"
            );
        }
    }

    #[test]
    fn load_returns_body_for_known_template() {
        let body = load_cozoscript_template("find_function_by_name").expect("body");
        assert!(body.contains("?[name, kind, file_path"));
        assert!(load_cozoscript_template("nonexistent").is_none());
    }
}
