use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use regex::Regex;

use crate::graph::{CodeGraph, NodeWeight};
use crate::language::{self, Language};
use crate::languages;
use crate::models::{CommentInfo, SymbolInfo, SymbolKind};
use crate::parser;
use crate::query::lang::{FindFilter, HasFilter, NameFilter, TsQuery};
use crate::signature;
use crate::storage::registry::ProjectEntry;
use crate::storage::workspace::Workspace;

pub use crate::pipeline::output::{AuditFinding, QueryResult};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReadResult {
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub total_lines: u32,
    pub content: String,
}

#[derive(Debug, serde::Serialize)]
pub struct QueryOutput {
    pub results: Vec<QueryResult>,
    pub files_parsed: usize,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read: Option<ReadResult>,
    /// Set when graph pipeline ends with Flag stage — contains AuditFindings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings: Option<Vec<crate::pipeline::output::AuditFinding>>,
}

pub fn execute(
    project: &ProjectEntry,
    query: &TsQuery,
    max: usize,
    workspace: &Workspace,
    graph: &CodeGraph,
) -> Result<QueryOutput> {
    // Handle read mode: return file content instead of symbol search
    if let Some(ref file_path) = query.read {
        return execute_read(workspace, file_path, query.lines.as_ref());
    }

    let languages = match &project.languages {
        Some(f) => language::parse_language_filter(f),
        None => Language::all().to_vec(),
    };

    // Get files from workspace (already relative paths)
    let all_files: Vec<&str> = workspace.files().iter().map(|s| s.as_str()).collect();

    // Apply file glob filter
    let files: Vec<&str> = if let Some(ref file_filter) = query.files {
        let globset = build_globset(&file_filter.patterns())?;
        all_files
            .into_iter()
            .filter(|path| globset.is_match(*path))
            .collect()
    } else {
        all_files
    };

    // Apply files_exclude
    let files: Vec<&str> = if let Some(ref excludes) = query.files_exclude {
        let patterns: Vec<&str> = excludes.iter().map(|s| s.as_str()).collect();
        let excludeset = build_globset(&patterns)?;
        files
            .into_iter()
            .filter(|path| !excludeset.is_match(*path))
            .collect()
    } else {
        files
    };

    // Resolve find kinds
    let find_kinds = resolve_find_kinds(query.find.as_ref());

    // Compile name matcher
    let name_matcher = compile_name_matcher(query.name.as_ref())?;

    // Pre-compile tree-sitter queries per language
    let mut sym_queries: HashMap<Language, Arc<tree_sitter::Query>> = HashMap::new();
    let mut cmt_queries: HashMap<Language, Arc<tree_sitter::Query>> = HashMap::new();
    for lang in &languages {
        sym_queries.insert(*lang, languages::compile_symbol_query(*lang)?);
        cmt_queries.insert(*lang, languages::compile_comment_query(*lang)?);
    }
    let sym_queries = Arc::new(sym_queries);
    let cmt_queries = Arc::new(cmt_queries);

    let need_comments = query.has.is_some();

    let include_body = query.body.unwrap_or(false);
    let preview_lines = query.preview;

    let visibility = query.visibility.as_deref();
    let inside = query.inside.as_deref();
    let lines_filter = query.lines.as_ref();
    let has_filter = query.has.as_ref();

    let files_parsed = files.len();

    // Parallel parse + filter
    let per_file_results: Vec<Vec<QueryResult>> = files
        .into_par_iter()
        .filter_map(|rel_path| {
            let lang = workspace.file_language(rel_path)?;
            let sym_query = sym_queries.get(&lang)?;
            let cmt_query = cmt_queries.get(&lang)?;

            let source = workspace.read_file(rel_path)?;
            let mut ts_parser = parser::create_parser(lang).ok()?;
            let (metadata, tree) =
                parser::parse_content(&mut ts_parser, &source, rel_path, lang).ok()?;

            let all_symbols = languages::extract_symbols(
                &tree,
                source.as_bytes(),
                sym_query,
                &metadata.path,
                lang,
            );

            let comments = if need_comments {
                languages::extract_comments(
                    &tree,
                    source.as_bytes(),
                    cmt_query,
                    &metadata.path,
                    lang,
                )
            } else {
                Vec::new()
            };

            let source_lines: Vec<&str> = source.lines().collect();

            let mut results = Vec::new();

            for sym in &all_symbols {
                // find filter
                if let Some(ref kinds) = find_kinds
                    && !kinds.contains(&sym.kind)
                {
                    continue;
                }

                // name filter
                if let Some(ref matcher) = name_matcher
                    && !matcher.matches(&sym.name)
                {
                    continue;
                }

                // visibility filter
                if let Some(vis) = visibility {
                    let matches = match vis {
                        "exported" | "public" => sym.is_exported,
                        "private" | "protected" | "internal" => !sym.is_exported,
                        _ => true,
                    };
                    if !matches {
                        continue;
                    }
                }

                // lines filter
                if let Some(lr) = lines_filter {
                    let line_count = sym.end_line.saturating_sub(sym.start_line) + 1;
                    if let Some(min) = lr.min
                        && line_count < min
                    {
                        continue;
                    }
                    if let Some(max) = lr.max
                        && line_count > max
                    {
                        continue;
                    }
                }

                // inside filter
                if let Some(parent_name) = inside {
                    let is_inside = all_symbols.iter().any(|s| {
                        s.name == parent_name
                            && s.start_line <= sym.start_line
                            && s.end_line >= sym.end_line
                            && !(s.start_line == sym.start_line
                                && s.end_line == sym.end_line
                                && s.name == sym.name)
                    });
                    if !is_inside {
                        continue;
                    }
                }

                // has filter
                if let Some(hf) = has_filter
                    && !check_has_filter(hf, sym, &comments)
                {
                    continue;
                }

                // Extract signature
                let signature = signature::extract_signature(&source, sym.start_line, lang);

                // Extract docstring from associated comments
                let docstring = comments.iter().find_map(|c| {
                    if c.associated_symbol.as_deref() == Some(&sym.name) && c.kind == "doc" {
                        Some(c.text.clone())
                    } else {
                        None
                    }
                });

                // Extract body
                let body = if include_body {
                    extract_lines(&source_lines, sym.start_line, sym.end_line)
                } else {
                    None
                };

                // Extract preview
                let preview = preview_lines.map(|n| {
                    let end = std::cmp::min(sym.start_line + n as u32 - 1, sym.end_line);
                    extract_lines(&source_lines, sym.start_line, end).unwrap_or_default()
                });

                // Determine parent
                let parent = find_parent(sym, &all_symbols);

                results.push(QueryResult {
                    name: sym.name.clone(),
                    kind: sym.kind.to_string(),
                    file: metadata.path.clone(),
                    line: sym.start_line,
                    end_line: sym.end_line,
                    column: sym.start_column,
                    exported: sym.is_exported,
                    signature,
                    docstring,
                    body,
                    preview,
                    parent,
                });
            }

            Some(results)
        })
        .collect();

    // Flatten, sort, limit
    let mut results: Vec<QueryResult> = per_file_results.into_iter().flatten().collect();
    results.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    // Call graph traversal if requested
    if let Some(ref direction) = query.calls {
        let depth = query.depth.unwrap_or(1).min(5);
        results = traverse_via_graph(graph, &results, direction, depth, workspace);
    }

    // Graph pipeline stages if present
    if let Some(ref stages) = query.graph {
        use petgraph::graph::NodeIndex;

        // Convert QueryResult seed nodes to NodeIndex via graph
        let seeds: Vec<NodeIndex> = results
            .iter()
            .filter_map(|r| graph.find_symbol(&r.file, r.line))
            .collect();

        // If no find filter was used, seeds are None (select stage handles it)
        let seed_nodes = if query.find.is_some() || query.name.is_some() {
            Some(seeds)
        } else {
            None
        };

        let pipeline_name = "graph_query";
        match crate::pipeline::executor::run_pipeline(
            stages,
            graph,
            None,
            None,
            seed_nodes,
            pipeline_name,
        )? {
            crate::pipeline::executor::PipelineOutput::Findings(findings) => {
                return Ok(QueryOutput {
                    results: Vec::new(),
                    files_parsed,
                    total: findings.len(),
                    read: None,
                    findings: Some(findings),
                });
            }
            crate::pipeline::executor::PipelineOutput::Results(graph_results) => {
                results = graph_results;
            }
        }
    }

    let total = results.len();
    results.truncate(max);

    Ok(QueryOutput {
        results,
        files_parsed,
        total,
        read: None,
        findings: None,
    })
}

