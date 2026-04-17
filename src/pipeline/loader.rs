//! JSON audit file loading and discovery.
//!
//! Discovery order: project-local (`.virgil/audits/`) → user-global (`~/.virgil-cli/audits/`) → built-ins.
//! Files with the same pipeline name AND the same language filter deduplicate (project-local wins).
//! Files with the same pipeline name but different language filters are all included (per-language variants).

use crate::pipeline::dsl::GraphStage;
use include_dir::{include_dir, Dir};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// JsonAuditFile — represents a JSON audit pipeline file
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct JsonAuditFile {
    /// Rust pipeline name this overrides (e.g. "circular_dependencies")
    pub pipeline: String,
    /// Audit category: "architecture", "code-quality", "security", etc.
    pub category: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Default severity used by Flag stages when no severity_map matches
    #[serde(default)]
    pub severity: Option<String>,
    /// Language filter. If None, applies to all languages.
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    /// The pipeline stages to execute
    pub graph: Vec<GraphStage>,
}

// ---------------------------------------------------------------------------
// Built-in audit files (embedded at compile time)
// ---------------------------------------------------------------------------

static BUILTIN_AUDITS_DIR: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/src/audit/builtin");

fn builtin_audits() -> Vec<JsonAuditFile> {
    BUILTIN_AUDITS_DIR
        .files()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|f| {
            let src = match f.contents_utf8() {
                Some(s) => s,
                None => {
                    eprintln!("Warning: built-in audit file {:?} is not valid UTF-8", f.path());
                    return None;
                }
            };
            match serde_json::from_str::<JsonAuditFile>(src) {
                Ok(audit) => Some(audit),
                Err(e) => {
                    eprintln!("Warning: failed to parse built-in audit {:?}: {e}", f.path());
                    None
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Compute a deduplication key for a JSON audit file.
///
/// Two files with the same pipeline name but different language filters are
/// considered distinct variants and should both run (e.g., 8 per-language
/// sync_blocking_in_async files). Two files with the same pipeline name AND
/// the same language filter are duplicates — project-local wins over built-in.
fn dedup_key(audit: &JsonAuditFile) -> String {
    let lang_key = match &audit.languages {
        Some(langs) => {
            let mut sorted = langs.clone();
            sorted.sort();
            sorted.join(",")
        }
        None => "*".to_string(),
    };
    format!("{}:{}", audit.pipeline, lang_key)
}

/// Discover JSON audit files from: project-local → user-global → built-ins.
/// Files with the same pipeline name AND the same language filter deduplicate
/// (project-local beats user-global beats built-in). Files with the same
/// pipeline name but DIFFERENT language filters are all included — this
/// supports per-language variants of a pipeline (e.g., sync_blocking_in_async).
pub fn discover_json_audits(project_dir: Option<&std::path::Path>) -> Vec<JsonAuditFile> {
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut result: Vec<JsonAuditFile> = Vec::new();

    // 1. Project-local: {project_dir}/.virgil/audits/*.json
    if let Some(dir) = project_dir {
        let audit_dir = dir.join(".virgil").join("audits");
        load_json_audits_from_dir(&audit_dir, &mut seen_keys, &mut result);
    }

    // 2. User-global: ~/.virgil-cli/audits/*.json
    if let Some(home) = dirs::home_dir() {
        let audit_dir = home.join(".virgil-cli").join("audits");
        load_json_audits_from_dir(&audit_dir, &mut seen_keys, &mut result);
    }

    // 3. Built-ins (embedded in binary)
    for audit in builtin_audits() {
        let key = dedup_key(&audit);
        if seen_keys.insert(key) {
            result.push(audit);
        }
    }

    result
}

fn load_json_audits_from_dir(
    dir: &std::path::Path,
    seen: &mut std::collections::HashSet<String>,
    result: &mut Vec<JsonAuditFile>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // directory doesn't exist or isn't readable — silently skip
    };
    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort(); // deterministic load order
    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<JsonAuditFile>(&content) {
                Ok(audit) => {
                    let key = dedup_key(&audit);
                    if seen.insert(key) {
                        result.push(audit);
                    }
                }
                Err(e) => eprintln!("Warning: failed to parse audit file {:?}: {e}", path),
            },
            Err(e) => eprintln!("Warning: failed to read audit file {:?}: {e}", path),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_audits_returns_four() {
        let audits = builtin_audits();
        assert!(audits.len() >= 44, "Expected at least 44 built-in audits, got {}", audits.len());
        for audit in &audits {
            assert!(
                !audit.graph.is_empty(),
                "Built-in audit '{}' has empty graph",
                audit.pipeline
            );
        }
    }

    #[test]
    fn test_builtin_audit_pipeline_names() {
        let audits = builtin_audits();
        let names: Vec<&str> = audits.iter().map(|a| a.pipeline.as_str()).collect();
        // Per-language pipeline names (representative samples from the 44 built-ins)
        assert!(names.contains(&"circular_dependencies_rust"), "missing circular_dependencies_rust in {:?}", names);
        assert!(names.contains(&"dependency_graph_depth_javascript"), "missing dependency_graph_depth_javascript in {:?}", names);
        assert!(names.contains(&"api_surface_area_python"), "missing api_surface_area_python in {:?}", names);
        assert!(names.contains(&"module_size_distribution_go"), "missing module_size_distribution_go in {:?}", names);
    }

    #[test]
    fn test_discover_json_audits_no_project_dir_returns_builtins() {
        let audits = discover_json_audits(None);
        assert!(
            audits.len() >= 4,
            "Expected at least 4 built-ins from discover_json_audits(None), got {}",
            audits.len()
        );
    }

    #[test]
    fn test_discover_json_audits_project_local_overrides_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let audit_dir = tmp.path().join(".virgil").join("audits");
        std::fs::create_dir_all(&audit_dir).unwrap();

        // Write a project-local file that overrides circular_dependencies
        let override_content = r#"{
            "pipeline": "circular_dependencies",
            "category": "architecture",
            "description": "project-local override",
            "graph": [
                {"select": "file"},
                {"flag": {"pattern": "circular_dependency", "message": "Override", "severity": "warning"}}
            ]
        }"#;
        std::fs::write(audit_dir.join("circular_dependencies.json"), override_content).unwrap();

        let audits = discover_json_audits(Some(tmp.path()));

        // The project-local override should appear before (and instead of) the built-in
        let circular = audits
            .iter()
            .filter(|a| a.pipeline == "circular_dependencies")
            .collect::<Vec<_>>();
        assert_eq!(circular.len(), 1, "Should only have one circular_dependencies entry");
        assert_eq!(
            circular[0].description.as_deref(),
            Some("project-local override"),
            "Project-local override should take precedence"
        );

        // The project-local file should be first in the result list
        assert_eq!(
            audits[0].pipeline, "circular_dependencies",
            "Project-local audit should appear first"
        );

        // Per-language built-ins should still be present alongside the override
        let pipeline_names: Vec<&str> = audits.iter().map(|a| a.pipeline.as_str()).collect();
        assert!(pipeline_names.contains(&"circular_dependencies"));
        assert!(pipeline_names.contains(&"circular_dependencies_rust"), "per-language built-in should still be present");
        assert!(pipeline_names.contains(&"module_size_distribution_go"), "per-language built-in should still be present");
    }

    #[test]
    fn test_discover_json_audits_invalid_json_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let audit_dir = tmp.path().join(".virgil").join("audits");
        std::fs::create_dir_all(&audit_dir).unwrap();

        // Write an invalid JSON file
        std::fs::write(audit_dir.join("bad.json"), "{ this is not valid json }").unwrap();

        // Should not panic; built-ins should still be returned
        let audits = discover_json_audits(Some(tmp.path()));
        assert!(
            audits.len() >= 4,
            "Should still return built-ins even with invalid project-local JSON"
        );
    }

    #[test]
    fn test_discover_json_audits_nonexistent_project_dir_is_ok() {
        let nonexistent = std::path::Path::new("/tmp/__virgil_nonexistent_test_dir__");
        // Should not panic
        let audits = discover_json_audits(Some(nonexistent));
        assert!(audits.len() >= 4);
    }

    #[test]
    fn test_load_json_audits_from_dir_deduplicates() {
        let tmp = tempfile::tempdir().unwrap();

        // Write two files with the same pipeline name
        let content_a = r#"{
            "pipeline": "my_pipeline",
            "category": "architecture",
            "description": "first",
            "graph": [{"select": "file"}]
        }"#;
        let content_b = r#"{
            "pipeline": "my_pipeline",
            "category": "architecture",
            "description": "second",
            "graph": [{"select": "file"}]
        }"#;
        std::fs::write(tmp.path().join("a_pipeline.json"), content_a).unwrap();
        std::fs::write(tmp.path().join("b_pipeline.json"), content_b).unwrap();

        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        load_json_audits_from_dir(tmp.path(), &mut seen, &mut result);

        // Only the first alphabetically should be loaded (dedup)
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].description.as_deref(), Some("first"));
    }
}
