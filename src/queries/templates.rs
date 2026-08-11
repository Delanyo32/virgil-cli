//! Built-in template discovery.
//!
//! Built-in pure-SQL templates live under `src/queries/builtin/`, one
//! `.sql` file per template, embedded at build time so they ship inside
//! the binary. To add a new template, drop a `<name>.sql` file next to
//! the existing ones and add one line to `BUILTIN_TEMPLATES`.
//!
//! Rust-side handlers (templates that need source access beyond what's
//! in the fact store) live in `rust_templates.rs` and short-circuit
//! the SQL path; their names are kept disjoint from the `.sql` file
//! names.

static BUILTIN_TEMPLATES: &[(&str, &str)] = &[
    ("export_surface", include_str!("builtin/export_surface.sql")),
    ("find_callees", include_str!("builtin/find_callees.sql")),
    ("find_callers", include_str!("builtin/find_callers.sql")),
    ("find_cycles", include_str!("builtin/find_cycles.sql")),
    (
        "find_function_by_name",
        include_str!("builtin/find_function_by_name.sql"),
    ),
    (
        "find_implementations_of",
        include_str!("builtin/find_implementations_of.sql"),
    ),
    ("import_depth", include_str!("builtin/import_depth.sql")),
];

/// Pure-SQL template names (one `.sql` file each).
pub fn sql_template_names() -> Vec<String> {
    BUILTIN_TEMPLATES
        .iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Returns the SQL body for a built-in template, or `None` if no `.sql`
/// file by that name is embedded.
pub fn load_sql_template(name: &str) -> Option<&'static str> {
    BUILTIN_TEMPLATES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, sql)| *sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_the_expected_sql_templates() {
        let mut names = sql_template_names();
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
        assert!(load_sql_template("nonexistent").is_none());
    }

    #[test]
    fn known_name_loads_a_non_empty_body() {
        let body =
            load_sql_template("find_function_by_name").expect("find_function_by_name template");
        assert!(
            body.to_uppercase().contains("SELECT"),
            "expected SQL SELECT, got {body}"
        );
    }
}
