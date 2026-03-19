use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Container methods that grow the container, problematic inside loops
const GROWTH_METHODS: &[&str] = &[
    "push_back",
    "emplace_back",
    "push_front",
    "emplace_front",
    "insert",
    "emplace",
];

fn cpp_lang() -> tree_sitter::Language {
    Language::Cpp.tree_sitter_language()
}

pub struct MemoryLeakIndicatorsPipeline {
    fn_query: Arc<Query>,
    new_query: Arc<Query>,
    loop_query: Arc<Query>,
    call_query: Arc<Query>,
    class_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        let fn_query_str = r#"
(function_definition
  declarator: (_) @declarator
  body: (compound_statement) @fn_body) @fn_def
"#;
        let fn_query = Query::new(&cpp_lang(), fn_query_str)
            .with_context(|| "failed to compile function query for C++ memory_leak_indicators")?;

        let new_query_str = r#"
(new_expression) @new_expr
"#;
        let new_query = Query::new(&cpp_lang(), new_query_str).with_context(
            || "failed to compile new_expression query for C++ memory_leak_indicators",
        )?;

        let loop_query_str = r#"
[
  (for_statement body: (_) @loop_body) @loop_expr
  (for_range_loop body: (_) @loop_body) @loop_expr
  (while_statement body: (_) @loop_body) @loop_expr
  (do_statement body: (_) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&cpp_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for C++ memory_leak_indicators")?;

        let call_query_str = r#"
(call_expression
  function: (_) @fn_name
  arguments: (argument_list) @args) @call
"#;
        let call_query = Query::new(&cpp_lang(), call_query_str)
            .with_context(|| "failed to compile call query for C++ memory_leak_indicators")?;

        let class_query_str = r#"
(class_specifier
  name: (type_identifier) @class_name
  body: (field_declaration_list) @class_body) @class_def
"#;
        let class_query = Query::new(&cpp_lang(), class_query_str)
            .with_context(|| "failed to compile class query for C++ memory_leak_indicators")?;

        Ok(Self {
            fn_query: Arc::new(fn_query),
            new_query: Arc::new(new_query),
            loop_query: Arc::new(loop_query),
            call_query: Arc::new(call_query),
            class_query: Arc::new(class_query),
        })
    }

    /// Check if a `new` expression is wrapped in a smart pointer
    fn is_smart_pointer_wrapped(new_node: tree_sitter::Node, source: &[u8]) -> bool {
        // Walk up to find if this `new` is inside a smart pointer constructor
        let mut current = new_node.parent();
        while let Some(parent) = current {
            let text = node_text(parent, source);
            if text.contains("unique_ptr")
                || text.contains("shared_ptr")
                || text.contains("make_unique")
                || text.contains("make_shared")
                || text.contains("auto_ptr")
                || text.contains("QScopedPointer")
                || text.contains("CComPtr")
            {
                return true;
            }
            // Stop walking up at statement boundaries
            if parent.kind() == "expression_statement"
                || parent.kind() == "declaration"
                || parent.kind() == "return_statement"
            {
                // Check the full statement text for smart pointer wrappers
                let stmt_text = node_text(parent, source);
                if stmt_text.contains("unique_ptr")
                    || stmt_text.contains("shared_ptr")
                    || stmt_text.contains("make_unique")
                    || stmt_text.contains("make_shared")
                    || stmt_text.contains("auto_ptr")
                {
                    return true;
                }
                break;
            }
            current = parent.parent();
        }
        false
    }

    /// Check if a function body has a matching delete for raw new allocations
    fn has_matching_delete(body: tree_sitter::Node, source: &[u8], is_array: bool) -> bool {
        let body_text = node_text(body, source);
        if is_array {
            body_text.contains("delete[]") || body_text.contains("delete []")
        } else {
            body_text.contains("delete ")
        }
    }

    /// Detect raw `new` without smart pointer wrapping
    fn check_raw_new(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut fn_matches = cursor.matches(&self.fn_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.fn_query, "fn_body");

        while let Some(m) = fn_matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let Some(body) = body_node {
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut new_matches =
                    inner_cursor.matches(&self.new_query, tree.root_node(), source);

                let new_idx = find_capture_index(&self.new_query, "new_expr");

                while let Some(nm) = new_matches.next() {
                    let new_node = nm
                        .captures
                        .iter()
                        .find(|c| c.index as usize == new_idx)
                        .map(|c| c.node);

                    if let Some(new_n) = new_node {
                        if Self::is_smart_pointer_wrapped(new_n, source) {
                            continue;
                        }

                        let new_text = node_text(new_n, source);
                        let is_array = new_text.contains("new[") || new_text.contains("new [");

                        if is_array {
                            if !Self::has_matching_delete(body, source, true) {
                                let start = new_n.start_position();
                                findings.push(AuditFinding {
                                    file_path: file_path.to_string(),
                                    line: start.row as u32 + 1,
                                    column: start.column as u32 + 1,
                                    severity: "warning".to_string(),
                                    pipeline: self.name().to_string(),
                                    pattern: "raw_new_array_without_delete".to_string(),
                                    message: "`new[]` without matching `delete[]` — use `std::vector` or `std::unique_ptr<T[]>` instead".to_string(),
                                    snippet: extract_snippet(source, new_n, 1),
                                });
                            }
                        } else if !Self::has_matching_delete(body, source, false) {
                            let start = new_n.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "raw_new_without_delete".to_string(),
                                message: "raw `new` without matching `delete` and not wrapped in smart pointer — potential memory leak".to_string(),
                                snippet: extract_snippet(source, new_n, 1),
                            });
                        }
                    }
                }
            }
        }

        findings
    }

    /// Detect unbounded container growth inside loops
    fn check_unbounded_growth(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.loop_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.loop_query, "loop_body");
        let loop_idx = find_capture_index(&self.loop_query, "loop_expr");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let loop_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == loop_idx)
                .map(|c| c.node);

            if let (Some(body), Some(loop_n)) = (body_node, loop_node) {
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut inner_matches =
                    inner_cursor.matches(&self.call_query, tree.root_node(), source);

                let fn_name_idx = find_capture_index(&self.call_query, "fn_name");
                let call_idx = find_capture_index(&self.call_query, "call");

                while let Some(im) = inner_matches.next() {
                    let fn_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == fn_name_idx)
                        .map(|c| c.node);
                    let call_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(fn_n), Some(call_n)) = (fn_node, call_node) {
                        let fn_text = node_text(fn_n, source);
                        // Extract the method name from field_expression (e.g., "vec.push_back")
                        let method = fn_text.rsplit('.').next().unwrap_or("");
                        let method = method.rsplit("::").next().unwrap_or(method);

                        if GROWTH_METHODS.contains(&method) {
                            let start = call_n.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "unbounded_container_growth".to_string(),
                                message: format!(
                                    "`.{method}()` inside loop — unbounded container growth, consider reserving capacity or bounding"
                                ),
                                snippet: extract_snippet(source, loop_n, 5),
                            });
                        }
                    }
                }
            }
        }

        findings
    }

    /// Detect missing virtual destructors: class has virtual methods but no virtual destructor
    fn check_missing_virtual_destructor(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.class_query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.class_query, "class_name");
        let body_idx = find_capture_index(&self.class_query, "class_body");
        let class_idx = find_capture_index(&self.class_query, "class_def");

        while let Some(m) = matches.next() {
            let name_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == name_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);
            let class_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == class_idx)
                .map(|c| c.node);

            if let (Some(name_n), Some(body_n), Some(class_n)) = (name_node, body_node, class_node)
            {
                let class_name = node_text(name_n, source);
                let body_text = node_text(body_n, source);

                // Check if the class has any virtual methods
                let has_virtual_method = Self::has_virtual_method(body_n, source);

                if !has_virtual_method {
                    continue;
                }

                // Check if the class has a virtual destructor
                let has_virtual_destructor =
                    if body_text.contains("virtual ~") || body_text.contains("virtual~") {
                        true
                    } else {
                        // More careful check: look for "virtual" before "~ClassName"
                        let dtor_pattern = format!("~{class_name}");
                        if let Some(dtor_pos) = body_text.find(&dtor_pattern) {
                            let before = &body_text[..dtor_pos];
                            before.rfind("virtual").map_or(false, |vpos| {
                                // Make sure there's no semicolon between virtual and ~
                                !before[vpos..].contains(';')
                            })
                        } else {
                            false
                        }
                    };

                if !has_virtual_destructor {
                    let start = class_n.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "missing_virtual_destructor".to_string(),
                        message: format!(
                            "class `{class_name}` has virtual methods but no virtual destructor — deleting through base pointer causes undefined behavior"
                        ),
                        snippet: extract_snippet(source, class_n, 3),
                    });
                }
            }
        }

        findings
    }

    /// Walk the class body to find virtual method declarations
    fn has_virtual_method(body: tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            // field_declaration or function_definition inside the class
            if child.kind() == "field_declaration"
                || child.kind() == "function_definition"
                || child.kind() == "declaration"
            {
                let text = node_text(child, source);
                // Check for "virtual" keyword but not in a destructor
                if text.trim_start().starts_with("virtual") {
                    // Make sure this isn't the destructor itself
                    if !text.contains('~') {
                        return true;
                    }
                }
            }
            // Also check inside access_specifier sections
            if child.kind() == "access_specifier" {
                continue;
            }
        }
        false
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects memory leak indicators: raw new without delete, unbounded container growth in loops, missing virtual destructors"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Pattern 1: raw new without delete
        findings.extend(self.check_raw_new(tree, source, file_path));

        // Pattern 2: unbounded container growth in loops
        findings.extend(self.check_unbounded_growth(tree, source, file_path));

        // Pattern 3: missing virtual destructors
        findings.extend(self.check_missing_virtual_destructor(tree, source, file_path));

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
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.cpp")
    }

    #[test]
    fn detects_raw_new_without_delete() {
        let src = r#"
void f() {
    int* p = new int(42);
    use(p);
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "raw_new_without_delete")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("raw `new`"));
    }

    #[test]
    fn ignores_new_with_delete() {
        let src = r#"
void f() {
    int* p = new int(42);
    use(p);
    delete p;
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "raw_new_without_delete")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_new_in_smart_pointer() {
        let src = r#"
void f() {
    std::unique_ptr<int> p(new int(42));
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "raw_new_without_delete")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_push_back_in_loop() {
        let src = r#"
void f() {
    std::vector<int> v;
    while (true) {
        v.push_back(read_value());
    }
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "unbounded_container_growth")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("push_back"));
    }

    #[test]
    fn detects_emplace_back_in_for_range() {
        let src = r#"
void f(std::vector<Item>& items) {
    std::vector<Result> results;
    for (auto& item : items) {
        results.emplace_back(process(item));
    }
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "unbounded_container_growth")
            .collect();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn detects_insert_in_for_loop() {
        let src = r#"
void f() {
    std::set<int> s;
    for (int i = 0; i < n; i++) {
        s.insert(compute(i));
    }
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "unbounded_container_growth")
            .collect();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn ignores_growth_outside_loop() {
        let src = r#"
void f() {
    std::vector<int> v;
    v.push_back(1);
    v.push_back(2);
}
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "unbounded_container_growth")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_missing_virtual_destructor() {
        let src = r#"
class Base {
    virtual void foo();
    void bar();
};
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "missing_virtual_destructor")
            .collect();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("Base"));
    }

    #[test]
    fn ignores_class_with_virtual_destructor() {
        let src = r#"
class Base {
    virtual void foo();
    virtual ~Base() = default;
};
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "missing_virtual_destructor")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_class_without_virtual_methods() {
        let src = r#"
class Simple {
    void foo();
    int bar();
};
"#;
        let findings: Vec<_> = parse_and_check(src)
            .into_iter()
            .filter(|f| f.pattern == "missing_virtual_destructor")
            .collect();
        assert!(findings.is_empty());
    }

    #[test]
    fn metadata_correct() {
        let src = r#"
void f() {
    int* p = new int(42);
    use(p);
}
"#;
        let findings = parse_and_check(src);
        let leak = findings
            .iter()
            .find(|f| f.pattern == "raw_new_without_delete")
            .unwrap();
        assert_eq!(leak.severity, "warning");
        assert_eq!(leak.pipeline, "memory_leak_indicators");
    }
}
