use std::collections::HashMap;

use petgraph::Direction;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};
use crate::pipeline::dsl::{MetricValue, NodeType, PipelineNode, WhereClause};
use crate::pipeline::node_helpers::node_path;

pub(crate) fn execute_select(
    node_type: &NodeType,
    filter: Option<&WhereClause>,
    exclude: Option<&WhereClause>,
    graph: &CodeGraph,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<PipelineNode>> {
    let mut result = Vec::new();

    match node_type {
        NodeType::File => {
            for (path, &file_idx) in &graph.file_nodes {
                let node = PipelineNode {
                    node_idx: file_idx,
                    file_path: path.clone(),
                    name: path.clone(),
                    kind: "file".to_string(),
                    line: 1,
                    exported: false,
                    language: match &graph.graph[file_idx] {
                        NodeWeight::File { language, .. } => language.as_str().to_string(),
                        _ => String::new(),
                    },
                    metrics: HashMap::new(),
                };
                if let Some(wc) = filter
                    && !wc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn)
                {
                    continue;
                }
                if let Some(exc) = exclude
                    && exc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn)
                {
                    continue;
                }
                result.push(node);
            }
        }
        NodeType::Symbol => {
            for sym_idx in graph.graph.node_indices() {
                if let NodeWeight::Symbol {
                    name,
                    kind,
                    file_path,
                    start_line,
                    exported,
                    ..
                } = &graph.graph[sym_idx]
                {
                    let language = graph
                        .file_nodes
                        .get(file_path)
                        .and_then(|&file_idx| match &graph.graph[file_idx] {
                            NodeWeight::File { language, .. } => {
                                Some(language.as_str().to_string())
                            }
                            _ => None,
                        })
                        .unwrap_or_default();

                    let mut metrics = HashMap::new();

                    let incoming_external = graph
                        .graph
                        .edges_directed(sym_idx, Direction::Incoming)
                        .filter(|e| {
                            matches!(e.weight(), EdgeWeight::Calls | EdgeWeight::Imports)
                                && node_path(&graph.graph[e.source()]) != *file_path
                        })
                        .count();
                    metrics.insert(
                        "unreferenced".to_string(),
                        MetricValue::Int(if incoming_external == 0 { 1 } else { 0 }),
                    );

                    const ENTRY_POINT_NAMES: &[&str] =
                        &["main", "lib", "mod", "index", "__init__", "__main__"];
                    let stem = std::path::Path::new(file_path.as_str())
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let ep = ENTRY_POINT_NAMES.contains(&stem);
                    metrics.insert(
                        "is_entry_point".to_string(),
                        MetricValue::Int(if ep { 1 } else { 0 }),
                    );

                    let node = PipelineNode {
                        node_idx: sym_idx,
                        file_path: file_path.clone(),
                        name: name.clone(),
                        kind: kind.to_string(),
                        line: *start_line,
                        exported: *exported,
                        language,
                        metrics,
                    };
                    if let Some(wc) = filter
                        && !wc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn)
                    {
                        continue;
                    }
                    if let Some(exc) = exclude
                        && exc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn)
                    {
                        continue;
                    }
                    result.push(node);
                }
            }
        }
        NodeType::CallSite => {
            for idx in graph.graph.node_indices() {
                if let NodeWeight::CallSite {
                    name,
                    file_path,
                    line,
                } = &graph.graph[idx]
                {
                    let language = graph
                        .file_nodes
                        .get(file_path)
                        .and_then(|&file_idx| match &graph.graph[file_idx] {
                            NodeWeight::File { language, .. } => {
                                Some(language.as_str().to_string())
                            }
                            _ => None,
                        })
                        .unwrap_or_default();

                    let node = PipelineNode {
                        node_idx: idx,
                        file_path: file_path.clone(),
                        name: name.clone(),
                        kind: "callsite".to_string(),
                        line: *line,
                        exported: false,
                        language,
                        metrics: HashMap::new(),
                    };
                    if let Some(wc) = filter
                        && !wc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn)
                    {
                        continue;
                    }
                    if let Some(exc) = exclude
                        && exc.eval(&node, is_test_fn, is_generated_fn, is_barrel_fn)
                    {
                        continue;
                    }
                    result.push(node);
                }
            }
        }
    }

    Ok(result)
}
