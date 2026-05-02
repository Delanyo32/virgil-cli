//! JSON pipeline layer: DSL, execution engine, and audit file loading.

pub mod dsl;
pub mod executor;
pub mod helpers;
pub mod loader;
pub mod node_helpers;
pub mod output;
pub mod stages;

pub use dsl::{
    EdgeDirection, EdgeType, FindDuplicatesStage, FlagConfig, GraphStage, MetricValue, NodeType,
    NumericPredicate, PipelineNode, SeverityEntry, TaintSanitizerPattern, TaintSinkPattern,
    TaintSourcePattern, TaintStage, WhereClause, interpolate_message,
};
pub use executor::{PipelineOutput, run_pipeline};
pub use loader::{JsonAuditFile, load_json_audits};
