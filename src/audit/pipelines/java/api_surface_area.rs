use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::primitives::{find_capture_index, has_modifier, node_text};
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

const EXCESSIVE_API_MIN_SYMBOLS: usize = 10;
const EXCESSIVE_API_EXPORT_RATIO: f64 = 0.8;

const JAVA_MEMBER_KINDS: &[&str] = &[
    "method_declaration",
    "field_declaration",
    "constructor_declaration",
];

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

pub struct ApiSurfaceAreaPipeline {
    class_query: Arc<Query>,
}

impl ApiSurfaceAreaPipeline {
    pub fn new() -> Result<Self> {
        let class_query_str = r#"
[
  (class_declaration
    name: (identifier) @class_name
    body: (class_body) @class_body) @class_def
  (interface_declaration
    name: (identifier) @class_name
    body: (interface_body) @class_body) @class_def
  (enum_declaration
    name: (identifier) @class_name
    body: (enum_body) @class_body) @class_def
]
"#;
        let class_query = Query::new(&java_lang(), class_query_str)
            .with_context(|| "failed to compile class query for Java API surface")?;

        Ok(Self {
            class_query: Arc::new(class_query),
        })
    }
}

impl Pipeline for ApiSurfaceAreaPipeline {
    fn name(&self) -> &str {
        "api_surface_area"
    }

    fn description(&self) -> &str {
        "Detects excessive public API and leaky abstraction boundaries"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let root = tree.root_node();

        let mut cursor = QueryCursor::new();
        let class_name_idx = find_capture_index(&self.class_query, "class_name");
        let class_body_idx = find_capture_index(&self.class_query, "class_body");
        let class_def_idx = find_capture_index(&self.class_query, "class_def");

        let mut matches = cursor.matches(&self.class_query, root, source);
        let mut reported_classes = HashSet::new();

        while let Some(m) = matches.next() {
            let mut class_name = "";
            let mut class_line = 0u32;
            let mut class_node = None;
            let mut body_node = None;

            for cap in m.captures {
                if cap.index as usize == class_name_idx {
                    class_name = node_text(cap.node, source);
                    class_line = cap.node.start_position().row as u32 + 1;
                }
                if cap.index as usize == class_body_idx {
                    body_node = Some(cap.node);
                }
                if cap.index as usize == class_def_idx {
                    class_node = Some(cap.node);
                }
            }

            if class_name.is_empty() || reported_classes.contains(class_name) {
                continue;
            }

            let class_decl = match class_node {
                Some(n) => n,
                None => continue,
            };
            let body = match body_node {
                Some(n) => n,
                None => continue,
            };

            // Only process top-level classes (direct children of program)
            if class_decl.parent().is_none_or(|p| p.kind() != "program") {
                continue;
            }

            reported_classes.insert(class_name.to_string());
            let is_public_class = has_modifier(class_decl, source, "public");

            // Count total members and public members
            let mut total_members = 0usize;
            let mut public_members = 0usize;
            let mut body_cursor = body.walk();
            for member in body.children(&mut body_cursor) {
                let kind = member.kind();
                if JAVA_MEMBER_KINDS.contains(&kind) {
                    total_members += 1;
                    if has_modifier(member, source, "public") {
                        public_members += 1;
                    }
                }
            }

            // Pattern 1: excessive_public_api
            if total_members >= EXCESSIVE_API_MIN_SYMBOLS {
                let ratio = public_members as f64 / total_members as f64;
                if ratio > EXCESSIVE_API_EXPORT_RATIO {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: class_line,
                        column: 1,
                        severity: "info".to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "excessive_public_api".to_string(),
                        message: format!(
                            "Class `{}` exports {}/{} members ({:.0}% exported, threshold: >{}%)",
                            class_name,
                            public_members,
                            total_members,
                            ratio * 100.0,
                            (EXCESSIVE_API_EXPORT_RATIO * 100.0) as u32
                        ),
                        snippet: String::new(),
                    });
                }
            }

            // Pattern 2: leaky_abstraction_boundary
            // Public class with public non-final fields
            if is_public_class {
                let mut leaky_field_names = Vec::new();
                let mut field_cursor = body.walk();
                for member in body.children(&mut field_cursor) {
                    if member.kind() == "field_declaration"
                        && has_modifier(member, source, "public")
                            && !has_modifier(member, source, "final")
                        {
                            // Extract field name
                            let mut inner_cursor = member.walk();
                            for child in member.children(&mut inner_cursor) {
                                if child.kind() == "variable_declarator"
                                    && let Some(name_node) = child.child_by_field_name("name") {
                                        leaky_field_names
                                            .push(node_text(name_node, source).to_string());
                                    }
                            }
                        }
                }

                if !leaky_field_names.is_empty() {
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: class_line,
                        column: 1,
                        severity: "warning".to_string(),
                        pipeline: "api_surface_area".to_string(),
                        pattern: "leaky_abstraction_boundary".to_string(),
                        message: format!(
                            "Public class `{}` has public non-final field(s): {} — consider encapsulating with methods",
                            class_name,
                            leaky_field_names.join(", ")
                        ),
                        snippet: String::new(),
                    });
                }
            }
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
        let pipeline = ApiSurfaceAreaPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_excessive_public_api() {
        let mut src = String::from("public class Foo {\n");
        // 10 public + 1 private = 11 total, 10/11 = 91% > 80%
        for i in 0..10 {
            src.push_str(&format!("    public void method_{}() {{}}\n", i));
        }
        src.push_str("    private void privateMethod() {}\n");
        src.push_str("}\n");
        let findings = parse_and_check(&src);
        assert!(findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn no_excessive_api_below_threshold() {
        let src = r#"
public class Foo {
    public void foo() {}
    public void bar() {}
    private void baz() {}
    private void qux() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(!findings.iter().any(|f| f.pattern == "excessive_public_api"));
    }

    #[test]
    fn detects_leaky_abstraction() {
        let src = r#"
public class SessionManager {
    public int maxSessions;
    public long timeoutMs;
    public void getSession() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_private_fields() {
        let src = r#"
public class SessionManager {
    private int maxSessions;
    private long timeoutMs;
    public void getSession() {}
}
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_public_final_fields() {
        let src = r#"
public class Config {
    public final int maxRetries = 5;
    public final String name = "test";
}
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }

    #[test]
    fn no_leaky_for_non_public_class() {
        let src = r#"
class InternalHelper {
    public int count;
}
"#;
        let findings = parse_and_check(src);
        assert!(
            !findings
                .iter()
                .any(|f| f.pattern == "leaky_abstraction_boundary")
        );
    }
}
