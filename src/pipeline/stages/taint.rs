use std::collections::HashMap;

use crate::graph::CodeGraph;
use crate::graph::taint::{TaintConfig, TaintEngine};
use crate::pipeline::dsl::{MetricValue, PipelineNode, TaintSinkPattern};

pub(crate) fn execute_taint_with_config(
    config: &TaintConfig,
    graph: &CodeGraph,
    sinks: &[TaintSinkPattern],
) -> anyhow::Result<Vec<PipelineNode>> {
    let findings = TaintEngine::analyze_all(graph, config);

    let nodes = findings
        .into_iter()
        .map(|f| {
            let mut metrics = HashMap::new();
            metrics.insert("sink".to_string(), MetricValue::Text(f.sink_name.clone()));
            let vulnerability = sinks
                .iter()
                .find(|s| f.sink_name.contains(s.pattern.as_str()))
                .map(|s| s.vulnerability.clone())
                .unwrap_or_else(|| "unknown".to_string());
            metrics.insert(
                "vulnerability".to_string(),
                MetricValue::Text(vulnerability),
            );
            metrics.insert(
                "tainted_var".to_string(),
                MetricValue::Text(f.tainted_var.clone()),
            );
            metrics.insert(
                "source_description".to_string(),
                MetricValue::Text(f.source_description.clone()),
            );
            PipelineNode {
                node_idx: f.function_node,
                file_path: f.file_path.clone(),
                name: f.function_name.clone(),
                kind: "taint_finding".to_string(),
                line: f.sink_line,
                exported: false,
                language: String::new(),
                metrics,
            }
        })
        .collect();

    Ok(nodes)
}
