//! Shared DSL types: graph-shape enums, predicates, runtime values, the
//! execution unit, and the message-interpolation helper.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

use super::WhereClause;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    File,
    Symbol,
    CallSite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Calls,
    Imports,
    FlowsTo,
    Acquires,
    ReleasedBy,
    Contains,
    Exports,
    // SanitizedBy excluded — has a string payload, handled separately in taint analysis
    DefinedIn,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum EdgeDirection {
    In,
    #[default]
    Out,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NumericPredicate {
    #[serde(default)]
    pub gte: Option<f64>,
    #[serde(default)]
    pub lte: Option<f64>,
    #[serde(default)]
    pub gt: Option<f64>,
    #[serde(default)]
    pub lt: Option<f64>,
    #[serde(default)]
    pub eq: Option<f64>,
}

impl NumericPredicate {
    pub fn matches(&self, value: f64) -> bool {
        if let Some(v) = self.gte
            && value < v
        {
            return false;
        }
        if let Some(v) = self.lte
            && value > v
        {
            return false;
        }
        if let Some(v) = self.gt
            && value <= v
        {
            return false;
        }
        if let Some(v) = self.lt
            && value >= v
        {
            return false;
        }
        if let Some(v) = self.eq
            && (value - v).abs() > f64::EPSILON
        {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone)]
pub enum MetricValue {
    Int(i64),
    Float(f64),
    Text(String),
}

impl MetricValue {
    pub fn as_f64(&self) -> f64 {
        match self {
            MetricValue::Int(i) => *i as f64,
            MetricValue::Float(f) => *f,
            MetricValue::Text(_) => 0.0,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            MetricValue::Text(s) => s.as_str(),
            _ => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PipelineNode {
    pub node_idx: NodeIndex,
    pub file_path: String,
    pub name: String,
    pub kind: String,
    pub line: u32,
    pub exported: bool,
    pub language: String,
    /// Computed metrics: count, depth, cycle_size, edge_count, ratio, _group, etc.
    pub metrics: HashMap<String, MetricValue>,
}

impl PipelineNode {
    pub fn metric_f64(&self, key: &str) -> f64 {
        self.metrics.get(key).map(|v| v.as_f64()).unwrap_or(0.0)
    }

    pub fn metric_str(&self, key: &str) -> &str {
        self.metrics.get(key).map(|v| v.as_str()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeverityEntry {
    /// Condition to match. If None or empty, acts as default (always matches).
    #[serde(default)]
    pub when: Option<WhereClause>,
    pub severity: String,
}

/// Interpolate {{var}} template variables in a message string.
/// Available vars: name, kind, file, line, count, depth, cycle_size,
///                 cycle_path, edge_count, ratio, threshold, language, exported
pub fn interpolate_message(template: &str, node: &PipelineNode) -> String {
    let mut result = template.to_string();
    result = result.replace("{{name}}", &node.name);
    result = result.replace("{{kind}}", &node.kind);
    result = result.replace("{{file}}", &node.file_path);
    result = result.replace("{{line}}", &node.line.to_string());
    result = result.replace("{{language}}", &node.language);

    for (key, val) in &node.metrics {
        let placeholder = format!("{{{{{}}}}}", key);
        let value_str = match val {
            MetricValue::Int(i) => i.to_string(),
            MetricValue::Float(f) => format!("{:.2}", f),
            MetricValue::Text(s) => s.clone(),
        };
        result = result.replace(&placeholder, &value_str);
    }
    result
}
