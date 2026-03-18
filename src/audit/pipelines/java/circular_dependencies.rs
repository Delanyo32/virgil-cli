use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;
use super::primitives::{find_capture_index, node_text};

const HUB_MODULE_THRESHOLD: usize = 5;

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

pub struct CircularDependenciesPipeline {
    import_query: Arc<Query>,
}

impl CircularDependenciesPipeline {
    pub fn new() -> Result<Self> {
        // Capture entire import_declaration node to handle both regular and wildcard imports
        let import_query_str = r#"
(import_declaration) @import_decl
"#;
        let import_query = Query::new(&java_lang(), import_query_str)
            .with_context(|| "failed to compile import declaration query for Java circular deps")?;

        Ok(Self {
            import_query: Arc::new(import_query),
        })
    }

    /// Parse the import path from an import_declaration node text.
    /// e.g., "import com.app.service.OrderService;" -> "com.app.service.OrderService"
    /// e.g., "import static java.lang.Math.PI;" -> "java.lang.Math.PI"
    fn parse_import_path(text: &str) -> Option<String> {
        let text = text.trim();
        let text = text.strip_prefix("import")?.trim();
        // Strip static keyword if present
        let text = text.strip_prefix("static").unwrap_or(text).trim();
        let text = text.strip_suffix(';').unwrap_or(text).trim();
        if text.is_empty() {
            return None;
        }
        Some(text.to_string())
    }

    /// Extract the package prefix from a fully qualified import path.
    /// e.g., "com.app.service.OrderService" -> "com.app.service"
    /// e.g., "com.app.auth.*" -> "com.app.auth" (wildcard = already a package)
    fn extract_package(path: &str) -> String {
        // Wildcard imports: "com.app.auth.*" -> the path without .* IS the package
        if path.ends_with(".*") {
            return path.trim_end_matches(".*").to_string();
        }
        // Regular imports: remove the last segment (class name) to get the package
        if let Some(last_dot) = path.rfind('.') {
            path[..last_dot].to_string()
        } else {
            path.to_string()
        }
    }
}

impl Pipeline for CircularDependenciesPipeline {
    fn name(&self) -> &str {
        "circular_dependencies"
    }

    fn description(&self) -> &str {
        "Detects high fan-out imports that indicate circular dependency risk"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let mut cursor = QueryCursor::new();
        let decl_idx = find_capture_index(&self.import_query, "import_decl");
        let mut matches = cursor.matches(&self.import_query, root, source);

        let mut distinct_packages: HashSet<String> = HashSet::new();

        while let Some(m) = matches.next() {
            for cap in m.captures {
                if cap.index as usize == decl_idx {
                    let text = node_text(cap.node, source);
                    if let Some(path) = Self::parse_import_path(text) {
                        let package = Self::extract_package(&path);
                        distinct_packages.insert(package);
                    }
                }
            }
        }

        let fan_out = distinct_packages.len();

        // Pattern: hub_module_bidirectional
        // Flag files with high fan-out (importing from many distinct packages)
        if fan_out >= HUB_MODULE_THRESHOLD {
            let mut package_list: Vec<&str> = distinct_packages.iter().map(|s| s.as_str()).collect();
            package_list.sort();
            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: "circular_dependencies".to_string(),
                pattern: "hub_module_bidirectional".to_string(),
                message: format!(
                    "Module imports from {} distinct packages (threshold: {}): {}",
                    fan_out,
                    HUB_MODULE_THRESHOLD,
                    package_list.join(", ")
                ),
                snippet: String::new(),
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&java_lang()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = CircularDependenciesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_hub_module() {
        let src = r#"
import com.app.auth.AuthManager;
import com.app.billing.PaymentProcessor;
import com.app.cache.CacheLayer;
import com.app.config.AppConfig;
import com.app.database.Pool;
import com.app.logging.Logger;
import com.app.messaging.EventBus;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn no_hub_for_few_imports() {
        let src = r#"
import com.app.config.AppConfig;
import com.app.database.Pool;
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn deduplicates_same_package_imports() {
        let src = r#"
import com.app.auth.AuthManager;
import com.app.auth.TokenValidator;
import com.app.auth.SessionManager;
import com.app.config.AppConfig;
"#;
        // Only 2 distinct packages: com.app.auth and com.app.config
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn handles_wildcard_imports() {
        let src = r#"
import com.app.auth.*;
import com.app.billing.*;
import com.app.cache.*;
import com.app.config.*;
import com.app.database.*;
"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "hub_module_bidirectional"));
    }

    #[test]
    fn no_findings_for_empty_file() {
        let src = "public class Empty {}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
