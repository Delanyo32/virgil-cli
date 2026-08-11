//! User-facing SQL query surface. Three mutually exclusive CLI entry
//! points: `--sql '<inline>'`, `--file <path.sql>`, `--template <name>`
//! (plus `--param k=v`). See [`runner::run`] for the dispatcher,
//! [`templates`] for the embedded SQL built-ins, and [`rust_templates`]
//! for `complexity_hotspots`, the one handler that needs source access.

pub mod runner;
pub mod rust_templates;
pub mod templates;

pub use runner::{QueryRequest, QuerySource, run};
