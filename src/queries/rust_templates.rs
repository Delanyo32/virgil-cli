//! Rust-side `--template` handlers.
//!
//! Phase 1 of the Datalog-model migration: handlers bound to the previous
//! schema shape (`*symbol{start_line, ...}` etc.) are stubbed out per
//! [ADR-0004]. They get rebuilt against the new schema in Phase 7
//! (issue #17), which can take advantage of the new `span` / `references`
//! / `*_attrs` relations.
//!
//! [ADR-0004]: docs/adr/0004-templates-dark-during-migration.md

use std::collections::BTreeMap;

use anyhow::Result;

use crate::cozo::CozoStore;
use crate::storage::workspace::Workspace;

use super::runner::QueryOutput;

pub struct Context<'a> {
    pub store: &'a CozoStore,
    pub workspace: &'a Workspace,
    pub params: &'a BTreeMap<String, String>,
}

pub type Handler = fn(&Context<'_>) -> Result<QueryOutput>;

/// No Rust-side template handlers are registered during the migration.
/// Templates are rebuilt against the new schema in issue #17.
pub fn lookup(_name: &str) -> Option<Handler> {
    None
}

pub fn names() -> &'static [&'static str] {
    &[]
}
