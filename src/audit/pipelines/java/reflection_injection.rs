use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{
    compile_method_invocation_with_object_query, extract_snippet, find_capture_index, node_text,
};

pub struct ReflectionInjectionPipeline {
    method_query: Arc<Query>,
}

impl ReflectionInjectionPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            method_query: compile_method_invocation_with_object_query()?,
        })
    }
}

impl Pipeline for ReflectionInjectionPipeline {
    fn name(&self) -> &str {
        "reflection_injection"
    }

    fn description(&self) -> &str {
        "Detects reflection and injection risks: Class.forName, Method.invoke, ScriptEngine.eval, JNDI lookup"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.method_query, tree.root_node(), source);

        let obj_idx = find_capture_index(&self.method_query, "object");
        let method_idx = find_capture_index(&self.method_query, "method_name");
        let args_idx = find_capture_index(&self.method_query, "args");
        let inv_idx = find_capture_index(&self.method_query, "invocation");

        while let Some(m) = matches.next() {
            let obj_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == obj_idx)
                .map(|c| c.node);
            let method_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == method_idx)
                .map(|c| c.node);
            let args_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == args_idx)
                .map(|c| c.node);
            let inv_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == inv_idx)
                .map(|c| c.node);

            if let (Some(obj_node), Some(method_node), Some(args_node), Some(inv_node)) =
                (obj_node, method_node, args_node, inv_node)
            {
                let obj_name = node_text(obj_node, source);
                let method_name = node_text(method_node, source);

                // Class.forName(param) — unsafe dynamic class loading
                if obj_name == "Class" && method_name == "forName"
                    && let Some(first_arg) = args_node.named_child(0)
                        && first_arg.kind() != "string_literal" {
                            let start = inv_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "error".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unsafe_class_loading".to_string(),
                                message: "Class.forName() with dynamic input — validate against allowlist".to_string(),
                                snippet: extract_snippet(source, inv_node, 1),
                            });
                        }

                // method.invoke() — unsafe reflective method invocation
                if method_name == "invoke" {
                    let inv_text = node_text(inv_node, source);
                    if inv_text.contains("Method") || inv_text.contains("method") {
                        let start = inv_node.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "error".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "unsafe_method_invoke".to_string(),
                            message: "Method.invoke() with potentially user-controlled method — validate method name".to_string(),
                            snippet: extract_snippet(source, inv_node, 1),
                        });
                    }
                }

                // ScriptEngine.eval(param) — script injection
                if method_name == "eval" {
                    let inv_text = node_text(inv_node, source);
                    if (inv_text.contains("engine")
                        || inv_text.contains("Engine")
                        || inv_text.contains("script"))
                        && let Some(first_arg) = args_node.named_child(0)
                            && first_arg.kind() != "string_literal" {
                                let start = inv_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "script_eval_injection".to_string(),
                                    message: "ScriptEngine.eval() with dynamic input — potential code execution".to_string(),
                                    snippet: extract_snippet(source, inv_node, 1),
                                });
                            }
                }

                // InitialContext.lookup(param) — JNDI injection
                if method_name == "lookup" {
                    let inv_text = node_text(inv_node, source);
                    if (inv_text.contains("Context") || inv_text.contains("ctx"))
                        && let Some(first_arg) = args_node.named_child(0)
                            && first_arg.kind() != "string_literal" {
                                let start = inv_node.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "error".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "jndi_injection".to_string(),
                                    message: "JNDI lookup() with dynamic input — potential remote code execution".to_string(),
                                    snippet: extract_snippet(source, inv_node, 1),
                                });
                            }
                }
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ReflectionInjectionPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.java")
    }

    #[test]
    fn detects_dynamic_class_forname() {
        let src = r#"class Foo {
    void load(String className) {
        Class.forName(className);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unsafe_class_loading");
    }

    #[test]
    fn detects_script_eval() {
        let src = r#"class Foo {
    void run(String code) {
        scriptEngine.eval(code);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "script_eval_injection");
    }

    #[test]
    fn detects_jndi_lookup() {
        let src = r#"class Foo {
    void load(String name) {
        ctx.lookup(name);
    }
}"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "jndi_injection");
    }

    #[test]
    fn ignores_static_class_forname() {
        let src = r#"class Foo {
    void load() {
        Class.forName("com.example.MyClass");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn clean_no_reflection() {
        let src = r#"class Foo {
    void bar() {
        System.out.println("hello");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
