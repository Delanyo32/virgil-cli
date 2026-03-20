use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{
    compile_method_call_security_query, compile_property_assignment_query, extract_snippet,
    find_capture_index, is_safe_literal, node_text,
};

pub struct XssDomInjectionPipeline {
    prop_assign_query: Arc<Query>,
    method_call_query: Arc<Query>,
}

impl XssDomInjectionPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            prop_assign_query: compile_property_assignment_query(language)?,
            method_call_query: compile_method_call_security_query(language)?,
        })
    }
}

impl Pipeline for XssDomInjectionPipeline {
    fn name(&self) -> &str {
        "xss_dom_injection"
    }

    fn description(&self) -> &str {
        "Detects XSS via innerHTML/outerHTML assignment, insertAdjacentHTML, document.write with non-literal content"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Check property assignments (innerHTML, outerHTML)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.prop_assign_query, tree.root_node(), source);
            let prop_idx = find_capture_index(&self.prop_assign_query, "prop");
            let value_idx = find_capture_index(&self.prop_assign_query, "value");
            let assign_idx = find_capture_index(&self.prop_assign_query, "assign");

            while let Some(m) = matches.next() {
                let prop_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == prop_idx)
                    .map(|c| c.node);
                let value_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == value_idx)
                    .map(|c| c.node);
                let assign_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == assign_idx)
                    .map(|c| c.node);

                if let (Some(prop), Some(value), Some(assign)) =
                    (prop_node, value_node, assign_node)
                {
                    let prop_name = node_text(prop, source);
                    if (prop_name == "innerHTML" || prop_name == "outerHTML")
                        && !is_safe_literal(value, source)
                    {
                        let start = assign.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "innerHTML_injection".to_string(),
                            message: format!(
                                "`{}` set with non-literal value — potential XSS",
                                prop_name
                            ),
                            snippet: extract_snippet(source, assign, 1),
                        });
                    }
                }
            }
        }

        // Check method calls (insertAdjacentHTML, document.write, document.writeln)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
            let obj_idx = find_capture_index(&self.method_call_query, "obj");
            let method_idx = find_capture_index(&self.method_call_query, "method");
            let args_idx = find_capture_index(&self.method_call_query, "args");
            let call_idx = find_capture_index(&self.method_call_query, "call");

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
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(obj), Some(method), Some(args), Some(call)) =
                    (obj_node, method_node, args_node, call_node)
                {
                    let method_name = node_text(method, source);
                    let obj_name = node_text(obj, source);

                    // insertAdjacentHTML with non-literal second arg
                    if method_name == "insertAdjacentHTML"
                        && let Some(second_arg) = args.named_child(1)
                            && !is_safe_literal(second_arg, source) {
                                let start = call.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "insertAdjacentHTML_injection".to_string(),
                                    message:
                                        "`insertAdjacentHTML` with non-literal HTML — potential XSS"
                                            .to_string(),
                                    snippet: extract_snippet(source, call, 1),
                                });
                            }

                    // document.write / document.writeln
                    if obj_name == "document"
                        && (method_name == "write" || method_name == "writeln")
                        && let Some(first_arg) = args.named_child(0)
                            && !is_safe_literal(first_arg, source) {
                                let start = call.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "document_write_injection".to_string(),
                                    message: format!(
                                        "`document.{}` with non-literal content — potential XSS",
                                        method_name
                                    ),
                                    snippet: extract_snippet(source, call, 1),
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

    fn parse_and_check(source: &str) -> Vec<AuditFinding> {
        let lang = Language::JavaScript;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = XssDomInjectionPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_innerhtml_with_variable() {
        let src = "el.innerHTML = userInput;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "innerHTML_injection");
    }

    #[test]
    fn detects_outerhtml_with_variable() {
        let src = "el.outerHTML = data;";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "innerHTML_injection");
    }

    #[test]
    fn ignores_innerhtml_with_literal() {
        let src = r#"el.innerHTML = "<p>safe</p>";"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_insert_adjacent_html() {
        let src = r#"el.insertAdjacentHTML("beforeend", userInput);"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "insertAdjacentHTML_injection");
    }

    #[test]
    fn detects_document_write() {
        let src = "document.write(content);";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "document_write_injection");
    }

    #[test]
    fn ignores_document_write_with_literal() {
        let src = r#"document.write("hello");"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }
}
