use std::sync::Arc;

use anyhow::Result;
use tree_sitter::Query;

use super::primitives;
use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::{is_main_function_rust, is_test_context_rust, is_test_file};

pub struct PanicDetectionPipeline {
    method_query: Arc<Query>,
    macro_query: Arc<Query>,
}

impl PanicDetectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: primitives::compile_method_call_query()?,
            macro_query: primitives::compile_macro_invocation_query()?,
        })
    }

    fn message_for_pattern(pattern: &str) -> &'static str {
        match pattern {
            "unwrap" => "call to .unwrap() may panic at runtime",
            "expect" => "call to .expect() may panic at runtime",
            "panic" => "explicit panic!() call",
            "todo" => "todo!() should be resolved before production",
            "unimplemented" => "unimplemented!() will panic if reached",
            "unreachable" => "unreachable!() will panic if reached",
            _ => "potential panic",
        }
    }

    /// Return true if the line before `row` (0-based) is a `// SAFETY:` comment.
    fn has_safety_comment(source: &[u8], row: usize) -> bool {
        if row == 0 {
            return false;
        }
        std::str::from_utf8(source)
            .unwrap_or("")
            .lines()
            .nth(row - 1)
            .map(|l| l.trim_start().starts_with("// SAFETY:"))
            .unwrap_or(false)
    }
}

impl GraphPipeline for PanicDetectionPipeline {
    fn name(&self) -> &str {
        "panic_detection"
    }

