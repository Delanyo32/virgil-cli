use petgraph::Direction;

use crate::graph::{CodeGraph, EdgeWeight};
use crate::language::Language;
use crate::pipeline::dsl::{MetricValue, PipelineNode};
use crate::storage::workspace::Workspace;

pub(crate) fn execute_compute_metric(
    metric_name: &str,
    nodes: Vec<PipelineNode>,
    workspace: &Workspace,
    graph: &CodeGraph,
) -> anyhow::Result<Vec<PipelineNode>> {
    // Graph-only metrics (no workspace/AST needed)
    match metric_name {
        "efferent_coupling" => {
            let mut result = nodes;
            for node in &mut result {
                let count = graph
                    .graph
                    .edges_directed(node.node_idx, Direction::Outgoing)
                    .filter(|e| matches!(e.weight(), EdgeWeight::Imports))
                    .count();
                node.metrics.insert(
                    "efferent_coupling".to_string(),
                    MetricValue::Int(count as i64),
                );
            }
            return Ok(result);
        }
        "afferent_coupling" => {
            let mut result = nodes;
            for node in &mut result {
                let count = graph
                    .graph
                    .edges_directed(node.node_idx, Direction::Incoming)
                    .filter(|e| matches!(e.weight(), EdgeWeight::Imports | EdgeWeight::Calls))
                    .count();
                node.metrics.insert(
                    "afferent_coupling".to_string(),
                    MetricValue::Int(count as i64),
                );
            }
            return Ok(result);
        }
        _ => {}
    }

    let mut result = Vec::new();

    for mut node in nodes {
        let Some(lang) = workspace.file_language(&node.file_path) else {
            result.push(node);
            continue;
        };
        let Some(source) = workspace.read_file(&node.file_path) else {
            result.push(node);
            continue;
        };

        let mut parser = crate::parser::create_parser(lang)?;
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => {
                eprintln!(
                    "Warning: compute_metric: failed to parse {}",
                    node.file_path
                );
                result.push(node);
                continue;
            }
        };

        let config = crate::graph::metrics::control_flow_config_for_language(lang);
        let target_line = node.line.saturating_sub(1) as usize;

        if metric_name == "comment_to_code_ratio" {
            let (comment_lines, code_lines) = crate::graph::metrics::compute_comment_ratio(
                tree.root_node(),
                source.as_bytes(),
                &config,
            );
            let ratio = if code_lines > 0 {
                (comment_lines as f64 / (comment_lines + code_lines) as f64 * 100.0) as i64
            } else {
                0
            };
            node.metrics
                .insert(metric_name.to_string(), MetricValue::Int(ratio));
            result.push(node);
            continue;
        }

        let body_node = find_function_body_at_line(tree.root_node(), target_line, lang);
        let Some(body) = body_node else {
            eprintln!(
                "Warning: compute_metric: no function body at line {} in {}",
                node.line, node.file_path
            );
            result.push(node);
            continue;
        };

        let value: i64 = match metric_name {
            "cyclomatic_complexity" => {
                crate::graph::metrics::compute_cyclomatic(body, &config, source.as_bytes()) as i64
            }
            "function_length" => {
                let (lines, _) = crate::graph::metrics::count_function_lines(body);
                lines as i64
            }
            "cognitive_complexity" => {
                crate::graph::metrics::compute_cognitive(body, &config, source.as_bytes()) as i64
            }
            "nesting_depth" => crate::graph::metrics::compute_nesting_depth(body, &config) as i64,
            other => {
                anyhow::bail!(
                    "compute_metric: unknown metric '{}' -- supported: cyclomatic_complexity, function_length, cognitive_complexity, comment_to_code_ratio, nesting_depth, efferent_coupling, afferent_coupling",
                    other
                );
            }
        };

        node.metrics
            .insert(metric_name.to_string(), MetricValue::Int(value));
        result.push(node);
    }

    Ok(result)
}

/// Walk the tree to find a function node whose start line matches `target_line`,
/// then return its body child.
fn find_function_body_at_line(
    root: tree_sitter::Node,
    target_line: usize,
    lang: Language,
) -> Option<tree_sitter::Node> {
    let func_kinds = crate::graph::metrics::function_node_kinds_for_language(lang);
    let body_field = crate::graph::metrics::body_field_for_language(lang);

    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if func_kinds.contains(&current.kind())
            && current.start_position().row == target_line
            && let Some(body) = current.child_by_field_name(body_field)
        {
            return Some(body);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
    None
}
