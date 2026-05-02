//! Pure-data input/output schema for taint analysis.

use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSourcePattern {
    pub pattern: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSinkPattern {
    pub pattern: String,
    pub vulnerability: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintSanitizerPattern {
    pub pattern: String,
}

/// Dynamic taint configuration — sources, sinks, and sanitizers come from JSON pipeline files.
pub struct TaintConfig {
    pub sources: Vec<TaintSourcePattern>,
    pub sinks: Vec<TaintSinkPattern>,
    pub sanitizers: Vec<TaintSanitizerPattern>,
}

/// A single taint finding: unsanitized data flowing from source to sink.
#[derive(Debug, Clone)]
pub struct TaintFinding {
    /// The function graph node where the finding was detected.
    pub function_node: NodeIndex,
    /// Human-readable name of the function.
    pub function_name: String,
    /// File path containing the function.
    pub file_path: String,
    /// The variable that carried taint into the sink.
    pub tainted_var: String,
    /// The sink call name.
    pub sink_name: String,
    /// Line of the sink call.
    pub sink_line: u32,
    /// How the variable became tainted (source description).
    pub source_description: String,
    /// Line where taint originated (if known).
    pub source_line: Option<u32>,
}
