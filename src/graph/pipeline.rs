use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// NodeType — what kind of graph node to select
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    File,
    Symbol,
    CallSite,
}

// ---------------------------------------------------------------------------
// EdgeType — which graph edge to traverse or count
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// EdgeDirection — traversal direction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDirection {
    In,
    Out,
    Both,
}

impl Default for EdgeDirection {
    fn default() -> Self {
        Self::Out
    }
}

// ---------------------------------------------------------------------------
// NumericPredicate — numeric threshold check
// ---------------------------------------------------------------------------

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
        if let Some(v) = self.gte {
            if value < v {
                return false;
            }
        }
        if let Some(v) = self.lte {
            if value > v {
                return false;
            }
        }
        if let Some(v) = self.gt {
            if value <= v {
                return false;
            }
        }
        if let Some(v) = self.lt {
            if value >= v {
                return false;
            }
        }
        if let Some(v) = self.eq {
            if (value - v).abs() > f64::EPSILON {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// WhereClause — composable predicate system
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WhereClause {
    // Logical operators
    #[serde(default)]
    pub and: Option<Vec<WhereClause>>,
    #[serde(default)]
    pub or: Option<Vec<WhereClause>>,
    #[serde(default)]
    pub not: Option<Box<WhereClause>>,

    // Semantic built-ins
    #[serde(default)]
    pub is_test_file: Option<bool>,
    #[serde(default)]
    pub is_generated: Option<bool>,
    #[serde(default)]
    pub is_barrel_file: Option<bool>,
    /// NOTE: not evaluated by WhereClause::eval() — the executor checks NOLINT
    /// suppression separately. This field is reserved for future executor integration.
    #[serde(default)]
    pub is_nolint: Option<bool>,

    // Node property predicates
    #[serde(default)]
    pub exported: Option<bool>,

    // Metric predicates (for severity_map "when" clauses)
    #[serde(default)]
    pub count: Option<NumericPredicate>,
    #[serde(default)]
    pub cycle_size: Option<NumericPredicate>,
    #[serde(default)]
    pub depth: Option<NumericPredicate>,
    #[serde(default)]
    pub edge_count: Option<NumericPredicate>,
    #[serde(default)]
    pub ratio: Option<NumericPredicate>,

    // Symbol kind filter (for select stage kind filtering per D-03)
    #[serde(default)]
    pub kind: Option<Vec<String>>,

    // Compute-metric predicates (for severity_map when clauses)
    #[serde(default)]
    pub cyclomatic_complexity: Option<NumericPredicate>,
    #[serde(default)]
    pub function_length: Option<NumericPredicate>,
    #[serde(default)]
    pub cognitive_complexity: Option<NumericPredicate>,
    #[serde(default)]
    pub comment_to_code_ratio: Option<NumericPredicate>,
}

impl WhereClause {
    /// Returns true if no conditions are set (empty predicate = always true)
    pub fn is_empty(&self) -> bool {
        self.and.is_none()
            && self.or.is_none()
            && self.not.is_none()
            && self.is_test_file.is_none()
            && self.is_generated.is_none()
            && self.is_barrel_file.is_none()
            && self.is_nolint.is_none()
            && self.exported.is_none()
            && self.count.is_none()
            && self.cycle_size.is_none()
            && self.depth.is_none()
            && self.edge_count.is_none()
            && self.ratio.is_none()
            && self.kind.is_none()
            && self.cyclomatic_complexity.is_none()
            && self.function_length.is_none()
            && self.cognitive_complexity.is_none()
            && self.comment_to_code_ratio.is_none()
    }

    /// Evaluate predicate against a node's metrics only (no file system access).
    /// Used in severity_map `when` evaluation.
    pub fn eval_metrics(&self, node: &PipelineNode) -> bool {
        // Logical operators
        if let Some(ref clauses) = self.and {
            if !clauses.iter().all(|c| c.eval_metrics(node)) {
                return false;
            }
        }
        if let Some(ref clauses) = self.or {
            if !clauses.is_empty() && !clauses.iter().any(|c| c.eval_metrics(node)) {
                return false;
            }
        }
        if let Some(ref clause) = self.not {
            if clause.eval_metrics(node) {
                return false;
            }
        }
        // Property predicates
        if let Some(exp) = self.exported {
            if node.exported != exp {
                return false;
            }
        }
        // Metric predicates
        if let Some(ref pred) = self.count {
            if !pred.matches(node.metric_f64("count")) {
                return false;
            }
        }
        if let Some(ref pred) = self.cycle_size {
            if !pred.matches(node.metric_f64("cycle_size")) {
                return false;
            }
        }
        if let Some(ref pred) = self.depth {
            if !pred.matches(node.metric_f64("depth")) {
                return false;
            }
        }
        if let Some(ref pred) = self.edge_count {
            if !pred.matches(node.metric_f64("edge_count")) {
                return false;
            }
        }
        if let Some(ref pred) = self.ratio {
            if !pred.matches(node.metric_f64("ratio")) {
                return false;
            }
        }
        if let Some(ref pred) = self.cyclomatic_complexity {
            if !pred.matches(node.metric_f64("cyclomatic_complexity")) {
                return false;
            }
        }
        if let Some(ref pred) = self.function_length {
            if !pred.matches(node.metric_f64("function_length")) {
                return false;
            }
        }
        if let Some(ref pred) = self.cognitive_complexity {
            if !pred.matches(node.metric_f64("cognitive_complexity")) {
                return false;
            }
        }
        if let Some(ref pred) = self.comment_to_code_ratio {
            if !pred.matches(node.metric_f64("comment_to_code_ratio")) {
                return false;
            }
        }
        // Semantic built-ins not evaluatable from metrics alone — skip here
        // (they are evaluated in executor using file path helpers)
        true
    }

    /// Evaluate predicate against a node including file-system semantic checks.
    /// `is_test_file_fn`, `is_generated_fn`, `is_barrel_file_fn` are function pointers
    /// to helper functions from audit/pipelines/helpers.rs.
    pub fn eval(
        &self,
        node: &PipelineNode,
        is_test_fn: &impl Fn(&str) -> bool,
        is_generated_fn: &impl Fn(&str) -> bool,
        is_barrel_fn: &impl Fn(&str) -> bool,
    ) -> bool {
        if let Some(ref clauses) = self.and {
            if !clauses
                .iter()
                .all(|c| c.eval(node, is_test_fn, is_generated_fn, is_barrel_fn))
            {
                return false;
            }
        }
        if let Some(ref clauses) = self.or {
            if !clauses.is_empty()
                && !clauses
                    .iter()
                    .any(|c| c.eval(node, is_test_fn, is_generated_fn, is_barrel_fn))
            {
                return false;
            }
        }
        if let Some(ref clause) = self.not {
            if clause.eval(node, is_test_fn, is_generated_fn, is_barrel_fn) {
                return false;
            }
        }
        if let Some(v) = self.is_test_file {
            if is_test_fn(&node.file_path) != v {
                return false;
            }
        }
        if let Some(v) = self.is_generated {
            if is_generated_fn(&node.file_path) != v {
                return false;
            }
        }
        if let Some(v) = self.is_barrel_file {
            if is_barrel_fn(&node.file_path) != v {
                return false;
            }
        }
        // is_nolint: requires source-level comment scanning — not evaluatable from
        // file path alone. Implemented in executor (Task 2) via a separate nolint check.
        // For now, is_nolint predicate in WhereClause is a no-op in eval().
        if let Some(exp) = self.exported {
            if node.exported != exp {
                return false;
            }
        }
        if let Some(ref kinds) = self.kind {
            if !kinds.iter().any(|k| k.eq_ignore_ascii_case(&node.kind)) {
                return false;
            }
        }
        if let Some(ref pred) = self.count {
            if !pred.matches(node.metric_f64("count")) {
                return false;
            }
        }
        if let Some(ref pred) = self.cycle_size {
            if !pred.matches(node.metric_f64("cycle_size")) {
                return false;
            }
        }
        if let Some(ref pred) = self.depth {
            if !pred.matches(node.metric_f64("depth")) {
                return false;
            }
        }
        if let Some(ref pred) = self.edge_count {
            if !pred.matches(node.metric_f64("edge_count")) {
                return false;
            }
        }
        if let Some(ref pred) = self.ratio {
            if !pred.matches(node.metric_f64("ratio")) {
                return false;
            }
        }
        if let Some(ref pred) = self.cyclomatic_complexity {
            if !pred.matches(node.metric_f64("cyclomatic_complexity")) {
                return false;
            }
        }
        if let Some(ref pred) = self.function_length {
            if !pred.matches(node.metric_f64("function_length")) {
                return false;
            }
        }
        if let Some(ref pred) = self.cognitive_complexity {
            if !pred.matches(node.metric_f64("cognitive_complexity")) {
                return false;
            }
        }
        if let Some(ref pred) = self.comment_to_code_ratio {
            if !pred.matches(node.metric_f64("comment_to_code_ratio")) {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// MetricValue — runtime metric values stored per node
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// PipelineNode — execution unit passed between stages
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// SeverityEntry — one entry in a severity_map
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeverityEntry {
    /// Condition to match. If None or empty, acts as default (always matches).
    #[serde(default)]
    pub when: Option<WhereClause>,
    pub severity: String,
}

// ---------------------------------------------------------------------------
// Per-stage config structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountConfig {
    pub threshold: NumericPredicate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaxDepthConfig {
    pub edge: EdgeType,
    #[serde(default)]
    pub skip_barrel_files: Option<bool>,
    pub threshold: NumericPredicate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindCyclesConfig {
    pub edge: EdgeType,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NumeratorConfig {
    #[serde(default, rename = "where")]
    pub filter: Option<WhereClause>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DenominatorConfig {
    #[serde(default, rename = "where")]
    pub filter: Option<WhereClause>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatioConfig {
    pub numerator: NumeratorConfig,
    pub denominator: DenominatorConfig,
    #[serde(default)]
    pub threshold: Option<WhereClause>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagConfig {
    pub pattern: String,
    pub message: String,
    /// Default severity (used when no severity_map matches)
    #[serde(default)]
    pub severity: Option<String>,
    /// Ordered list of conditional severities. First matching `when` wins.
    /// An entry with no `when` (or empty `when`) acts as the catch-all default.
    #[serde(default)]
    pub severity_map: Option<Vec<SeverityEntry>>,
    /// Optional pipeline name to tag findings with (overrides JsonAuditFile.pipeline)
    #[serde(default)]
    pub pipeline_name: Option<String>,
}

impl FlagConfig {
    /// Resolve the effective severity given the current node's metrics.
    /// Checks severity_map entries in order; first matching `when` wins.
    /// Falls back to `severity` field or "warning" if nothing matches.
    pub fn resolve_severity(&self, node: &PipelineNode) -> String {
        if let Some(ref map) = self.severity_map {
            for entry in map {
                let matches = match &entry.when {
                    None => true, // no condition = default
                    Some(wc) => wc.is_empty() || wc.eval_metrics(node),
                };
                if matches {
                    return entry.severity.clone();
                }
            }
        }
        self.severity.clone().unwrap_or_else(|| "warning".to_string())
    }
}

// ---------------------------------------------------------------------------
// GraphStage — the top-level enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GraphStage {
    Select {
        select: NodeType,
        #[serde(default, rename = "where")]
        filter: Option<WhereClause>,
        #[serde(default)]
        exclude: Option<WhereClause>,
    },
    GroupBy {
        group_by: String,
    },
    Count {
        count: CountConfig,
    },
    MaxDepth {
        max_depth: MaxDepthConfig,
    },
    FindCycles {
        find_cycles: FindCyclesConfig,
    },
    Ratio {
        ratio: RatioConfig,
    },
    MatchPattern {
        match_pattern: String,
    },
    ComputeMetric {
        compute_metric: String,
    },
    Flag {
        flag: FlagConfig,
    },
}

// ---------------------------------------------------------------------------
// Template interpolation helper
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(metrics: Vec<(&str, MetricValue)>) -> PipelineNode {
        let mut m = HashMap::new();
        for (k, v) in metrics {
            m.insert(k.to_string(), v);
        }
        PipelineNode {
            node_idx: NodeIndex::new(0),
            file_path: "src/foo.rs".to_string(),
            name: "my_func".to_string(),
            kind: "function".to_string(),
            line: 42,
            exported: true,
            language: "rust".to_string(),
            metrics: m,
        }
    }

    // -----------------------------------------------------------------------
    // NumericPredicate::matches
    // -----------------------------------------------------------------------

    #[test]
    fn test_numeric_predicate_gte() {
        let pred = NumericPredicate { gte: Some(5.0), ..Default::default() };
        assert!(pred.matches(5.0));
        assert!(pred.matches(10.0));
        assert!(!pred.matches(4.9));
    }

    #[test]
    fn test_numeric_predicate_lte() {
        let pred = NumericPredicate { lte: Some(10.0), ..Default::default() };
        assert!(pred.matches(10.0));
        assert!(pred.matches(0.0));
        assert!(!pred.matches(10.1));
    }

    #[test]
    fn test_numeric_predicate_gt() {
        let pred = NumericPredicate { gt: Some(5.0), ..Default::default() };
        assert!(pred.matches(5.1));
        assert!(!pred.matches(5.0));
        assert!(!pred.matches(4.0));
    }

    #[test]
    fn test_numeric_predicate_lt() {
        let pred = NumericPredicate { lt: Some(5.0), ..Default::default() };
        assert!(pred.matches(4.9));
        assert!(!pred.matches(5.0));
        assert!(!pred.matches(6.0));
    }

    #[test]
    fn test_numeric_predicate_eq() {
        let pred = NumericPredicate { eq: Some(7.0), ..Default::default() };
        assert!(pred.matches(7.0));
        assert!(!pred.matches(7.1));
        assert!(!pred.matches(6.9));
    }

    #[test]
    fn test_numeric_predicate_compound_range() {
        // 5 <= value <= 10
        let pred = NumericPredicate {
            gte: Some(5.0),
            lte: Some(10.0),
            ..Default::default()
        };
        assert!(pred.matches(5.0));
        assert!(pred.matches(7.5));
        assert!(pred.matches(10.0));
        assert!(!pred.matches(4.9));
        assert!(!pred.matches(10.1));
    }

    #[test]
    fn test_numeric_predicate_empty_always_true() {
        let pred = NumericPredicate::default();
        assert!(pred.matches(0.0));
        assert!(pred.matches(f64::MAX));
        assert!(pred.matches(f64::MIN));
    }

    // -----------------------------------------------------------------------
    // WhereClause::is_empty
    // -----------------------------------------------------------------------

    #[test]
    fn test_where_clause_is_empty_default() {
        let wc = WhereClause::default();
        assert!(wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_exported() {
        let wc = WhereClause { exported: Some(true), ..Default::default() };
        assert!(!wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_count() {
        let wc = WhereClause {
            count: Some(NumericPredicate { gte: Some(1.0), ..Default::default() }),
            ..Default::default()
        };
        assert!(!wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_is_test_file() {
        let wc = WhereClause { is_test_file: Some(false), ..Default::default() };
        assert!(!wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_and() {
        let wc = WhereClause {
            and: Some(vec![WhereClause::default()]),
            ..Default::default()
        };
        assert!(!wc.is_empty());
    }

    // -----------------------------------------------------------------------
    // WhereClause::eval_metrics
    // -----------------------------------------------------------------------

    #[test]
    fn test_eval_metrics_count_threshold() {
        let node = make_node(vec![("count", MetricValue::Int(15))]);

        let wc_pass = WhereClause {
            count: Some(NumericPredicate { gte: Some(10.0), ..Default::default() }),
            ..Default::default()
        };
        assert!(wc_pass.eval_metrics(&node));

        let wc_fail = WhereClause {
            count: Some(NumericPredicate { gte: Some(20.0), ..Default::default() }),
            ..Default::default()
        };
        assert!(!wc_fail.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_cycle_size() {
        let node = make_node(vec![("cycle_size", MetricValue::Int(3))]);

        let wc = WhereClause {
            cycle_size: Some(NumericPredicate { gte: Some(3.0), ..Default::default() }),
            ..Default::default()
        };
        assert!(wc.eval_metrics(&node));

        let wc_fail = WhereClause {
            cycle_size: Some(NumericPredicate { gt: Some(3.0), ..Default::default() }),
            ..Default::default()
        };
        assert!(!wc_fail.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_exported() {
        let node_exported = make_node(vec![]);
        assert!(node_exported.exported);

        let wc_exported = WhereClause { exported: Some(true), ..Default::default() };
        assert!(wc_exported.eval_metrics(&node_exported));

        let wc_not_exported = WhereClause { exported: Some(false), ..Default::default() };
        assert!(!wc_not_exported.eval_metrics(&node_exported));
    }

    #[test]
    fn test_eval_metrics_and_operator() {
        let node = make_node(vec![
            ("count", MetricValue::Int(10)),
            ("depth", MetricValue::Int(5)),
        ]);

        let wc = WhereClause {
            and: Some(vec![
                WhereClause {
                    count: Some(NumericPredicate { gte: Some(5.0), ..Default::default() }),
                    ..Default::default()
                },
                WhereClause {
                    depth: Some(NumericPredicate { lte: Some(10.0), ..Default::default() }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        assert!(wc.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_or_operator() {
        let node = make_node(vec![("count", MetricValue::Int(2))]);

        let wc = WhereClause {
            or: Some(vec![
                WhereClause {
                    count: Some(NumericPredicate { gte: Some(100.0), ..Default::default() }),
                    ..Default::default()
                },
                WhereClause {
                    count: Some(NumericPredicate { lte: Some(5.0), ..Default::default() }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        assert!(wc.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_not_operator() {
        let node = make_node(vec![("count", MetricValue::Int(2))]);

        let wc = WhereClause {
            not: Some(Box::new(WhereClause {
                count: Some(NumericPredicate { gte: Some(100.0), ..Default::default() }),
                ..Default::default()
            })),
            ..Default::default()
        };
        // count=2, NOT(count >= 100) => true
        assert!(wc.eval_metrics(&node));
    }

    // -----------------------------------------------------------------------
    // FlagConfig::resolve_severity
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_severity_fallback_default() {
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: None,
            severity_map: None,
            pipeline_name: None,
        };
        let node = make_node(vec![]);
        assert_eq!(flag.resolve_severity(&node), "warning");
    }

    #[test]
    fn test_resolve_severity_uses_severity_field() {
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("error".to_string()),
            severity_map: None,
            pipeline_name: None,
        };
        let node = make_node(vec![]);
        assert_eq!(flag.resolve_severity(&node), "error");
    }

    #[test]
    fn test_resolve_severity_map_first_match_wins() {
        let node = make_node(vec![("count", MetricValue::Int(25))]);

        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![
                SeverityEntry {
                    when: Some(WhereClause {
                        count: Some(NumericPredicate { gte: Some(20.0), ..Default::default() }),
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                SeverityEntry {
                    when: Some(WhereClause {
                        count: Some(NumericPredicate { gte: Some(10.0), ..Default::default() }),
                        ..Default::default()
                    }),
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };
        // count=25 >= 20 matches first entry
        assert_eq!(flag.resolve_severity(&node), "error");
    }

    #[test]
    fn test_resolve_severity_map_fallthrough_to_second() {
        let node = make_node(vec![("count", MetricValue::Int(12))]);

        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![
                SeverityEntry {
                    when: Some(WhereClause {
                        count: Some(NumericPredicate { gte: Some(20.0), ..Default::default() }),
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                SeverityEntry {
                    when: Some(WhereClause {
                        count: Some(NumericPredicate { gte: Some(10.0), ..Default::default() }),
                        ..Default::default()
                    }),
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };
        // count=12 does NOT match first (>= 20), does match second (>= 10)
        assert_eq!(flag.resolve_severity(&node), "warning");
    }

    #[test]
    fn test_resolve_severity_map_none_when_is_default() {
        // An entry with when=None acts as catch-all
        let node = make_node(vec![("count", MetricValue::Int(0))]);
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![
                SeverityEntry {
                    when: Some(WhereClause {
                        count: Some(NumericPredicate { gte: Some(100.0), ..Default::default() }),
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                SeverityEntry {
                    when: None, // catch-all
                    severity: "hint".to_string(),
                },
            ]),
            pipeline_name: None,
        };
        // count=0 does not match first, second has no condition => "hint"
        assert_eq!(flag.resolve_severity(&node), "hint");
    }

    #[test]
    fn test_resolve_severity_map_no_match_falls_back_to_severity() {
        let node = make_node(vec![("count", MetricValue::Int(0))]);
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![SeverityEntry {
                when: Some(WhereClause {
                    count: Some(NumericPredicate { gte: Some(100.0), ..Default::default() }),
                    ..Default::default()
                }),
                severity: "error".to_string(),
            }]),
            pipeline_name: None,
        };
        // Nothing matches, falls back to severity field
        assert_eq!(flag.resolve_severity(&node), "info");
    }

    // -----------------------------------------------------------------------
    // interpolate_message
    // -----------------------------------------------------------------------

    #[test]
    fn test_interpolate_basic_fields() {
        let node = make_node(vec![]);
        let result = interpolate_message("{{name}} at {{file}}:{{line}}", &node);
        assert_eq!(result, "my_func at src/foo.rs:42");
    }

    #[test]
    fn test_interpolate_kind_and_language() {
        let node = make_node(vec![]);
        let result = interpolate_message("{{kind}} in {{language}}", &node);
        assert_eq!(result, "function in rust");
    }

    #[test]
    fn test_interpolate_metric_int() {
        let node = make_node(vec![("count", MetricValue::Int(7))]);
        let result = interpolate_message("count={{count}}", &node);
        assert_eq!(result, "count=7");
    }

    #[test]
    fn test_interpolate_metric_float() {
        let node = make_node(vec![("ratio", MetricValue::Float(0.75))]);
        let result = interpolate_message("ratio={{ratio}}", &node);
        assert_eq!(result, "ratio=0.75");
    }

    #[test]
    fn test_interpolate_metric_text() {
        let node = make_node(vec![("cycle_path", MetricValue::Text("a->b->a".to_string()))]);
        let result = interpolate_message("path={{cycle_path}}", &node);
        assert_eq!(result, "path=a->b->a");
    }

    #[test]
    fn test_interpolate_no_placeholders() {
        let node = make_node(vec![]);
        let result = interpolate_message("no placeholders here", &node);
        assert_eq!(result, "no placeholders here");
    }

    // -----------------------------------------------------------------------
    // GraphStage deserialization (untagged enum)
    // -----------------------------------------------------------------------

    #[test]
    fn test_deserialize_select_stage() {
        let json = r#"{"select": "symbol"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Select { select, filter, exclude } => {
                assert_eq!(select, NodeType::Symbol);
                assert!(filter.is_none());
                assert!(exclude.is_none());
            }
            _ => panic!("expected Select stage"),
        }
    }

    #[test]
    fn test_deserialize_select_with_where() {
        let json = r#"{"select": "file", "where": {"exported": true}}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Select { select, filter, .. } => {
                assert_eq!(select, NodeType::File);
                let wc = filter.unwrap();
                assert_eq!(wc.exported, Some(true));
            }
            _ => panic!("expected Select stage"),
        }
    }

    #[test]
    fn test_deserialize_find_cycles_stage() {
        let json = r#"{"find_cycles": {"edge": "calls"}}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::FindCycles { find_cycles } => {
                assert_eq!(find_cycles.edge, EdgeType::Calls);
            }
            _ => panic!("expected FindCycles stage"),
        }
    }

    #[test]
    fn test_deserialize_flag_stage() {
        let json = r#"{
            "flag": {
                "pattern": "oversized_module",
                "message": "{{name}} has {{count}} symbols",
                "severity": "warning"
            }
        }"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Flag { flag } => {
                assert_eq!(flag.pattern, "oversized_module");
                assert_eq!(flag.severity, Some("warning".to_string()));
                assert!(flag.severity_map.is_none());
            }
            _ => panic!("expected Flag stage"),
        }
    }

    #[test]
    fn test_deserialize_group_by_stage() {
        let json = r#"{"group_by": "file_path"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::GroupBy { group_by } => {
                assert_eq!(group_by, "file_path");
            }
            _ => panic!("expected GroupBy stage"),
        }
    }

    #[test]
    fn test_deserialize_match_pattern_stage() {
        let json = r#"{"match_pattern": "(identifier) @name"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::MatchPattern { match_pattern } => {
                assert_eq!(match_pattern, "(identifier) @name");
            }
            _ => panic!("expected MatchPattern stage"),
        }
    }

    #[test]
    fn test_deserialize_compute_metric_stage() {
        let json = r#"{"compute_metric": "cyclomatic_complexity"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::ComputeMetric { compute_metric } => {
                assert_eq!(compute_metric, "cyclomatic_complexity");
            }
            _ => panic!("expected ComputeMetric stage"),
        }
    }

    #[test]
    fn test_deserialize_flag_with_severity_map() {
        let json = r#"{
            "flag": {
                "pattern": "hub_module",
                "message": "Hub module {{name}}",
                "severity_map": [
                    {"when": {"count": {"gte": 20}}, "severity": "error"},
                    {"when": {"count": {"gte": 10}}, "severity": "warning"},
                    {"severity": "info"}
                ]
            }
        }"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Flag { flag } => {
                let map = flag.severity_map.unwrap();
                assert_eq!(map.len(), 3);
                assert_eq!(map[0].severity, "error");
                assert_eq!(map[1].severity, "warning");
                assert_eq!(map[2].severity, "info");
                assert!(map[2].when.is_none());
            }
            _ => panic!("expected Flag stage"),
        }
    }

    #[test]
    fn test_node_type_roundtrip() {
        let variants = vec![NodeType::File, NodeType::Symbol, NodeType::CallSite];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let back: NodeType = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn test_edge_type_roundtrip() {
        let variants = vec![
            EdgeType::Calls,
            EdgeType::Imports,
            EdgeType::FlowsTo,
            EdgeType::Acquires,
            EdgeType::ReleasedBy,
            EdgeType::Contains,
            EdgeType::Exports,
            EdgeType::DefinedIn,
        ];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let back: EdgeType = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn test_edge_direction_default_is_out() {
        assert_eq!(EdgeDirection::default(), EdgeDirection::Out);
    }

    #[test]
    fn where_clause_metric_predicates_cyclomatic() {
        let json = r#"{"cyclomatic_complexity": {"gt": 10}}"#;
        let wc: WhereClause = serde_json::from_str(json).unwrap();
        assert!(wc.cyclomatic_complexity.is_some());

        let mut node = PipelineNode {
            node_idx: petgraph::graph::NodeIndex::new(0),
            file_path: "test.rs".to_string(),
            name: "foo".to_string(),
            kind: "function".to_string(),
            line: 1,
            exported: false,
            language: "rust".to_string(),
            metrics: std::collections::HashMap::new(),
        };
        // CC = 15 > 10 -- should pass
        node.metrics.insert("cyclomatic_complexity".to_string(), MetricValue::Int(15));
        assert!(wc.eval_metrics(&node));

        // CC = 5 <= 10 -- should fail
        node.metrics.insert("cyclomatic_complexity".to_string(), MetricValue::Int(5));
        assert!(!wc.eval_metrics(&node));
    }

    #[test]
    fn where_clause_kind_filter() {
        let json = r#"{"kind": ["function", "method"]}"#;
        let wc: WhereClause = serde_json::from_str(json).unwrap();
        assert!(wc.kind.is_some());

        let node_fn = PipelineNode {
            node_idx: petgraph::graph::NodeIndex::new(0),
            file_path: "test.rs".to_string(),
            name: "foo".to_string(),
            kind: "function".to_string(),
            line: 1,
            exported: false,
            language: "rust".to_string(),
            metrics: std::collections::HashMap::new(),
        };
        let is_test = |_: &str| false;
        let is_gen = |_: &str| false;
        let is_barrel = |_: &str| false;
        assert!(wc.eval(&node_fn, &is_test, &is_gen, &is_barrel));

        let node_class = PipelineNode {
            node_idx: petgraph::graph::NodeIndex::new(0),
            file_path: "test.rs".to_string(),
            name: "Foo".to_string(),
            kind: "class".to_string(),
            line: 1,
            exported: false,
            language: "rust".to_string(),
            metrics: std::collections::HashMap::new(),
        };
        assert!(!wc.eval(&node_class, &is_test, &is_gen, &is_barrel));
    }
}