fn execute_read(
    workspace: &Workspace,
    file_path: &str,
    lines: Option<&crate::query::lang::LineRange>,
) -> Result<QueryOutput> {
    let source = workspace
        .read_file(file_path)
        .or_else(|| {
            // Fallback: read from disk for files not in workspace (e.g., non-language files)
            // Skip for S3 workspaces where root is a synthetic path
            let root = workspace.root();
            if root.exists() {
                std::fs::read_to_string(root.join(file_path))
                    .ok()
                    .map(|s| Arc::from(s.as_str()))
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("file not found: {file_path}"))?;

    let all_lines: Vec<&str> = source.lines().collect();
    let total_lines = all_lines.len() as u32;

    let (start, end) = match lines {
        Some(lr) => {
            let s = lr.min.unwrap_or(1).max(1);
            let e = lr.max.unwrap_or(total_lines).min(total_lines);
            (s, e)
        }
        None => (1, total_lines),
    };

    let start_idx = (start - 1) as usize;
    let end_idx = std::cmp::min(end as usize, all_lines.len());
    let content = if start_idx < all_lines.len() {
        all_lines[start_idx..end_idx]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>4}  {}", start_idx + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    Ok(QueryOutput {
        results: Vec::new(),
        files_parsed: 0,
        total: 0,
        read: Some(ReadResult {
            file: file_path.to_string(),
            start_line: start,
            end_line: end,
            total_lines,
            content,
        }),
        findings: None,
    })
}

