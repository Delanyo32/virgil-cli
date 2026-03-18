use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Node.js *Sync() API methods that block the event loop.
const SYNC_METHOD_NAMES: &[&str] = &[
    "readFileSync",
    "writeFileSync",
    "appendFileSync",
    "copyFileSync",
    "mkdirSync",
    "readdirSync",
    "statSync",
    "lstatSync",
    "unlinkSync",
    "renameSync",
    "existsSync",
    "accessSync",
    "chmodSync",
    "chownSync",
    "truncateSync",
    "realpathSync",
];

/// Bare function calls that are blocking.
const SYNC_BARE_CALLS: &[&str] = &[
    "execSync",
    "spawnSync",
    "execFileSync",
];

fn js_lang() -> tree_sitter::Language {
    Language::JavaScript.tree_sitter_language()
}

pub struct SyncBlockingInAsyncPipeline {
    async_fn_query: Arc<Query>,
    method_call_query: Arc<Query>,
    direct_call_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new() -> Result<Self> {
        // Match async function declarations, async arrow functions, and async methods
        let async_fn_str = r#"
[
  (function_declaration
    body: (statement_block) @fn_body) @fn_def

  (lexical_declaration
    (variable_declarator
      value: (arrow_function
        body: (statement_block) @fn_body))) @fn_def

  (method_definition
    body: (statement_block) @fn_body) @fn_def

  (variable_declaration
    (variable_declarator
      value: (arrow_function
        body: (statement_block) @fn_body))) @fn_def
]
"#;

        let method_call_str = r#"
(call_expression
  function: (member_expression
    object: (_) @obj
    property: (property_identifier) @method)
  arguments: (arguments) @args) @call
"#;

        let direct_call_str = r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args) @call
"#;

