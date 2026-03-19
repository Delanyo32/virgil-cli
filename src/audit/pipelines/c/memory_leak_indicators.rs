use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

const ALLOC_FUNCTIONS: &[&str] = &[
    "malloc", "calloc", "realloc", "strdup", "strndup", "asprintf",
];
const FORGOTTEN_FREE_ALLOCS: &[&str] = &["strdup", "strndup", "asprintf"];

fn c_lang() -> tree_sitter::Language {
    Language::C.tree_sitter_language()
}

pub struct MemoryLeakIndicatorsPipeline {
    loop_query: Arc<Query>,
    fn_def_query: Arc<Query>,
    call_query: Arc<Query>,
}

impl MemoryLeakIndicatorsPipeline {
    pub fn new() -> Result<Self> {
        let loop_query_str = r#"
[
  (for_statement body: (compound_statement) @loop_body) @loop_expr
  (while_statement body: (compound_statement) @loop_body) @loop_expr
  (do_statement body: (compound_statement) @loop_body) @loop_expr
]
"#;
        let loop_query = Query::new(&c_lang(), loop_query_str)
            .with_context(|| "failed to compile loop query for C memory_leak_indicators")?;

        let fn_def_query_str = r#"
(function_definition
  declarator: (_) @declarator
  body: (compound_statement) @fn_body) @fn_def
"#;
        let fn_def_query = Query::new(&c_lang(), fn_def_query_str).with_context(
            || "failed to compile function_definition query for C memory_leak_indicators",
        )?;

        let call_query_str = r#"
(call_expression
  function: (identifier) @fn_name) @call
"#;
        let call_query = Query::new(&c_lang(), call_query_str)
            .with_context(|| "failed to compile call query for C memory_leak_indicators")?;

        Ok(Self {
            loop_query: Arc::new(loop_query),
            fn_def_query: Arc::new(fn_def_query),
            call_query: Arc::new(call_query),
        })
    }

    /// Scan a node range for calls to specific functions, returning whether
    /// any call to the given names exists.
    fn has_call_in_range(
        &self,
        tree: &Tree,
        source: &[u8],
        range: std::ops::Range<usize>,
        target_names: &[&str],
    ) -> bool {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range);
        let mut matches = cursor.matches(&self.call_query, tree.root_node(), source);

        let name_idx = find_capture_index(&self.call_query, "fn_name");

        while let Some(m) = matches.next() {
            if let Some(cap) = m.captures.iter().find(|c| c.index as usize == name_idx) {
                let fn_name = node_text(cap.node, source);
                if target_names.contains(&fn_name) {
                    return true;
                }
            }
        }
        false
    }

    /// Find allocations inside a loop body without corresponding free.
    fn check_alloc_in_loops(
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
                let has_free = self.has_call_in_range(tree, source, body.byte_range(), &["free"]);

                if has_free {
                    continue;
                }

                // Look for alloc calls inside the loop body
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut inner_matches =
                    inner_cursor.matches(&self.call_query, tree.root_node(), source);

                let name_idx = find_capture_index(&self.call_query, "fn_name");
                let call_idx = find_capture_index(&self.call_query, "call");

                while let Some(im) = inner_matches.next() {
                    let name_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == name_idx)
                        .map(|c| c.node);
                    let call_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                        let fn_name = node_text(name_n, source);

                        if ALLOC_FUNCTIONS.contains(&fn_name) {
                            let start = call_n.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: "memory_leak_indicators".to_string(),
                                pattern: "alloc_in_loop".to_string(),
                                message: format!(
                                    "`{fn_name}()` called inside loop without `free()` — memory leak on each iteration"
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

    /// Find fopen without fclose in the same function body.
    fn check_unclosed_files(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let Some(body) = body_node {
                let has_fclose =
                    self.has_call_in_range(tree, source, body.byte_range(), &["fclose"]);

                if has_fclose {
                    continue;
                }

                // Look for fopen calls
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut inner_matches =
                    inner_cursor.matches(&self.call_query, tree.root_node(), source);

                let name_idx = find_capture_index(&self.call_query, "fn_name");
                let call_idx = find_capture_index(&self.call_query, "call");

                while let Some(im) = inner_matches.next() {
                    let name_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == name_idx)
                        .map(|c| c.node);
                    let call_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                        let fn_name = node_text(name_n, source);

                        if fn_name == "fopen" {
                            let start = call_n.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: "memory_leak_indicators".to_string(),
                                pattern: "unclosed_file".to_string(),
                                message: "`fopen()` without corresponding `fclose()` in the same function — potential file handle leak".to_string(),
                                snippet: extract_snippet(source, call_n, 1),
                            });
                        }
                    }
                }
            }
        }

        findings
    }

    /// Find strdup/asprintf allocations without free in the same function.
    fn check_alloc_without_free(
        &self,
        tree: &Tree,
        source: &[u8],
        file_path: &str,
    ) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.fn_def_query, tree.root_node(), source);

        let body_idx = find_capture_index(&self.fn_def_query, "fn_body");

        while let Some(m) = matches.next() {
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == body_idx)
                .map(|c| c.node);

            if let Some(body) = body_node {
                let has_free = self.has_call_in_range(tree, source, body.byte_range(), &["free"]);

                if has_free {
                    continue;
                }

                // Look for strdup/asprintf calls
                let mut inner_cursor = QueryCursor::new();
                inner_cursor.set_byte_range(body.byte_range());
                let mut inner_matches =
                    inner_cursor.matches(&self.call_query, tree.root_node(), source);

                let name_idx = find_capture_index(&self.call_query, "fn_name");
                let call_idx = find_capture_index(&self.call_query, "call");

                while let Some(im) = inner_matches.next() {
                    let name_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == name_idx)
                        .map(|c| c.node);
                    let call_node = im
                        .captures
                        .iter()
                        .find(|c| c.index as usize == call_idx)
                        .map(|c| c.node);

                    if let (Some(name_n), Some(call_n)) = (name_node, call_node) {
                        let fn_name = node_text(name_n, source);

                        if FORGOTTEN_FREE_ALLOCS.contains(&fn_name) {
                            let start = call_n.start_position();
                            findings.push(AuditFinding {
                                file_path: file_path.to_string(),
                                line: start.row as u32 + 1,
                                column: start.column as u32 + 1,
                                severity: "warning".to_string(),
                                pipeline: "memory_leak_indicators".to_string(),
                                pattern: "alloc_without_free".to_string(),
                                message: format!(
                                    "`{fn_name}()` allocates memory but no `free()` found in the same function — potential memory leak"
                                ),
                                snippet: extract_snippet(source, call_n, 1),
                            });
                        }
                    }
                }
            }
        }

        findings
    }
}

