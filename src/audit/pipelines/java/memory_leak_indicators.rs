use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

fn java_lang() -> tree_sitter::Language {
    Language::Java.tree_sitter_language()
}

const RESOURCE_TYPES: &[&str] = &[
    "FileInputStream",
    "FileOutputStream",
    "BufferedReader",
    "BufferedWriter",
    "FileReader",
    "FileWriter",
    "InputStreamReader",
    "OutputStreamWriter",
    "DataInputStream",
    "DataOutputStream",
    "ObjectInputStream",
    "ObjectOutputStream",
    "Connection",
    "Statement",
    "PreparedStatement",
    "ResultSet",
    "Socket",
    "ServerSocket",
    "Scanner",
    "PrintWriter",
    "RandomAccessFile",
    "ZipInputStream",
    "ZipOutputStream",
    "GZIPInputStream",
    "GZIPOutputStream",
];

const COLLECTION_ADD_METHODS: &[&str] = &["add", "put", "offer", "push", "addAll", "putAll"];

pub struct MemoryLeakIndicatorsPipeline {
    local_var_query: Arc<Query>,
    method_invocation_query: Arc<Query>,
    static_field_query: Arc<Query>,
    resource_types: HashSet<&'static str>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        let local_var_str = r#"
(local_variable_declaration
  type: (_) @var_type
  declarator: (variable_declarator
    name: (identifier) @var_name
    value: (object_creation_expression
      type: (_) @creation_type
      arguments: (argument_list) @args)? @creation)) @var_decl
"#;
        let local_var_query = Query::new(&java_lang(), local_var_str).with_context(
            || "failed to compile local_variable_declaration query for memory_leak_indicators",
        )?;

        let method_invocation_str = r#"
(method_invocation
  object: (_)? @object
  name: (identifier) @method_name
  arguments: (argument_list) @args) @invocation
"#;
        let method_invocation_query = Query::new(&java_lang(), method_invocation_str)
            .with_context(
                || "failed to compile method_invocation query for memory_leak_indicators",
            )?;

        let static_field_str = r#"
(field_declaration
  declarator: (variable_declarator
    name: (identifier) @field_name
    value: (_)? @field_value)) @field_decl
"#;
        let static_field_query = Query::new(&java_lang(), static_field_str)
            .with_context(|| "failed to compile static field query for memory_leak_indicators")?;

        Ok(Self {
            local_var_query: Arc::new(local_var_query),
            method_invocation_query: Arc::new(method_invocation_query),
            static_field_query: Arc::new(static_field_query),
            resource_types: RESOURCE_TYPES.iter().copied().collect(),
        })
    }
}

fn is_in_try_with_resources(node: tree_sitter::Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "try_with_resources_statement" {
            return true;
        }
        if p.kind() == "method_declaration"
            || p.kind() == "constructor_declaration"
            || p.kind() == "class_declaration"
        {
            return false;
        }
        parent = p.parent();
    }
    false
}

fn is_inside_loop(node: tree_sitter::Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" => {
                return true;
            }
            "method_declaration" | "constructor_declaration" | "class_declaration" => return false,
            _ => parent = p.parent(),
        }
    }
    false
}

fn has_modifier_text(node: tree_sitter::Node, source: &[u8], modifier_text: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mod_cursor = child.walk();
            for modifier in child.children(&mut mod_cursor) {
                if modifier.utf8_text(source).unwrap_or("") == modifier_text {
                    return true;
                }
            }
        }
    }
    false
}

