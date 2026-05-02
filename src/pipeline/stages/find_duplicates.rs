use std::collections::HashMap;

use crate::pipeline::dsl::{FindDuplicatesStage, MetricValue, PipelineNode};

pub(crate) fn execute_find_duplicates(
    stage: &FindDuplicatesStage,
    nodes: Vec<PipelineNode>,
) -> Vec<PipelineNode> {
    let mut groups: HashMap<String, Vec<PipelineNode>> = HashMap::new();
    for node in nodes {
        let key = match stage.by.as_str() {
            "name" => node.name.clone(),
            other => node
                .metrics
                .get(other)
                .map(|v| match v {
                    MetricValue::Text(s) => s.clone(),
                    MetricValue::Int(i) => i.to_string(),
                    MetricValue::Float(f) => f.to_string(),
                })
                .unwrap_or_default(),
        };
        groups.entry(key).or_default().push(node);
    }

    groups
        .into_iter()
        .filter(|(_, members)| members.len() >= stage.min_count)
        .map(|(key, members)| {
            let count = members.len();
            let files: Vec<String> = members.iter().map(|n| n.file_path.clone()).collect();
            let mut rep = members.into_iter().next().unwrap();
            rep.metrics
                .insert("count".to_string(), MetricValue::Int(count as i64));
            rep.metrics
                .insert("files".to_string(), MetricValue::Text(files.join(", ")));
            rep.metrics
                .insert("name".to_string(), MetricValue::Text(key));
            rep
        })
        .collect()
}
