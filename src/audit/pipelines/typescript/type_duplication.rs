use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{compile_interface_declaration_query, find_capture_index, node_text};

pub struct TypeDuplicationPipeline {
    query: Arc<Query>,
    name_idx: usize,
    body_idx: usize,
}

impl TypeDuplicationPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let query = compile_interface_declaration_query(language)?;
        let name_idx = find_capture_index(&query, "name");
        let body_idx = find_capture_index(&query, "body");
        Ok(Self {
            query,
            name_idx,
            body_idx,
        })
    }
}

struct InterfaceInfo {
    name: String,
    fields: HashSet<String>,
    line: u32,
    column: u32,
    node_start: usize,
    node_end: usize,
}

impl Pipeline for TypeDuplicationPipeline {
    fn name(&self) -> &str {
        "type_duplication"
    }

    fn description(&self) -> &str {
        "Detects interfaces with highly overlapping property sets suggesting type duplication"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);
        let mut interfaces: Vec<InterfaceInfo> = Vec::new();

        while let Some(m) = matches.next() {
            let iface_name = m
                .captures
                .iter()
                .find(|c| c.index as usize == self.name_idx)
                .map(|c| node_text(c.node, source).to_string())
                .unwrap_or_default();

            let body_node = match m
                .captures
                .iter()
                .find(|c| c.index as usize == self.body_idx)
            {
                Some(c) => c.node,
                None => continue,
            };

            let decl_node = m.captures.first().map(|c| c.node).unwrap_or(body_node);
            let start = decl_node.start_position();

            let mut fields = HashSet::new();
            let mut body_cursor = body_node.walk();
            for child in body_node.named_children(&mut body_cursor) {
                if child.kind() == "property_signature" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        fields.insert(node_text(name_node, source).to_string());
                    }
                }
            }

            interfaces.push(InterfaceInfo {
                name: iface_name,
                fields,
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                node_start: decl_node.start_byte(),
                node_end: decl_node.end_byte(),
            });
        }

        let mut findings = Vec::new();
        let mut reported: HashSet<(usize, usize)> = HashSet::new();

        for i in 0..interfaces.len() {
            for j in (i + 1)..interfaces.len() {
                if reported.contains(&(i, j)) {
                    continue;
                }

                let a = &interfaces[i];
                let b = &interfaces[j];

                let intersection = a.fields.intersection(&b.fields).count();
                let union = a.fields.union(&b.fields).count();

                if union == 0 || intersection < 3 {
                    continue;
                }

                let jaccard = intersection as f64 / union as f64;
                if jaccard > 0.7 {
                    reported.insert((i, j));

                    let shared: Vec<&String> = {
                        let mut s: Vec<&String> = a.fields.intersection(&b.fields).collect();
                        s.sort();
                        s
                    };
                    let shared_preview: String = if shared.len() <= 5 {
                        shared
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    } else {
                        let mut preview: String = shared[..5]
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        preview.push_str(&format!(" (+{} more)", shared.len() - 5));
                        preview
                    };

                    let snippet_bytes = &source[a.node_start..a.node_end.min(source.len())];
                    let snippet_text = std::str::from_utf8(snippet_bytes).unwrap_or("");
                    let snippet_lines: Vec<&str> = snippet_text.lines().collect();
                    let snippet = if snippet_lines.len() <= 3 {
                        snippet_text.to_string()
                    } else {
                        format!("{}\n...", snippet_lines[..3].join("\n"))
                    };

                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: a.line,
                        column: a.column,
                        severity: "info".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "duplicate_shape".to_string(),
                        message: format!(
                            "`{}` and `{}` share {}/{} fields ({:.0}% overlap: {}) — consider extracting a common base type",
                            a.name,
                            b.name,
                            intersection,
                            union,
                            jaccard * 100.0,
                            shared_preview,
                        ),
                        snippet,
                    });
                }
            }
        }

        // Check for suffix patterns (UserRow/UserDTO/UserResponse)
        let mut base_groups: HashMap<String, Vec<usize>> = HashMap::new();
        let suffixes = [
            "Row", "DTO", "Response", "Input", "Output", "Model", "Entity",
        ];
        for (idx, iface) in interfaces.iter().enumerate() {
            for suffix in &suffixes {
                if iface.name.ends_with(suffix) {
                    let base = iface.name[..iface.name.len() - suffix.len()].to_string();
                    if !base.is_empty() {
                        base_groups.entry(base).or_default().push(idx);
                    }
                }
            }
        }

        for (base, indices) in &base_groups {
            if indices.len() >= 2 {
                let names: Vec<&str> = indices
                    .iter()
                    .map(|&i| interfaces[i].name.as_str())
                    .collect();
                // Only report if not already reported via Jaccard
                let pair = (indices[0], indices[1]);
                if reported.contains(&pair) {
                    continue;
                }

                let first = &interfaces[indices[0]];
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: first.line,
                    column: first.column,
                    severity: "info".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "duplicate_shape".to_string(),
                    message: format!(
                        "Multiple `{base}*` interfaces ({}) suggest type duplication — consider a single generic or mapped type",
                        names.join(", "),
                    ),
                    snippet: String::new(),
                });
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
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = TypeDuplicationPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_high_overlap() {
        // 4 shared out of 5 total = Jaccard 0.8 > 0.7
        let src = r#"
interface UserA {
    id: string;
    name: string;
    email: string;
    age: number;
}
interface UserB {
    id: string;
    name: string;
    email: string;
    age: number;
    phone: string;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "duplicate_shape");
    }

    #[test]
    fn skips_low_overlap() {
        let src = r#"
interface Dog {
    breed: string;
    age: number;
    weight: number;
}
interface Car {
    make: string;
    model: string;
    year: number;
}
"#;
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_suffix_pattern() {
        let src = r#"
interface UserRow {
    id: number;
}
interface UserDTO {
    name: string;
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("User*"));
    }

    #[test]
    fn skips_single_interface() {
        let src = "interface Foo { a: string; b: number; }";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        TypeDuplicationPipeline::new(Language::Tsx).unwrap();
    }
}
