use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::{GraphPipeline, GraphPipelineContext};
use crate::audit::pipelines::helpers::is_nolint_suppressed;

use super::primitives::{compile_preproc_include_query, find_capture_index, node_text};

const INCLUDE_THRESHOLD_SOURCE: usize = 20;
const INCLUDE_THRESHOLD_HEADER: usize = 15;

pub struct ExcessiveIncludesPipeline {
    include_query: Arc<Query>,
}

impl ExcessiveIncludesPipeline {
    pub fn new() -> Result<Self> {
        Ok(Self {
            include_query: compile_preproc_include_query()?,
        })
    }

    fn is_header(file_path: &str) -> bool {
        file_path.ends_with(".h")
            || file_path.ends_with(".hpp")
            || file_path.ends_with(".hxx")
            || file_path.ends_with(".hh")
    }
}

impl GraphPipeline for ExcessiveIncludesPipeline {
    fn name(&self) -> &str {
        "excessive_includes"
    }

    fn description(&self) -> &str {
        "Detects files with too many #include directives — consider forward declarations or splitting"
    }

    fn check(&self, ctx: &GraphPipelineContext) -> Vec<AuditFinding> {
        let (tree, source, file_path) = (ctx.tree, ctx.source, ctx.file_path);
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.include_query, tree.root_node(), source);

        let include_idx = find_capture_index(&self.include_query, "include_dir");
        let mut total_count = 0;
        let mut system_count = 0;
        let mut project_count = 0;
        let mut first_include_node = None;

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.iter().find(|c| c.index as usize == include_idx) {
                total_count += 1;
                if first_include_node.is_none() {
                    first_include_node = Some(cap.node);
                }

                // Distinguish system (<...>) vs project ("...") includes
                let text = node_text(cap.node, source);
                if text.contains('<') {
                    system_count += 1;
                } else {
                    project_count += 1;
                }
            }
        }

        let base_threshold = if Self::is_header(file_path) {
            INCLUDE_THRESHOLD_HEADER
        } else {
            INCLUDE_THRESHOLD_SOURCE
        };

        // Scale threshold with file size (1 extra include per 100 lines, minimum is base)
        let line_count = source.iter().filter(|&&b| b == b'\n').count().max(1);
        let scaled_threshold = base_threshold + line_count / 100;

        if total_count <= scaled_threshold {
            return vec![];
        }

        // Check NOLINT on first include
        if let Some(node) = first_include_node
            && is_nolint_suppressed(source, node, self.name()) {
                return vec![];
            }

        // Graduate severity
        let severity = if total_count >= scaled_threshold * 2 {
            "warning"
        } else {
            "info"
        };

        vec![AuditFinding {
            file_path: file_path.to_string(),
            line: 1,
            column: 1,
            severity: severity.to_string(),
            pipeline: self.name().to_string(),
            pattern: "excessive_includes".to_string(),
            message: format!(
                "{total_count} #include directives ({system_count} system, {project_count} project; threshold: {scaled_threshold}) — consider forward declarations or splitting the file"
            ),
            snippet: format!("{total_count} includes"),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::pipeline::GraphPipelineContext;
    use crate::language::Language;
    use std::collections::HashMap;

    fn parse_and_check_with_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExcessiveIncludesPipeline::new().unwrap();
        let graph = crate::graph::CodeGraph::new();
        let id_counts = HashMap::new();
        let ctx = GraphPipelineContext {
            tree: &tree,
            source: source.as_bytes(),
            file_path: path,
            id_counts: &id_counts,
            graph: &graph,
        };
        pipeline.check(&ctx)
    }

    fn make_includes(n: usize) -> String {
        (0..n)
            .map(|i| format!("#include <header{i}.h>"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn no_finding_under_threshold() {
        let src = make_includes(15);
        let findings = parse_and_check_with_path(&src, "test.cpp");
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_over_threshold_source() {
        let src = make_includes(21);
        let findings = parse_and_check_with_path(&src, "test.cpp");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "excessive_includes");
        assert!(findings[0].message.contains("21"));
    }

    #[test]
    fn header_lower_threshold() {
        let src = make_includes(16);
        let findings = parse_and_check_with_path(&src, "test.hpp");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "excessive_includes");
    }

    #[test]
    fn header_at_threshold_ok() {
        let src = make_includes(15);
        let findings = parse_and_check_with_path(&src, "test.hpp");
        assert!(findings.is_empty());
    }

    #[test]
    fn empty_file_no_findings() {
        let findings = parse_and_check_with_path("", "test.cpp");
        assert!(findings.is_empty());
    }

    #[test]
    fn message_includes_system_project_breakdown() {
        let mut src = String::new();
        for i in 0..15 {
            src.push_str(&format!("#include <system{i}.h>\n"));
        }
        for i in 0..7 {
            src.push_str(&format!("#include \"project{i}.h\"\n"));
        }
        let findings = parse_and_check_with_path(&src, "test.cpp");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("15 system"));
        assert!(findings[0].message.contains("7 project"));
    }

    #[test]
    fn severity_graduates_to_warning() {
        let src = make_includes(42);
        let findings = parse_and_check_with_path(&src, "test.cpp");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warning");
    }
}