        Ok(Self {
            async_fn_query: Arc::new(
                Query::new(&js_lang(), async_fn_str)
                    .with_context(|| "failed to compile async_fn query for sync_blocking")?,
            ),
            method_call_query: Arc::new(
                Query::new(&js_lang(), method_call_str)
                    .with_context(|| "failed to compile method_call query for sync_blocking")?,
            ),
            direct_call_query: Arc::new(
                Query::new(&js_lang(), direct_call_str)
                    .with_context(|| "failed to compile direct_call query for sync_blocking")?,
            ),
        })
    }

    /// Collect byte ranges of async function bodies.
    fn find_async_body_ranges(&self, tree: &Tree, source: &[u8]) -> Vec<Range<usize>> {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.async_fn_query, tree.root_node(), source);
        let mut ranges = Vec::new();

        let fn_def_idx = find_capture_index(&self.async_fn_query, "fn_def");
        let fn_body_idx = find_capture_index(&self.async_fn_query, "fn_body");

        while let Some(m) = matches.next() {
            let fn_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_def_idx)
                .map(|c| c.node);
            let body_node = m
                .captures
                .iter()
                .find(|c| c.index as usize == fn_body_idx)
                .map(|c| c.node);

            if let (Some(fn_n), Some(body)) = (fn_node, body_node) {
                // For function declarations and method definitions, "async" appears
                // at the start of the fn_def text. For arrow functions inside variable
                // declarations, fn_def captures the lexical/variable_declaration which
                // starts with const/let/var. In that case, check the arrow_function
                // node (parent of fn_body) instead.
                let is_async = if body.parent().map(|p| p.kind()) == Some("arrow_function") {
                    let arrow_text = node_text(body.parent().unwrap(), source);
                    arrow_text.trim_start().starts_with("async")
                } else {
                    let fn_text = node_text(fn_n, source);
                    fn_text.trim_start().starts_with("async")
                };
                if is_async {
                    ranges.push(body.start_byte()..body.end_byte());
                }
            }
        }

        ranges
    }

    fn is_in_async_body(ranges: &[Range<usize>], byte_offset: usize) -> bool {
        ranges.iter().any(|r| r.contains(&byte_offset))
    }

    fn make_finding(
        &self,
        call_node: tree_sitter::Node,
        source: &[u8],
        file_path: &str,
        message: &str,
    ) -> AuditFinding {
        let start = call_node.start_position();
        AuditFinding {
            file_path: file_path.to_string(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            severity: "warning".to_string(),
            pipeline: self.name().to_string(),
            pattern: "sync_api_call".to_string(),
            message: message.to_string(),
            snippet: extract_snippet(source, call_node, 1),
        }
    }
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects blocking synchronous Node.js calls (fs.*Sync, execSync, spawnSync) inside async functions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let async_ranges = self.find_async_body_ranges(tree, source);
        if async_ranges.is_empty() {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // Check method calls (fs.readFileSync, etc.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.method_call_query, tree.root_node(), source);
            let method_idx = find_capture_index(&self.method_call_query, "method");
            let obj_idx = find_capture_index(&self.method_call_query, "obj");
            let call_idx = find_capture_index(&self.method_call_query, "call");

            while let Some(m) = matches.next() {
                let method_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == method_idx)
                    .map(|c| c.node);
                let obj_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == obj_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(method), Some(obj), Some(call)) = (method_node, obj_node, call_node) {
                    let method_name = node_text(method, source);
                    let obj_name = node_text(obj, source);

                    if SYNC_METHOD_NAMES.contains(&method_name)
                        && Self::is_in_async_body(&async_ranges, call.start_byte())
                    {
                        findings.push(self.make_finding(
                            call,
                            source,
                            file_path,
                            &format!(
                                "`{obj_name}.{method_name}()` blocks the event loop inside an async function — use the async variant instead"
                            ),
                        ));
                    }
                }
            }
        }

        // Check bare function calls (execSync, spawnSync, etc.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&self.direct_call_query, tree.root_node(), source);
            let fn_name_idx = find_capture_index(&self.direct_call_query, "fn_name");
            let call_idx = find_capture_index(&self.direct_call_query, "call");

            while let Some(m) = matches.next() {
                let fn_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == fn_name_idx)
                    .map(|c| c.node);
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(fn_n), Some(call)) = (fn_node, call_node) {
                    let fn_name = node_text(fn_n, source);

                    if SYNC_BARE_CALLS.contains(&fn_name)
                        && Self::is_in_async_body(&async_ranges, call.start_byte())
                    {
                        findings.push(self.make_finding(
                            call,
                            source,
                            file_path,
                            &format!(
                                "`{fn_name}()` blocks the event loop inside an async function — use the async variant instead"
                            ),
                        ));
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
            .set_language(&Language::JavaScript.tree_sitter_language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SyncBlockingInAsyncPipeline::new().unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.js")
    }

    #[test]
    fn detects_read_file_sync_in_async() {
        let src = "\
async function loadData() {
    const data = fs.readFileSync('config.json');
    return JSON.parse(data);
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sync_api_call");
        assert!(findings[0].message.contains("readFileSync"));
    }

    #[test]
    fn detects_write_file_sync_in_async_arrow() {
        let src = "\
const save = async () => {
    fs.writeFileSync('output.json', JSON.stringify(data));
};";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("writeFileSync"));
    }

    #[test]
    fn detects_exec_sync_in_async() {
        let src = "\
async function deploy() {
    execSync('npm run build');
    await upload();
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("execSync"));
    }

    #[test]
    fn detects_spawn_sync_in_async() {
        let src = "\
async function runTests() {
    const result = spawnSync('jest', ['--coverage']);
    return result;
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("spawnSync"));
    }

    #[test]
    fn ignores_sync_calls_in_sync_function() {
        let src = "\
function loadConfig() {
    const data = fs.readFileSync('config.json');
    return JSON.parse(data);
}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_async_fs_calls() {
        let src = "\
async function loadData() {
    const data = await fs.readFile('config.json');
    return JSON.parse(data);
}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_sync_at_top_level() {
        let src = "\
const config = fs.readFileSync('config.json');
execSync('echo hello');";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_multiple_sync_calls_in_async() {
        let src = "\
async function process() {
    const input = fs.readFileSync('input.txt');
    const result = execSync('process ' + input);
    fs.writeFileSync('output.txt', result);
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 3);
    }
}
