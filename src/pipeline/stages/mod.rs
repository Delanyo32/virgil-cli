//! Stage executors. Each stage family lives in its own submodule.
//! `executor::execute_stage` dispatches to these functions.

pub(crate) mod aggregate;
pub(crate) mod compute_metric;
pub(crate) mod cycles;
pub(crate) mod find_duplicates;
pub(crate) mod match_pattern;
pub(crate) mod select;
pub(crate) mod taint;