/// Collect names of static collection fields
fn collect_static_collection_fields<'a>(
    tree: &'a Tree,
    source: &[u8],
    static_field_query: &Query,
) -> HashSet<String> {
    let collection_types: HashSet<&str> = [
        "List",
        "ArrayList",
        "LinkedList",
        "Set",
        "HashSet",
        "TreeSet",
        "Map",
        "HashMap",
        "TreeMap",
        "LinkedHashMap",
        "ConcurrentHashMap",
        "Queue",
        "Deque",
        "ArrayDeque",
        "PriorityQueue",
        "Vector",
    ]
    .iter()
    .copied()
    .collect();

    let mut static_fields = HashSet::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(static_field_query, tree.root_node(), source);

    let field_decl_idx = find_capture_index(static_field_query, "field_decl");
    let field_name_idx = find_capture_index(static_field_query, "field_name");

    while let Some(m) = matches.next() {
        let decl_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == field_decl_idx)
            .map(|c| c.node);
        let name_node = m
            .captures
            .iter()
            .find(|c| c.index as usize == field_name_idx)
            .map(|c| c.node);

        if let (Some(decl_node), Some(name_node)) = (decl_node, name_node) {
            if !has_modifier_text(decl_node, source, "static") {
                continue;
            }

            // Check type
            let decl_text = node_text(decl_node, source);
            let is_collection = collection_types.iter().any(|ct| decl_text.contains(ct));

            if is_collection {
                let field_name = node_text(name_node, source);
                static_fields.insert(field_name.to_string());
            }
        }
    }

    static_fields
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects memory leak indicators: unclosed resources, unbounded collection growth in loops, static collections"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // 1. Find resource creation without try-with-resources
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.local_var_query, tree.root_node(), source);

            let var_type_idx = find_capture_index(&self.local_var_query, "var_type");
            let var_name_idx = find_capture_index(&self.local_var_query, "var_name");
            let creation_idx = find_capture_index(&self.local_var_query, "creation");
            let creation_type_idx = find_capture_index(&self.local_var_query, "creation_type");
            let var_decl_idx = find_capture_index(&self.local_var_query, "var_decl");

            while let Some(m) = matches.next() {
                let _creation = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == creation_idx)
                    .map(|c| c.node);

                // Must have object creation
                if _creation.is_none() {
                    continue;
                }

                let type_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == var_type_idx)
                    .map(|c| c.node);
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == var_name_idx)
                    .map(|c| c.node);
                let creation_type_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == creation_type_idx)
                    .map(|c| c.node);
                let decl_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == var_decl_idx)
                    .map(|c| c.node);

                if let (Some(name_node), Some(decl_node)) = (name_node, decl_node) {
                    // Get the type being created (from the object_creation_expression)
                    let creation_type = creation_type_node
                        .map(|n| {
                            if n.kind() == "generic_type" {
                                n.named_child(0).map(|c| node_text(c, source)).unwrap_or("")
                            } else {
                                node_text(n, source)
                            }
                        })
                        .unwrap_or("");

                    // Also check the declared type
                    let declared_type = type_node
                        .map(|n| {
                            if n.kind() == "generic_type" {
                                n.named_child(0).map(|c| node_text(c, source)).unwrap_or("")
                            } else {
                                node_text(n, source)
                            }
                        })
                        .unwrap_or("");

                    let is_resource = self.resource_types.contains(creation_type)
                        || self.resource_types.contains(declared_type);

                    if !is_resource {
                        continue;
                    }

                    if is_in_try_with_resources(decl_node) {
                        continue;
                    }

                    let var_name = node_text(name_node, source);
                    let type_name = if !creation_type.is_empty() {
                        creation_type
                    } else {
                        declared_type
                    };
                    let start = decl_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unclosed_resource".to_string(),
                        message: format!(
                            "`{type_name} {var_name}` created outside try-with-resources — potential memory/resource leak"
                        ),
                        snippet: extract_snippet(source, decl_node, 3),
                    });
                }
            }
        }

        // 2. Find .add()/.put() inside loops (unbounded collection growth)
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.method_invocation_query, tree.root_node(), source);

            let method_idx = find_capture_index(&self.method_invocation_query, "method_name");
            let invocation_idx = find_capture_index(&self.method_invocation_query, "invocation");

            while let Some(m) = matches.next() {
                let method_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_idx)
                    .map(|c| c.node);
                let inv_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == invocation_idx)
                    .map(|c| c.node);

                if let (Some(method_node), Some(inv_node)) = (method_node, inv_node) {
                    let method_name = node_text(method_node, source);
                    if !COLLECTION_ADD_METHODS.contains(&method_name) {
                        continue;
                    }
                    if !is_inside_loop(inv_node) {
                        continue;
                    }

                    let start = inv_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "unbounded_collection_growth".to_string(),
                        message: format!(
                            "`.{method_name}()` inside loop — potential unbounded collection growth"
                        ),
                        snippet: extract_snippet(source, inv_node, 2),
                    });
                }
            }
        }

        // 3. Find static collections being populated (never cleared)
        {
            let static_fields =
                collect_static_collection_fields(tree, source, &self.static_field_query);

            if !static_fields.is_empty() {
                let mut cursor = QueryCursor::new();
                let mut matches =
                    cursor.matches(&self.method_invocation_query, tree.root_node(), source);

                let object_idx = find_capture_index(&self.method_invocation_query, "object");
                let method_idx = find_capture_index(&self.method_invocation_query, "method_name");
                let invocation_idx =
                    find_capture_index(&self.method_invocation_query, "invocation");

                while let Some(m) = matches.next() {
                    let object_node = m
                        .captures
                        .iter()
                        .find(|c| c.index as usize == object_idx)
                        .map(|c| c.node);
                    let method_node = m
                        .captures
                        .iter()
                        .find(|c| c.index as usize == method_idx)
                        .map(|c| c.node);
                    let inv_node = m
                        .captures
                        .iter()
                        .find(|c| c.index as usize == invocation_idx)
                        .map(|c| c.node);

                    if let (Some(obj), Some(method_node), Some(inv_node)) =
                        (object_node, method_node, inv_node)
                    {
                        let obj_text = node_text(obj, source);
                        let method_name = node_text(method_node, source);

                        if static_fields.contains(obj_text)
                            && COLLECTION_ADD_METHODS.contains(&method_name)
                        {
                            let start = inv_node.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: self.name().to_string(),
                                pattern: "static_collection_accumulation".to_string(),
                                message: format!(
                                    "static collection `{obj_text}` grows via `.{method_name}()` — may leak memory if never cleared"
                                ),
                                snippet: extract_snippet(source, inv_node, 2),
                            });
                        }
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
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Java.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "Test.java")
    }

    #[test]
    fn detects_unclosed_file_input_stream() {
        let src = r#"class Foo {
    void m() {
        FileInputStream fis = new FileInputStream("file.txt");
        fis.read();
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().any(|f| f.pattern == "unclosed_resource"));
        assert!(findings[0].message.contains("FileInputStream"));
    }

    #[test]
    fn ignores_resource_in_try_with_resources() {
        let src = r#"class Foo {
    void m() {
        try (FileInputStream fis = new FileInputStream("file.txt")) {
            fis.read();
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert!(findings.iter().all(|f| f.pattern != "unclosed_resource"));
    }

    #[test]
    fn detects_collection_add_in_loop() {
        let src = r#"class Foo {
    void m() {
        List<String> items = new ArrayList<>();
        while (true) {
            items.add("something");
        }
    }
}"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "unbounded_collection_growth")
        );
    }

    #[test]
    fn ignores_collection_add_outside_loop() {
        let src = r#"class Foo {
    void m() {
        List<String> items = new ArrayList<>();
        items.add("one");
        items.add("two");
    }
}"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .all(|f| f.pattern != "unbounded_collection_growth")
        );
    }

    #[test]
    fn detects_static_collection_accumulation() {
        let src = r#"class Cache {
    static List<String> cache = new ArrayList<>();

    void store(String item) {
        cache.add(item);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "static_collection_accumulation")
        );
    }

    #[test]
    fn ignores_non_static_field_add() {
        let src = r#"class Service {
    List<String> items = new ArrayList<>();

    void store(String item) {
        items.add(item);
    }
}"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .all(|f| f.pattern != "static_collection_accumulation")
        );
    }
}