impl Pipeline for MemoryLeakIndicatorsPipeline {
    fn name(&self) -> &str {
        "memory_leak_indicators"
    }

    fn description(&self) -> &str {
        "Detects memory leak indicators: allocations in loops without free, unclosed files, and forgotten frees for strdup/asprintf"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        findings.extend(self.check_alloc_in_loops(tree, source, file_path));
        findings.extend(self.check_unclosed_files(tree, source, file_path));
        findings.extend(self.check_alloc_without_free(tree, source, file_path));
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
            .set_language(&Language::C.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = MemoryLeakIndicatorsPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.c")
    }

    #[test]
    fn detects_malloc_in_loop_without_free() {
        let src = r#"
void process(int n) {
    for (int i = 0; i < n; i++) {
        char *buf = malloc(256);
        do_something(buf);
    }
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "alloc_in_loop");
        assert!(findings[0].message.contains("malloc"));
    }

    #[test]
    fn skips_alloc_in_loop_with_free() {
        let src = r#"
void process(int n) {
    for (int i = 0; i < n; i++) {
        char *buf = malloc(256);
        do_something(buf);
        free(buf);
    }
}
"#;
        let findings = parse_and_check(src);
        let alloc_in_loop: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "alloc_in_loop")
            .collect();
        assert!(alloc_in_loop.is_empty());
    }

    #[test]
    fn detects_realloc_in_while_loop() {
        let src = r#"
void grow(int n) {
    int i = 0;
    while (i < n) {
        char *p = realloc(old, i * 100);
        i++;
    }
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "alloc_in_loop" && f.message.contains("realloc"))
        );
    }

    #[test]
    fn detects_fopen_without_fclose() {
        let src = r#"
void read_config(const char *path) {
    FILE *fp = fopen(path, "r");
    fread(buf, 1, sizeof(buf), fp);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "unclosed_file");
        assert!(findings[0].message.contains("fopen"));
    }

    #[test]
    fn skips_fopen_with_fclose() {
        let src = r#"
void read_config(const char *path) {
    FILE *fp = fopen(path, "r");
    fread(buf, 1, sizeof(buf), fp);
    fclose(fp);
}
"#;
        let findings = parse_and_check(src);
        let unclosed: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "unclosed_file")
            .collect();
        assert!(unclosed.is_empty());
    }

    #[test]
    fn detects_strdup_without_free() {
        let src = r#"
void copy_name(const char *name) {
    char *copy = strdup(name);
    printf("name: %s\n", copy);
}
"#;
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "alloc_without_free");
        assert!(findings[0].message.contains("strdup"));
    }

    #[test]
    fn skips_strdup_with_free() {
        let src = r#"
void copy_name(const char *name) {
    char *copy = strdup(name);
    printf("name: %s\n", copy);
    free(copy);
}
"#;
        let findings = parse_and_check(src);
        let alloc_no_free: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "alloc_without_free")
            .collect();
        assert!(alloc_no_free.is_empty());
    }

    #[test]
    fn detects_asprintf_without_free() {
        let src = r#"
void format_msg(int id) {
    char *msg;
    asprintf(&msg, "id=%d", id);
    send_msg(msg);
}
"#;
        let findings = parse_and_check(src);
        assert!(
            findings
                .iter()
                .any(|f| f.pattern == "alloc_without_free" && f.message.contains("asprintf"))
        );
    }
}
