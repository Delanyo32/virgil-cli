//! Per-stage config structs and the top-level `GraphStage` enum.

use serde::{Deserialize, Serialize};

use crate::graph::taint::{TaintSanitizerPattern, TaintSinkPattern, TaintSourcePattern};

use super::{
    EdgeType, NodeType, NumericPredicate, PipelineNode, SeverityEntry, WhereClause,
};

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
pub struct TaintStage {
    pub sources: Vec<TaintSourcePattern>,
    pub sinks: Vec<TaintSinkPattern>,
    #[serde(default)]
    pub sanitizers: Vec<TaintSanitizerPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindDuplicatesStage {
    pub by: String,
    #[serde(default = "default_min_count")]
    pub min_count: usize,
}

fn default_min_count() -> usize {
    2
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
    /// Falls back to `severity` field if no severity_map entry matches.
    /// Returns None (suppresses finding) when severity_map has only conditional
    /// entries, none match, and no bare `severity` field exists.
    pub fn resolve_severity(&self, node: &PipelineNode) -> Option<String> {
        if let Some(ref map) = self.severity_map {
            for entry in map {
                let matches = match &entry.when {
                    None => true,
                    Some(wc) => wc.is_empty() || wc.eval_metrics(node),
                };
                if matches {
                    return Some(entry.severity.clone());
                }
            }
            self.severity.clone()
        } else {
            Some(
                self.severity
                    .clone()
                    .unwrap_or_else(|| "warning".to_string()),
            )
        }
    }
}

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
        /// Optional post-filter on each match result.
        #[serde(default)]
        when: Option<WhereClause>,
    },
    ComputeMetric {
        compute_metric: String,
    },
    Flag {
        flag: FlagConfig,
    },
    Taint {
        taint: TaintStage,
    },
    TaintSources {
        taint_sources: Vec<TaintSourcePattern>,
    },
    TaintSanitizers {
        taint_sanitizers: Vec<TaintSanitizerPattern>,
    },
    TaintSinks {
        taint_sinks: Vec<TaintSinkPattern>,
    },
    FindDuplicates {
        find_duplicates: FindDuplicatesStage,
    },
}