    fn description(&self) -> &str {
        "Detects patterns that may cause panics: .unwrap(), .expect(), panic!(), todo!(), unimplemented!(), unreachable!()"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let tree = ctx.tree;
        let source = ctx.source;
        let file_path = ctx.file_path;

        // Skip test files entirely
        if is_test_file(file_path) {
            return Vec::new();
        }

        // Skip build scripts — unwrap()/expect() are idiomatic there
        if file_path.ends_with("build.rs") {
            return Vec::new();
        }

        let mut findings = Vec::new();

        let method_matches =
            primitives::find_method_calls(tree, source, &self.method_query, &["unwrap", "expect"]);

        for m in method_matches {
            // Find the node at this position for context checks
            let node = tree.root_node().descendant_for_point_range(
                tree_sitter::Point {
                    row: (m.line - 1) as usize,
                    column: (m.column - 1) as usize,
                },
                tree_sitter::Point {
                    row: (m.line - 1) as usize,
                    column: (m.column - 1) as usize,
                },
            );

            if let Some(n) = node {
                // Skip .unwrap()/.expect() inside test contexts
                if is_test_context_rust(n, source) {
                    continue;
                }
                // Skip when receiver chain includes .lock(), .read(), or .write()
                // (Mutex::lock().unwrap() / RwLock::read().unwrap() / RwLock::write().unwrap() are idiomatic)
                if (m.name == "unwrap" || m.name == "expect")
                    && (m.text.contains(".lock()")
                        || m.text.contains(".read()")
                        || m.text.contains(".write()"))
                {
                    continue;
                }
                // Skip if the preceding line has a // SAFETY: comment
                if Self::has_safety_comment(source, (m.line - 1) as usize) {
                    continue;
                }
                // Downgrade severity in main() to info
                let severity = if is_main_function_rust(n, source) {
                    "info"
                } else {
                    "warning"
                };
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: m.line,
                    column: m.column,
                    severity: severity.to_string(),
                    pipeline: self.name().to_string(),
                    pattern: m.name.clone(),
                    message: Self::message_for_pattern(&m.name).to_string(),
                    snippet: m.text,
                });
            }
        }

        let macro_matches = primitives::find_macro_invocations(
            tree,
            source,
            &self.macro_query,
            &["panic", "todo", "unimplemented", "unreachable"],
        );

        for m in macro_matches {
            let node = tree.root_node().descendant_for_point_range(
                tree_sitter::Point {
                    row: (m.line - 1) as usize,
                    column: (m.column - 1) as usize,
                },
                tree_sitter::Point {
                    row: (m.line - 1) as usize,
                    column: (m.column - 1) as usize,
                },
            );

            if let Some(n) = node {
                // Skip macros inside test contexts
                if is_test_context_rust(n, source) {
                    continue;
                }
            }

            // Severity graduation:
            // - unreachable!() documents impossible states → info
            // - todo!() / unimplemented!() are active tech debt → warning
            // - panic!() is an explicit failure path → warning
            let severity = match m.name.as_str() {
                "unreachable" => "info",
                _ => "warning",
            };

            findings.push(AuditFinding {
                file_path: file_path.to_string(),
                line: m.line,
                column: m.column,
                severity: severity.to_string(),
                pipeline: self.name().to_string(),
                pattern: m.name.clone(),
                message: Self::message_for_pattern(&m.name).to_string(),
                snippet: m.text,
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        parse_and_check_path(source, "test.rs")
    }

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Rust.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = PanicDetectionPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = std::collections::HashMap::new();
        let ctx = crate::audit::pipeline::GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    #[test]
    fn detects_all_panic_patterns() {
        let src = r#"
fn example() {
    let a = Some(1).unwrap();
    let b = Some(2).expect("msg");
    panic!("oops");
    todo!();
    unimplemented!();
    unreachable!();
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 6);

        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(patterns.contains(&"unwrap"));
        assert!(patterns.contains(&"expect"));
        assert!(patterns.contains(&"panic"));
        assert!(patterns.contains(&"todo"));
        assert!(patterns.contains(&"unimplemented"));
        assert!(patterns.contains(&"unreachable"));
    }

    #[test]
    fn clean_code_no_findings() {
        let src = r#"
fn example() -> Result<i32, String> {
    let a = Some(1).unwrap_or(0);
    let b = Some(2).unwrap_or_default();
    let c = match Some(3) {
        Some(v) => v,
        None => 0,
    };
    Ok(a + b + c)
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn findings_have_correct_metadata() {
        let src = r#"fn process() { Some(1).unwrap(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.file_path, "test.rs");
        assert_eq!(f.pipeline, "panic_detection");
        assert_eq!(f.pattern, "unwrap");
        assert_eq!(f.severity, "warning");
        assert_eq!(f.message, "call to .unwrap() may panic at runtime");
    }

    #[test]
    fn unwrap_in_main_is_info() {
        let src = r#"fn main() { Some(1).unwrap(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn snippet_captures_full_expression() {
        let src = r#"fn process() { Some(1).unwrap(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].snippet.contains("Some(1).unwrap()"));
    }

    #[test]
    fn build_rs_excluded() {
        let src = r#"fn main() { let x = std::env::var("KEY").unwrap(); }"#;
        let findings = parse_and_check_path(src, "build.rs");
        assert!(findings.is_empty(), "build.rs should be excluded");
    }

    #[test]
    fn rwlock_read_not_flagged() {
        let src = r#"fn f(s: &std::sync::RwLock<i32>) { let _g = s.read().unwrap(); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "rwlock.read().unwrap() is idiomatic");
    }

    #[test]
    fn rwlock_write_not_flagged() {
        let src = r#"fn f(s: &std::sync::RwLock<i32>) { let _g = s.write().unwrap(); }"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "rwlock.write().unwrap() is idiomatic");
    }

    #[test]
    fn safety_comment_suppresses_unwrap() {
        let src = "fn f() {\n    // SAFETY: guaranteed Some\n    val.unwrap();\n}\n";
        let findings = parse_and_check(src);
        assert!(findings.is_empty(), "SAFETY comment should suppress");
    }

    #[test]
    fn unreachable_is_info() {
        let src = r#"fn f() { unreachable!(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "info");
    }

    #[test]
    fn todo_is_warning() {
        let src = r#"fn f() { todo!(); }"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }
}