fn build_globset(patterns: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder
            .add(Glob::new(pattern).with_context(|| format!("invalid glob pattern: {pattern}"))?);
    }
    builder.build().context("failed to build glob set")
}

fn resolve_find_kinds(find: Option<&FindFilter>) -> Option<Vec<SymbolKind>> {
    let filter = find?;
    let kinds: Vec<SymbolKind> = filter
        .kinds()
        .iter()
        .flat_map(|k| match *k {
            "function" => vec![SymbolKind::Function, SymbolKind::ArrowFunction],
            "method" => vec![SymbolKind::Method],
            "class" => vec![SymbolKind::Class],
            "interface" => vec![SymbolKind::Interface],
            "type" => vec![SymbolKind::TypeAlias, SymbolKind::Typedef],
            "enum" => vec![SymbolKind::Enum],
            "struct" => vec![SymbolKind::Struct],
            "trait" => vec![SymbolKind::Trait],
            "variable" => vec![SymbolKind::Variable],
            "constant" => vec![SymbolKind::Constant],
            "property" => vec![SymbolKind::Property],
            "namespace" => vec![SymbolKind::Namespace],
            "module" => vec![SymbolKind::Module],
            "macro" => vec![SymbolKind::Macro],
            "union" => vec![SymbolKind::Union],
            "arrow_function" => vec![SymbolKind::ArrowFunction],
            "constructor" => vec![SymbolKind::Method], // post-filter by name handled separately
            "import" => vec![],                        // handled separately
            "any" => vec![
                SymbolKind::Function,
                SymbolKind::ArrowFunction,
                SymbolKind::Class,
                SymbolKind::Method,
                SymbolKind::Variable,
                SymbolKind::Interface,
                SymbolKind::TypeAlias,
                SymbolKind::Enum,
                SymbolKind::Struct,
                SymbolKind::Union,
                SymbolKind::Namespace,
                SymbolKind::Macro,
                SymbolKind::Property,
                SymbolKind::Typedef,
                SymbolKind::Trait,
                SymbolKind::Constant,
                SymbolKind::Module,
            ],
            _ => vec![],
        })
        .collect();

    if kinds.is_empty() { None } else { Some(kinds) }
}

enum NameMatcher {
    GlobSet(GlobSet),
    Contains(String),
    Regex(Regex),
}

impl NameMatcher {
    fn matches(&self, name: &str) -> bool {
        match self {
            NameMatcher::GlobSet(gs) => gs.is_match(name),
            NameMatcher::Contains(s) => name.contains(s.as_str()),
            NameMatcher::Regex(r) => r.is_match(name),
        }
    }
}

fn compile_name_matcher(name: Option<&NameFilter>) -> Result<Option<NameMatcher>> {
    let filter = match name {
        Some(f) => f,
        None => return Ok(None),
    };

    match filter {
        NameFilter::Glob(pattern) => {
            let mut builder = GlobSetBuilder::new();
            builder
                .add(Glob::new(pattern).with_context(|| format!("invalid name glob: {pattern}"))?);
            Ok(Some(NameMatcher::GlobSet(builder.build()?)))
        }
        NameFilter::Complex { contains, regex } => {
            if let Some(r) = regex {
                let re = Regex::new(r).with_context(|| format!("invalid regex: {r}"))?;
                Ok(Some(NameMatcher::Regex(re)))
            } else if let Some(c) = contains {
                Ok(Some(NameMatcher::Contains(c.clone())))
            } else {
                Ok(None)
            }
        }
    }
}

