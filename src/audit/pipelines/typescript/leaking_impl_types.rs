use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{compile_function_query, extract_snippet, is_test_file, is_ts_suppressed, node_text};

const ORM_PATTERNS: &[&str] = &[
    "PrismaClient",
    "Repository",
    "EntityManager",
    "QueryRunner",
    "RowDataPacket",
    "QueryResult",
    "Connection",
    "Pool",
    "Knex",
    "Sequelize",
    "Model",
    "DataSource",
    "TypeORMError",
    "SelectQueryBuilder",
    "InsertResult",
    "UpdateResult",
    "DeleteResult",
    "ObjectLiteral",
];

pub struct LeakingImplTypesPipeline {
    query: Arc<Query>,
}

impl LeakingImplTypesPipeline {
    pub fn new(language: Language) -> Result<Self> {
        Ok(Self {
            query: compile_function_query(language)?,
        })
    }
}

fn matches_orm_pattern(return_type_text: &str, patterns: &[&str]) -> bool {
    let words: Vec<&str> = return_type_text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect();
    patterns.iter().any(|p| words.contains(p))
}

fn is_exported_node(node: tree_sitter::Node) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "export_statement" => return true,
            "program" | "statement_block" | "class_body" => return false,
            _ => {
                current = parent;
            }
        }
    }
    false
}

impl Pipeline for LeakingImplTypesPipeline {
    fn name(&self) -> &str {
        "leaking_impl_types"
    }

    fn description(&self) -> &str {
        "Detects exported functions that expose ORM/database implementation types in their return type"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        if is_test_file(file_path) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let func_node = match m.captures.first() {
                Some(c) => c.node,
                None => continue,
            };

            // Check if exported using parent-chain walk
            if !is_exported_node(func_node) {
                continue;
            }

            // Check return type
            let return_type_text = match func_node.child_by_field_name("return_type") {
                Some(rt) => node_text(rt, source).to_string(),
                None => continue,
            };

            if matches_orm_pattern(&return_type_text, ORM_PATTERNS) {
                if is_ts_suppressed(source, func_node) {
                    continue;
                }
                // Find the matched pattern for the message
                let matched = ORM_PATTERNS
                    .iter()
                    .find(|&&p| {
                        return_type_text
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .filter(|s| !s.is_empty())
                            .any(|w| w == p)
                    })
                    .copied()
                    .unwrap_or("ORM type");
                let start = func_node.start_position();
                findings.push(AuditFinding {
                    file_path: file_path.to_string(),
                    line: start.row as u32 + 1,
                    column: start.column as u32 + 1,
                    severity: "warning".to_string(),
                    pipeline: self.name().to_string(),
                    pattern: "leaking_orm_type".to_string(),
                    message: format!(
                        "Exported function exposes `{matched}` in return type — consumers become coupled to the ORM/database implementation"
                    ),
                    snippet: extract_snippet(source, func_node, 1),
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
        let pipeline = LeakingImplTypesPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    fn parse_and_check_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::TypeScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = LeakingImplTypesPipeline::new(Language::TypeScript).unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
    }

    #[test]
    fn detects_leaking_prisma() {
        let findings = parse_and_check("export function getDB(): PrismaClient { return prisma; }");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "leaking_orm_type");
    }

    #[test]
    fn detects_leaking_repository() {
        let findings =
            parse_and_check("export function getRepo(): Repository<User> { return repo; }");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_non_exported() {
        let findings = parse_and_check("function getDB(): PrismaClient { return prisma; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_safe_return_type() {
        let findings = parse_and_check("export function getUser(): User { return user; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_no_return_type() {
        let findings = parse_and_check("export function doStuff() { return 1; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_substring_false_positive_connection_config() {
        let findings = parse_and_check("export function getConfig(): ConnectionConfig {}");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_model_substring_false_positive() {
        let findings = parse_and_check("export function getVM(): ViewModel {}");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_arrow_function_export() {
        let findings = parse_and_check("export const getDB = (): PrismaClient => ({});");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn skips_test_file() {
        let findings = parse_and_check_path("export function getDB(): PrismaClient {}", "src/db.test.ts");
        assert!(findings.is_empty());
    }

    #[test]
    fn suppression_skips_leaking_type() {
        let findings = parse_and_check("// virgil-ignore\nexport function getDB(): PrismaClient {}");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        LeakingImplTypesPipeline::new(Language::Tsx).unwrap();
    }
}
