//! Helpers shared between `executor` and individual stage modules.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;

use crate::graph::{CodeGraph, EdgeWeight, NodeWeight, Symbols};
use crate::pipeline::dsl::{EdgeType, MetricValue, PipelineNode};

/// Convert a `NodeIndex` to a `PipelineNode`, returning `None` for
/// unsupported node types (Parameter, ExternalSource).
pub fn pipeline_node_from_index(idx: NodeIndex, graph: &CodeGraph) -> Option<PipelineNode> {
    match &graph.graph[idx] {
        NodeWeight::File { path, language } => Some(PipelineNode {
            node_idx: idx,
            file_path: Some(*path),
            name: Some(*path),
            kind: "file".to_string(),
            line: 1,
            exported: false,
            language: language.as_str().to_string(),
            metrics: HashMap::new(),
            ..Default::default()
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
                file_path: Some(*file_path),
                name: Some(*name),
                kind: kind.to_string(),
                line: *start_line,
                exported: *exported,
                language,
                metrics: HashMap::new(),
                ..Default::default()
            })
        }
        NodeWeight::CallSite {
            name,
            file_path,
            line,
            arg_literals,
            enclosing_test_name,
            ..
        } => {
            let s = &graph.symbols;
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
                file_path: Some(*file_path),
                name: Some(*name),
                kind: "callsite".to_string(),
                line: *line,
                exported: false,
                language,
                metrics: HashMap::new(),
                arg_literals: arg_literals.iter().map(|sp| s.resolve(*sp).to_string()).collect(),
                enclosing_test_name: enclosing_test_name.map(|sp| s.resolve(sp).to_string()),
                ..Default::default()
            })
        }
        NodeWeight::CfgExit {
            function_name,
            file_path,
            line,
            exit_kind,
            exit_label,
            ..
        } => {
            let s = &graph.symbols;
            let language = graph
                .file_nodes
                .get(file_path)
                .and_then(|&file_idx| match &graph.graph[file_idx] {
                    NodeWeight::File { language, .. } => Some(language.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let mut metrics = HashMap::new();
            metrics.insert(
                "exit_kind".to_string(),
                MetricValue::Text(exit_kind.as_str().to_string()),
            );
            metrics.insert(
                "exit_label".to_string(),
                MetricValue::Text(
                    exit_label
                        .map(|sp| s.resolve(sp).to_string())
                        .unwrap_or_default(),
                ),
            );
            Some(PipelineNode {
                node_idx: idx,
                file_path: Some(*file_path),
                name: Some(*function_name),
                kind: "cfg_exit".to_string(),
                line: *line,
                exported: false,
                language,
                metrics,
                ..Default::default()
            })
        }
        NodeWeight::Parameter { .. } | NodeWeight::ExternalSource { .. } => None,
    }
}

/// Check if an `EdgeWeight` matches an `EdgeType`.
pub(crate) fn edge_matches_type(ew: &EdgeWeight, et: &EdgeType) -> bool {
    use crate::graph::CfgExitKind;
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
            | (
                EdgeWeight::ExitsVia(CfgExitKind::Normal),
                EdgeType::ExitsViaNormal
            )
            | (
                EdgeWeight::ExitsVia(CfgExitKind::TrueBranch),
                EdgeType::ExitsViaTrue
            )
            | (
                EdgeWeight::ExitsVia(CfgExitKind::FalseBranch),
                EdgeType::ExitsViaFalse
            )
            | (
                EdgeWeight::ExitsVia(CfgExitKind::Exception),
                EdgeType::ExitsViaException
            )
            | (
                EdgeWeight::ExitsVia(CfgExitKind::Cleanup),
                EdgeType::ExitsViaCleanup
            )
    )
}

/// Extract a display path from a `NodeWeight`, resolved via the graph's
/// interner.
pub(crate) fn node_path(nw: &NodeWeight, symbols: &Symbols) -> String {
    match nw {
        NodeWeight::File { path, .. } => symbols.resolve(*path).to_string(),
        NodeWeight::Symbol { file_path, .. } => symbols.resolve(*file_path).to_string(),
        NodeWeight::CallSite { file_path, .. } => symbols.resolve(*file_path).to_string(),
        NodeWeight::CfgExit { file_path, .. } => symbols.resolve(*file_path).to_string(),
        _ => String::new(),
    }
}
