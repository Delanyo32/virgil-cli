use std::sync::Arc;

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;

use super::primitives::{compile_preproc_include_query, find_capture_index};

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

impl Pipeline for ExcessiveIncludesPipeline {
    fn name(&self) -> &str {
        "excessive_includes"
    }

    fn description(&self) -> &str {
        "Detects files with too many #include directives — consider forward declarations or splitting"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.include_query, tree.root_node(), source);

        let include_idx = find_capture_index(&self.include_query, "include_dir");
        let mut count = 0;

        while let Some(m) = matches.next() {
            if m.captures
                .iter()
                .any(|c| c.index as usize == include_idx)
            {
                count += 1;
            }
        }

        let threshold = if Self::is_header(file_path) {
            INCLUDE_THRESHOLD_HEADER
        } else {
            INCLUDE_THRESHOLD_SOURCE
        };

        if count > threshold {
            vec![AuditFinding {
                file_path: file_path.to_string(),
                line: 1,
                column: 1,
                severity: "info".to_string(),
                pipeline: self.name().to_string(),
                pattern: "excessive_includes".to_string(),
                message: format!(
                    "{count} #include directives (threshold: {threshold}) — consider forward declarations or splitting the file"
                ),
                snippet: format!("{count} includes"),
            }]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    fn parse_and_check_with_path(source: &str, path: &str) -> Vec<AuditFinding> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Cpp.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = ExcessiveIncludesPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), path)
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
}
