//! Helpers shared between `executor` and individual stage modules.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;

use crate::graph::{CodeGraph, EdgeWeight, NodeWeight};
use crate::pipeline::dsl::{EdgeType, PipelineNode};

/// Convert a `NodeIndex` to a `PipelineNode`, returning `None` for
/// unsupported node types (Parameter, ExternalSource).
pub fn pipeline_node_from_index(idx: NodeIndex, graph: &CodeGraph) -> Option<PipelineNode> {
    match &graph.graph[idx] {
        NodeWeight::File { path, language } => Some(PipelineNode {
            node_idx: idx,
            file_path: path.clone(),
            name: path.clone(),
            kind: "file".to_string(),
            line: 1,
            exported: false,
            language: language.as_str().to_string(),
            metrics: HashMap::new(),
        }),
        NodeWeight::Symbol {
            name,
            kind,
            file_path,
            start_line,
            exported,
            ..
        } => {
            let language = graph
                .file_nodes
                .get(file_path)
                .and_then(|&file_idx| match &graph.graph[file_idx] {
                    NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            Some(PipelineNode {
                node_idx: idx,
                file_path: file_path.clone(),
                name: name.clone(),
                kind: kind.to_string(),
                line: *start_line,
                exported: *exported,
                language,
                metrics: HashMap::new(),
            })
        }
        NodeWeight::CallSite {
            name,
            file_path,
            line,
        } => {
            let language = graph
                .file_nodes
                .get(file_path)
                .and_then(|&file_idx| match &graph.graph[file_idx] {
                    NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            Some(PipelineNode {
                node_idx: idx,
                file_path: file_path.clone(),
                name: name.clone(),
                kind: "callsite".to_string(),
                line: *line,
                exported: false,
                language,
                metrics: HashMap::new(),
            })
        }
        NodeWeight::Parameter { .. } | NodeWeight::ExternalSource { .. } => None,
    }
}

/// Check if an `EdgeWeight` matches an `EdgeType`.
pub(crate) fn edge_matches_type(ew: &EdgeWeight, et: &EdgeType) -> bool {
    matches!(
        (ew, et),
        (EdgeWeight::Calls, EdgeType::Calls)
            | (EdgeWeight::Imports, EdgeType::Imports)
            | (EdgeWeight::FlowsTo, EdgeType::FlowsTo)
            | (EdgeWeight::Acquires { .. }, EdgeType::Acquires)
            | (EdgeWeight::ReleasedBy, EdgeType::ReleasedBy)
            | (EdgeWeight::Contains, EdgeType::Contains)
            | (EdgeWeight::Exports, EdgeType::Exports)
            | (EdgeWeight::DefinedIn, EdgeType::DefinedIn)
    )
}

/// Extract a display path from a `NodeWeight`.
pub(crate) fn node_path(nw: &NodeWeight) -> String {
    match nw {
        NodeWeight::File { path, .. } => path.clone(),
        NodeWeight::Symbol { file_path, .. } => file_path.clone(),
        NodeWeight::CallSite { file_path, .. } => file_path.clone(),
        _ => String::new(),
    }
}
