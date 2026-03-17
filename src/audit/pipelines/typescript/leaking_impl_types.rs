use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::typescript_primitives::{compile_function_query, extract_snippet, node_text};

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

impl Pipeline for LeakingImplTypesPipeline {
    fn name(&self) -> &str {
        "leaking_impl_types"
    }

    fn description(&self) -> &str {
        "Detects exported functions that expose ORM/database implementation types in their return type"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let func_node = match m.captures.first() {
                Some(c) => c.node,
                None => continue,
            };

            // Check if exported
            if !is_exported(func_node) {
                continue;
            }

            // Check return type
            let return_type_text = match func_node.child_by_field_name("return_type") {
                Some(rt) => node_text(rt, source).to_string(),
                None => continue,
            };

            for pattern in ORM_PATTERNS {
                if return_type_text.contains(pattern) {
                    let start = func_node.start_position();
                    findings.push(AuditFinding {
                        file_path: file_path.to_string(),
                        line: start.row as u32 + 1,
                        column: start.column as u32 + 1,
                        severity: "warning".to_string(),
                        pipeline: self.name().to_string(),
                        pattern: "leaking_orm_type".to_string(),
                        message: format!(
                            "Exported function exposes `{pattern}` in return type — consumers become coupled to the ORM/database implementation"
                        ),
                        snippet: extract_snippet(source, func_node, 1),
                    });
                    break;
                }
            }
        }

        findings
    }
}

fn is_exported(node: tree_sitter::Node) -> bool {
    if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            return true;
        }
    }
    false
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

    #[test]
    fn detects_leaking_prisma() {
        let findings =
            parse_and_check("export function getDB(): PrismaClient { return prisma; }");
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
        let findings =
            parse_and_check("function getDB(): PrismaClient { return prisma; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_safe_return_type() {
        let findings =
            parse_and_check("export function getUser(): User { return user; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_no_return_type() {
        let findings =
            parse_and_check("export function doStuff() { return 1; }");
        assert!(findings.is_empty());
    }

    #[test]
    fn tsx_compiles() {
        LeakingImplTypesPipeline::new(Language::Tsx).unwrap();
    }
}
