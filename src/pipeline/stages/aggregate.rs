use std::collections::HashMap;

use crate::pipeline::dsl::{MetricValue, NumericPredicate, PipelineNode, RatioConfig};

/// Tag each node with `metrics["_group"] = MetricValue::Text(group_key)`.
pub(crate) fn execute_group_by(
    group_by_field: &str,
    mut nodes: Vec<PipelineNode>,
) -> Vec<PipelineNode> {
    for node in &mut nodes {
        let group_key = match group_by_field {
            "file_path" | "file" => node.file_path.clone(),
            "language" => node.language.clone(),
            "kind" => node.kind.clone(),
            "name" => node.name.clone(),
            other => node
                .metrics
                .get(other)
                .map(|v| v.as_str().to_string())
                .unwrap_or_default(),
        };
        node.metrics
            .insert("_group".to_string(), MetricValue::Text(group_key));
    }
    nodes
}

/// Group nodes by `_group` metric, count members per group, keep groups
/// whose count satisfies the threshold predicate. Emits one representative
/// `PipelineNode` per surviving group with `metrics["count"]` set.
pub(crate) fn execute_count(
    threshold: &NumericPredicate,
    nodes: Vec<PipelineNode>,
) -> Vec<PipelineNode> {
    let mut group_order: Vec<String> = Vec::new();
    let mut group_map: HashMap<String, Vec<PipelineNode>> = HashMap::new();

    for node in nodes {
        let group_key = node.metric_str("_group").to_string();
        if !group_map.contains_key(&group_key) {
            group_order.push(group_key.clone());
        }
        group_map.entry(group_key).or_default().push(node);
    }

    let mut result = Vec::new();
    for key in &group_order {
        let members = &group_map[key];
        let count = members.len() as f64;
        if !threshold.matches(count) {
            continue;
        }
        let mut rep = members[0].clone();
        rep.metrics
            .insert("count".to_string(), MetricValue::Int(members.len() as i64));
        result.push(rep);
    }
    result
}

pub(crate) fn execute_ratio(
    config: &RatioConfig,
    nodes: Vec<PipelineNode>,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<PipelineNode>> {
    let mut group_order: Vec<String> = Vec::new();
    let mut group_map: HashMap<String, Vec<PipelineNode>> = HashMap::new();

    for node in nodes {
        let group_key = node.metric_str("_group").to_string();
        if !group_map.contains_key(&group_key) {
            group_order.push(group_key.clone());
        }
        group_map.entry(group_key).or_default().push(node);
    }

    let mut result = Vec::new();

    for key in &group_order {
        let members = &group_map[key];

        let numerator_count = members
            .iter()
            .filter(|n| {
                if let Some(wc) = &config.numerator.filter {
                    wc.eval(n, is_test_fn, is_generated_fn, is_barrel_fn)
                } else {
                    true
                }
            })
            .count();

        let denominator_count = members
            .iter()
            .filter(|n| {
                if let Some(wc) = &config.denominator.filter {
                    wc.eval(n, is_test_fn, is_generated_fn, is_barrel_fn)
                } else {
                    true
                }
            })
            .count();

        if denominator_count == 0 {
            continue;
        }

        let ratio = numerator_count as f64 / denominator_count as f64;

        let mut rep = members[0].clone();
        rep.metrics.insert(
            "count".to_string(),
            MetricValue::Int(numerator_count as i64),
        );
        rep.metrics
            .insert("ratio".to_string(), MetricValue::Float(ratio));

        if let Some(wc) = &config.threshold
            && !wc.eval_metrics(&rep)
        {
            continue;
        }

        result.push(rep);
    }

    Ok(result)
}