fn check_has_filter(filter: &HasFilter, sym: &SymbolInfo, comments: &[CommentInfo]) -> bool {
    match filter {
        HasFilter::Single(text) => has_associated_text(sym, comments, text),
        HasFilter::Multiple(texts) => texts.iter().all(|t| has_associated_text(sym, comments, t)),
        HasFilter::Not { not } => {
            if not == "docstring" {
                !comments
                    .iter()
                    .any(|c| c.associated_symbol.as_deref() == Some(&sym.name) && c.kind == "doc")
            } else {
                !has_associated_text(sym, comments, not)
            }
        }
    }
}

fn has_associated_text(sym: &SymbolInfo, comments: &[CommentInfo], text: &str) -> bool {
    // Check comments associated with this symbol
    comments
        .iter()
        .any(|c| c.associated_symbol.as_deref() == Some(&sym.name) && c.text.contains(text))
}

fn extract_lines(source_lines: &[&str], start: u32, end: u32) -> Option<String> {
    if start == 0 || end == 0 {
        return None;
    }
    let start_idx = (start - 1) as usize;
    let end_idx = std::cmp::min(end as usize, source_lines.len());
    if start_idx >= source_lines.len() {
        return None;
    }
    Some(source_lines[start_idx..end_idx].join("\n"))
}

fn find_parent(sym: &SymbolInfo, all_symbols: &[SymbolInfo]) -> Option<String> {
    all_symbols
        .iter()
        .filter(|s| {
            s.start_line < sym.start_line && s.end_line > sym.end_line && s.name != sym.name
        })
        .min_by_key(|s| s.end_line - s.start_line)
        .map(|s| s.name.clone())
}

