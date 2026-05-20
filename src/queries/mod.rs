//! User-facing Cozoscript query surface (issue 05).
//!
//! Three entry points (mutually exclusive at the CLI):
//!   - `--cozoscript '<inline>'`
//!   - `--file <path.cozoql>`
//!   - `--template <name>` (plus `--param k=v`)
//!
//! See [`runner::run`] for the unified entry point; [`templates`] for the
//! embedded built-ins; [`rust_templates`] for the three handlers that
//! cannot be expressed as pure Cozoscript (complexity_hotspots,
//! taint_paths, unreleased_resources).

pub mod runner;
pub mod rust_templates;
pub mod templates;

pub use runner::{QueryRequest, QuerySource, run};
