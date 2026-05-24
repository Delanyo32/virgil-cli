//! Built-in template discovery.
//!
//! Built-in pure-Cozoscript templates live under `src/queries/builtin/`,
//! one `.cozoql` file per template. The `include_dir!` macro embeds them
//! at build time so they ship inside the binary. To add a new template,
//! drop a `<name>.cozoql` file next to the existing ones — no Rust glue
//! required.
//!
//! Rust-side handlers (templates that need source access beyond what's
//! in the fact store) live in `rust_templates.rs` and short-circuit the
//! Cozoscript path; their names are kept disjoint from the `.cozoql`
//! file names.

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
    fn ships_the_expected_cozoscript_templates() {
        let mut names = cozoscript_template_names();
        names.sort();
        assert_eq!(
            names,
            vec![
                "export_surface".to_string(),
                "find_callees".to_string(),
                "find_callers".to_string(),
                "find_cycles".to_string(),
                "find_function_by_name".to_string(),
                "find_implementations_of".to_string(),
                "import_depth".to_string(),
            ],
        );
    }

    #[test]
    fn unknown_name_loads_to_none() {
        assert!(load_cozoscript_template("nonexistent").is_none());
    }

    #[test]
    fn known_name_loads_a_non_empty_body() {
        let body = load_cozoscript_template("find_function_by_name")
            .expect("find_function_by_name template");
        assert!(
            body.contains("?["),
            "expected a Cozoscript head, got {body}"
        );
    }
}