fn traverse_via_graph(
    graph: &CodeGraph,
    seeds: &[QueryResult],
    direction: &str,
    max_depth: usize,
    workspace: &Workspace,
) -> Vec<QueryResult> {
    // Map seed results to NodeIndex
    let seed_indices: Vec<_> = seeds
        .iter()
        .filter_map(|r| graph.find_symbol(&r.file, r.line))
        .collect();

    let result_indices = match direction {
        "down" => graph.traverse_callees(&seed_indices, max_depth),
        "up" => graph.traverse_callers(&seed_indices, max_depth),
        "both" => {
            let mut combined = graph.traverse_callees(&seed_indices, max_depth);
            let callers = graph.traverse_callers(&seed_indices, max_depth);
            for idx in callers {
                if !combined.contains(&idx) {
                    combined.push(idx);
                }
            }
            combined
        }
        _ => Vec::new(),
    };

    // Convert NodeIndex results back to QueryResult
    let mut results: Vec<QueryResult> = result_indices
        .iter()
        .filter_map(|&idx| match &graph.graph[idx] {
            NodeWeight::Symbol {
                name,
                kind,
                file_path,
                start_line,
                end_line,
                exported,
            } => {
                let sig = workspace.read_file(file_path).and_then(|source| {
                    let lang = workspace.file_language(file_path)?;
                    signature::extract_signature(&source, *start_line, lang)
                });

                Some(QueryResult {
                    name: name.clone(),
                    kind: kind.to_string(),
                    file: file_path.clone(),
                    line: *start_line,
                    end_line: *end_line,
                    column: 0,
                    exported: *exported,
                    signature: sig,
                    docstring: None,
                    body: None,
                    preview: None,
                    parent: None,
                })
            }
            _ => None,
        })
        .collect();

    results.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_function_kinds() {
        let filter = FindFilter::Single("function".to_string());
        let kinds = resolve_find_kinds(Some(&filter)).unwrap();
        assert!(kinds.contains(&SymbolKind::Function));
        assert!(kinds.contains(&SymbolKind::ArrowFunction));
    }

    #[test]
    fn resolve_any_kinds() {
        let filter = FindFilter::Single("any".to_string());
        let kinds = resolve_find_kinds(Some(&filter)).unwrap();
        assert!(kinds.len() >= 10);
    }

    #[test]
    fn name_glob_matching() {
        let matcher = compile_name_matcher(Some(&NameFilter::Glob("handle*".to_string())))
            .unwrap()
            .unwrap();
        assert!(matcher.matches("handleClick"));
        assert!(matcher.matches("handleSubmit"));
        assert!(!matcher.matches("onClick"));
    }

    #[test]
    fn name_contains_matching() {
        let matcher = compile_name_matcher(Some(&NameFilter::Complex {
            contains: Some("auth".to_string()),
            regex: None,
        }))
        .unwrap()
        .unwrap();
        assert!(matcher.matches("authenticate"));
        assert!(matcher.matches("oauth_token"));
        assert!(!matcher.matches("login"));
        assert!(!matcher.matches("isAuthValid")); // case-sensitive: "Auth" != "auth"
    }

    #[test]
    fn name_regex_matching() {
        let matcher = compile_name_matcher(Some(&NameFilter::Complex {
            contains: None,
            regex: Some("^get[A-Z]".to_string()),
        }))
        .unwrap()
        .unwrap();
        assert!(matcher.matches("getUser"));
        assert!(matcher.matches("getName"));
        assert!(!matcher.matches("fetchUser"));
        assert!(!matcher.matches("getter"));
    }

    // Tests for the graph pipeline field in TsQuery / QueryOutput

    #[test]
    fn graph_pipeline_flag_produces_findings() {
        use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};
        use crate::language::Language;
        use crate::pipeline::dsl::{FlagConfig, GraphStage, NodeType};

        // Build a minimal CodeGraph with one file node and one symbol node
        let mut g = CodeGraph::new();
        let file_idx = g.graph.add_node(NodeWeight::File {
            path: "src/big.rs".to_string(),
            language: Language::Rust,
        });
        g.file_nodes.insert("src/big.rs".to_string(), file_idx);
        let sym_idx = g.graph.add_node(NodeWeight::Symbol {
            name: "my_fn".to_string(),
            kind: crate::models::SymbolKind::Function,
            file_path: "src/big.rs".to_string(),
            start_line: 1,
            end_line: 10,
            exported: true,
        });
        g.graph.add_edge(file_idx, sym_idx, EdgeWeight::Contains);
        g.symbol_nodes
            .insert(("src/big.rs".to_string(), 1), sym_idx);

        // Build a graph pipeline: select(file) -> flag
        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::Flag {
                flag: FlagConfig {
                    pattern: "test_pattern".to_string(),
                    message: "Found {{file}}".to_string(),
                    severity: Some("info".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];

        // Build a TsQuery with graph stages
        let query: crate::query::lang::TsQuery = serde_json::from_str(
            &serde_json::json!({
                "graph": stages
            })
            .to_string(),
        )
        .unwrap();

        assert!(query.graph.is_some());
        assert_eq!(query.graph.as_ref().unwrap().len(), 2);

        // Run the pipeline stages directly via executor to verify Findings output
        let out = crate::pipeline::executor::run_pipeline(
            query.graph.as_ref().unwrap(),
            &g,
            None,
            None,
            None,
            "graph_query",
        )
        .unwrap();

        match out {
            crate::pipeline::executor::PipelineOutput::Findings(findings) => {
                assert_eq!(findings.len(), 1);
                assert_eq!(findings[0].severity, "info");
                assert_eq!(findings[0].pattern, "test_pattern");
                assert!(findings[0].message.contains("src/big.rs"));
            }
            _ => panic!("expected Findings output"),
        }
    }

    #[test]
    fn graph_pipeline_select_produces_results() {
        use crate::graph::{CodeGraph, NodeWeight};
        use crate::language::Language;
        use crate::pipeline::dsl::{GraphStage, NodeType};

        let mut g = CodeGraph::new();
        let file_idx = g.graph.add_node(NodeWeight::File {
            path: "src/a.rs".to_string(),
            language: Language::Rust,
        });
        g.file_nodes.insert("src/a.rs".to_string(), file_idx);

        let stages = vec![GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        }];

        let out =
            crate::pipeline::executor::run_pipeline(&stages, &g, None, None, None, "graph_query")
                .unwrap();

        match out {
            crate::pipeline::executor::PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].file, "src/a.rs");
            }
            _ => panic!("expected Results output"),
        }
    }

    #[test]
    fn query_output_findings_field_serializes() {
        use crate::pipeline::output::AuditFinding;

        let output = QueryOutput {
            results: Vec::new(),
            files_parsed: 5,
            total: 1,
            read: None,
            findings: Some(vec![AuditFinding {
                file_path: "src/a.rs".to_string(),
                line: 10,
                column: 1,
                severity: "warning".to_string(),
                pipeline: "graph_query".to_string(),
                pattern: "test".to_string(),
                message: "Found issue".to_string(),
                snippet: String::new(),
            }]),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"findings\""));
        assert!(json.contains("src/a.rs"));
        assert!(json.contains("warning"));
    }

    #[test]
    fn query_output_no_findings_field_skipped() {
        let output = QueryOutput {
            results: Vec::new(),
            files_parsed: 0,
            total: 0,
            read: None,
            findings: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        // findings: None should be omitted from JSON (skip_serializing_if)
        assert!(!json.contains("\"findings\""));
    }
}
