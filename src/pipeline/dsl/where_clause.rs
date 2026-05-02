//! `WhereClause` — composable predicate system for filtering pipeline nodes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{NumericPredicate, PipelineNode};

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

    // Symbol kind filter (for select stage kind filtering per D-03)
    #[serde(default)]
    pub kind: Option<Vec<String>>,

    // Dead-export predicates
    #[serde(default)]
    pub unreferenced: Option<bool>,
    #[serde(default)]
    pub is_entry_point: Option<bool>,

    /// Generic computed-metric predicates. Any metric produced by a `compute_metric` stage
    /// can be filtered without changing the Rust schema.
    #[serde(default)]
    pub metrics: HashMap<String, NumericPredicate>,

    /// When true, the matched node must be an assignment expression whose LHS
    /// member-expression object is a named parameter of the enclosing function.
    /// Evaluated by execute_match_pattern when used in a MatchPattern when clause.
    #[serde(default)]
    pub lhs_is_parameter: Option<bool>,
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
            && self.kind.is_none()
            && self.unreferenced.is_none()
            && self.is_entry_point.is_none()
            && self.metrics.is_empty()
            && self.lhs_is_parameter.is_none()
    }

    /// Evaluate predicate against a node's metrics only (no file system access).
    /// Used in severity_map `when` evaluation.
    pub fn eval_metrics(&self, node: &PipelineNode) -> bool {
        if let Some(ref clauses) = self.and
            && !clauses.iter().all(|c| c.eval_metrics(node))
        {
            return false;
        }
        if let Some(ref clauses) = self.or
            && !clauses.is_empty()
            && !clauses.iter().any(|c| c.eval_metrics(node))
        {
            return false;
        }
        if let Some(ref clause) = self.not
            && clause.eval_metrics(node)
        {
            return false;
        }
        if let Some(exp) = self.exported
            && node.exported != exp
        {
            return false;
        }
        for (metric_name, pred) in &self.metrics {
            if !pred.matches(node.metric_f64(metric_name)) {
                return false;
            }
        }
        // unreferenced and is_entry_point require graph access; skip in metrics-only eval
        // Semantic built-ins not evaluatable from metrics alone — skip here
        true
    }

    /// Evaluate predicate against a node including file-system semantic checks.
    pub fn eval(
        &self,
        node: &PipelineNode,
        is_test_fn: &impl Fn(&str) -> bool,
        is_generated_fn: &impl Fn(&str) -> bool,
        is_barrel_fn: &impl Fn(&str) -> bool,
    ) -> bool {
        if let Some(ref clauses) = self.and
            && !clauses
                .iter()
                .all(|c| c.eval(node, is_test_fn, is_generated_fn, is_barrel_fn))
        {
            return false;
        }
        if let Some(ref clauses) = self.or
            && !clauses.is_empty()
            && !clauses
                .iter()
                .any(|c| c.eval(node, is_test_fn, is_generated_fn, is_barrel_fn))
        {
            return false;
        }
        if let Some(ref clause) = self.not
            && clause.eval(node, is_test_fn, is_generated_fn, is_barrel_fn)
        {
            return false;
        }
        if let Some(v) = self.is_test_file
            && is_test_fn(&node.file_path) != v
        {
            return false;
        }
        if let Some(v) = self.is_generated
            && is_generated_fn(&node.file_path) != v
        {
            return false;
        }
        if let Some(v) = self.is_barrel_file
            && is_barrel_fn(&node.file_path) != v
        {
            return false;
        }
        // is_nolint: requires source-level comment scanning — not evaluatable from
        // file path alone. Implemented in executor via a separate nolint check.
        if let Some(exp) = self.exported
            && node.exported != exp
        {
            return false;
        }
        if let Some(ref kinds) = self.kind
            && !kinds.iter().any(|k| k.eq_ignore_ascii_case(&node.kind))
        {
            return false;
        }
        for (metric_name, pred) in &self.metrics {
            if !pred.matches(node.metric_f64(metric_name)) {
                return false;
            }
        }
        if let Some(exp) = self.unreferenced {
            let val = node.metric_f64("unreferenced") > 0.0;
            if val != exp {
                return false;
            }
        }
        if let Some(exp) = self.is_entry_point {
            let val = node.metric_f64("is_entry_point") > 0.0;
            if val != exp {
                return false;
            }
        }
        true
    }
}
