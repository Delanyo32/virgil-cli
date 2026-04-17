//! JSON pipeline layer: DSL, execution engine, and audit file loading.

pub mod dsl;
pub mod executor;
pub mod helpers;
pub mod loader;

pub use dsl::{
    EdgeDirection, EdgeType, FlagConfig, FindDuplicatesStage, GraphStage, MetricValue,
    NodeType, NumericPredicate, PipelineNode, SeverityEntry, TaintSanitizerPattern,
    TaintSinkPattern, TaintSourcePattern, TaintStage, WhereClause, interpolate_message,
};
pub use executor::{PipelineOutput, run_pipeline};
pub use loader::{JsonAuditFile, discover_json_audits};
