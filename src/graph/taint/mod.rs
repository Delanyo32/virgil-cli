//! Taint analysis subsystem.
//!
//! - `types` — pure data: pattern types, `TaintConfig` (input), `TaintFinding` (output)
//! - `engine` — `TaintEngine` and its private state, pattern matching, CFG traversal

pub mod engine;
pub mod types;

pub use engine::TaintEngine;
pub use types::{
    TaintConfig, TaintFinding, TaintSanitizerPattern, TaintSinkPattern, TaintSourcePattern,
};
