//! Shared DSL types: graph-shape enums, predicates, runtime values, the
//! execution unit, and the message-interpolation helper.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

use crate::graph::{Spur, Symbols};

use super::WhereClause;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    File,
    Symbol,
    CallSite,
    CfgExit,
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
    ExitsViaNormal,
    ExitsViaTrue,
    ExitsViaFalse,
    ExitsViaException,
    ExitsViaCleanup,
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
    /// Interned file path. `None` only for transient default-constructed
    /// nodes; populated nodes always carry a `Spur`. Resolve through the
    /// graph's `Symbols` handle.
    pub file_path: Option<Spur>,
    /// Interned symbol name. Same conventions as `file_path`.
    pub name: Option<Spur>,
    pub kind: String,
    pub line: u32,
    pub exported: bool,
    pub language: String,
    /// Computed metrics: count, depth, cycle_size, edge_count, ratio, _group, etc.
    pub metrics: HashMap<String, MetricValue>,
    /// match_pattern capture name -> captured text. Keys and values are
    /// interned to deduplicate across nodes (capture names repeat heavily).
    pub captures: HashMap<Spur, Spur>,
    /// Literal arguments at this call site (CallSite nodes only).
    pub arg_literals: Vec<String>,
    /// Name of the enclosing test function, if this CallSite is inside one.
    pub enclosing_test_name: Option<String>,
}

impl Default for PipelineNode {
    fn default() -> Self {
        Self {
            node_idx: NodeIndex::new(0),
            file_path: None,
            name: None,
            kind: String::new(),
            line: 0,
            exported: false,
            language: String::new(),
            metrics: HashMap::new(),
            captures: HashMap::new(),
            arg_literals: Vec::new(),
            enclosing_test_name: None,
        }
    }
}

impl PipelineNode {
    pub fn metric_f64(&self, key: &str) -> f64 {
        self.metrics.get(key).map(|v| v.as_f64()).unwrap_or(0.0)
    }

    pub fn metric_str(&self, key: &str) -> &str {
        self.metrics.get(key).map(|v| v.as_str()).unwrap_or("")
    }

    /// Resolve `file_path` through the interner, returning `""` if unset.
    pub fn file_path_str<'a>(&self, symbols: &'a Symbols) -> &'a str {
        self.file_path
            .map(|s| symbols.resolve(s))
            .unwrap_or("")
    }

    /// Resolve `name` through the interner, returning `""` if unset.
    pub fn name_str<'a>(&self, symbols: &'a Symbols) -> &'a str {
        self.name.map(|s| symbols.resolve(s)).unwrap_or("")
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
pub fn interpolate_message(template: &str, node: &PipelineNode, symbols: &Symbols) -> String {
    let mut result = template.to_string();
    result = result.replace("{{name}}", node.name_str(symbols));
    result = result.replace("{{kind}}", &node.kind);
    result = result.replace("{{file}}", node.file_path_str(symbols));
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

    if !node.arg_literals.is_empty() {
        result = result.replace("{{arg_literals}}", &node.arg_literals.join(", "));
    }
    if let Some(ref test_name) = node.enclosing_test_name {
        result = result.replace("{{enclosing_test_name}}", test_name);
    }

    for (name_spur, val_spur) in &node.captures {
        let placeholder = format!("{{{{@{}}}}}", symbols.resolve(*name_spur));
        result = result.replace(&placeholder, symbols.resolve(*val_spur));
    }
    result
}
