use std::sync::Arc;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::audit::models::AuditFinding;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

use super::primitives::{extract_snippet, find_capture_index, node_text};

/// Method names ending in "Sync" that block the event loop.
const SYNC_SUFFIX_METHODS: &[&str] = &[
    "readFileSync",
    "writeFileSync",
    "appendFileSync",
    "mkdirSync",
    "readdirSync",
    "statSync",
    "unlinkSync",
    "renameSync",
    "copyFileSync",
    "existsSync",
    "accessSync",
    "openSync",
    "closeSync",
    "chmodSync",
    "chownSync",
    "execSync",
    "execFileSync",
    "spawnSync",
];

/// Object.method patterns that block.
const BLOCKING_OBJ_METHOD_PAIRS: &[(&str, &str)] = &[
    ("fs", "readFileSync"),
    ("fs", "writeFileSync"),
    ("fs", "appendFileSync"),
    ("fs", "mkdirSync"),
    ("fs", "readdirSync"),
    ("fs", "statSync"),
    ("fs", "unlinkSync"),
    ("fs", "renameSync"),
    ("fs", "copyFileSync"),
    ("fs", "existsSync"),
    ("fs", "accessSync"),
    ("fs", "openSync"),
    ("fs", "closeSync"),
    ("fs", "chmodSync"),
    ("fs", "chownSync"),
    ("child_process", "execSync"),
    ("child_process", "execFileSync"),
    ("child_process", "spawnSync"),
    ("crypto", "pbkdf2Sync"),
    ("crypto", "scryptSync"),
    ("crypto", "randomFillSync"),
    ("zlib", "deflateSync"),
    ("zlib", "inflateSync"),
    ("zlib", "gzipSync"),
    ("zlib", "gunzipSync"),
];

/// Bare function calls that block.
const BLOCKING_BARE_CALLS: &[&str] = &[
    "readFileSync",
    "writeFileSync",
    "execSync",
    "execFileSync",
    "spawnSync",
];

/// Blocking patterns: `alert`, `prompt`, `confirm` in browser contexts.
const BROWSER_BLOCKING_CALLS: &[&str] = &["alert", "prompt", "confirm"];

pub struct SyncBlockingInAsyncPipeline {
    method_call_query: Arc<Query>,
    direct_call_query: Arc<Query>,
}

impl SyncBlockingInAsyncPipeline {
    pub fn new(language: Language) -> Result<Self> {
        let ts_lang = language.tree_sitter_language();
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
            method_call_query: Arc::new(
                Query::new(&ts_lang, method_call_str)
                    .with_context(|| "failed to compile method_call query for TS sync_blocking")?,
            ),
            direct_call_query: Arc::new(
                Query::new(&ts_lang, direct_call_str)
                    .with_context(|| "failed to compile direct_call query for TS sync_blocking")?,
            ),
        })
    }

    /// Walk up from `node` to determine if it is inside an `async` function.
    /// In tree-sitter-typescript, async functions have the `async` keyword as part of
    /// the function declaration text.
    fn is_inside_async_function(node: tree_sitter::Node, source: &[u8]) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            match parent.kind() {
                "function_declaration"
                | "arrow_function"
                | "function_expression"
                | "method_definition" => {
                    let fn_text = node_text(parent, source);
                    if fn_text.trim_start().starts_with("async") {
                        return true;
                    }
                    // We found the enclosing function and it's not async
                    return false;
                }
                _ => {}
            }
            current = parent.parent();
        }
        false
    }
}

impl Pipeline for SyncBlockingInAsyncPipeline {
    fn name(&self) -> &str {
        "sync_blocking_in_async"
    }

    fn description(&self) -> &str {
        "Detects synchronous blocking calls (*Sync functions, alert/prompt) inside async functions"
    }

    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding> {
        let mut findings = Vec::new();

        // Check method calls (obj.methodSync patterns)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.method_call_query, tree.root_node(), source);
            let obj_idx = find_capture_index(&self.method_call_query, "obj");
            let method_idx = find_capture_index(&self.method_call_query, "method");
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
                let call_node = m
                    .captures
                    .iter()
                    .find(|c| c.index as usize == call_idx)
                    .map(|c| c.node);

                if let (Some(obj), Some(method), Some(call)) = (obj_node, method_node, call_node) {
                    if !Self::is_inside_async_function(call, source) {
                        continue;
                    }

                    let obj_name = node_text(obj, source);
                    let method_name = node_text(method, source);

                    // Check specific obj.method pairs
                    let mut matched = false;
                    for &(expected_obj, expected_method) in BLOCKING_OBJ_METHOD_PAIRS {
                        if obj_name == expected_obj && method_name == expected_method {
                            matched = true;
                            break;
                        }
                    }

                    // Also check if method ends with "Sync"
                    if !matched && SYNC_SUFFIX_METHODS.contains(&method_name) {
                        matched = true;
                    }

                    if matched {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "sync_call_in_async".to_string(),
                            message: format!(
                                "`{obj_name}.{method_name}()` is a blocking call inside an async function — use the async equivalent"
                            ),
                            snippet: extract_snippet(source, call, 1),
                        });
                    }
                }
            }
        }

        // Check bare function calls (readFileSync, alert, prompt, etc.)
        {
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&self.direct_call_query, tree.root_node(), source);
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
                    if !Self::is_inside_async_function(call, source) {
                        continue;
                    }

                    let fn_name = node_text(fn_n, source);
                    let is_blocking = BLOCKING_BARE_CALLS.contains(&fn_name)
                        || BROWSER_BLOCKING_CALLS.contains(&fn_name);

                    if is_blocking {
                        let start = call.start_position();
                        findings.push(AuditFinding {
                            file_path: file_path.to_string(),
                            line: start.row as u32 + 1,
                            column: start.column as u32 + 1,
                            severity: "warning".to_string(),
                            pipeline: self.name().to_string(),
                            pattern: "sync_call_in_async".to_string(),
                            message: format!(
                                "`{fn_name}()` is a blocking call inside an async function — use an async equivalent"
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
        let lang = Language::TypeScript;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let pipeline = SyncBlockingInAsyncPipeline::new(lang).unwrap();
        pipeline.check(&tree, source.as_bytes(), "test.ts")
    }

    #[test]
    fn detects_read_file_sync_in_async() {
        let src = "\
async function loadConfig(): Promise<string> {
    const data = fs.readFileSync('config.json', 'utf8');
    return data;
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern, "sync_call_in_async");
        assert!(findings[0].message.contains("readFileSync"));
    }

    #[test]
    fn detects_exec_sync_in_async_arrow() {
        let src = "\
const deploy = async (): Promise<void> => {
    child_process.execSync('git push');
};";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("execSync"));
    }

    #[test]
    fn ignores_sync_call_in_sync_function() {
        let src = "\
function loadConfig(): string {
    const data = fs.readFileSync('config.json', 'utf8');
    return data;
}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_async_call_in_async_function() {
        let src = "\
async function loadConfig(): Promise<string> {
    const data = await fs.promises.readFile('config.json', 'utf8');
    return data;
}";
        let findings = parse_and_check(src);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_bare_read_file_sync_in_async() {
        let src = "\
async function load(): Promise<Buffer> {
    return readFileSync('data.bin');
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("readFileSync"));
    }

    #[test]
    fn detects_alert_in_async() {
        let src = "\
async function handleClick(): Promise<void> {
    alert('done');
}";
        let findings = parse_and_check(src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("alert"));
    }

    #[test]
    fn tsx_compiles() {
        SyncBlockingInAsyncPipeline::new(Language::Tsx).unwrap();
    }
}
