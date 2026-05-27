pub mod builder;
pub mod intern;
pub mod metrics;

pub use intern::{Spur, Symbols};

/// A node in the import resolution result. Most languages resolve to a file;
/// Go resolves to a package directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GraphNode {
    File(String),
    Package(String),
}

/// Build-time scratch state. After the SQL-staging refactor, all
/// per-file extractor output is emitted directly to DuckDB during
/// `absorb_file_data` and inheritance is staged in the `raw_inheritance`
/// table for a post-parse SQL resolver. Only the interner survives
/// here — used by the deferred-imports/calls resolution loop and the
/// per-file lookup maps inside the builder.
pub struct CodeGraph {
    pub symbols: Symbols,
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraph {
    pub fn new() -> Self {
        Self::with_symbols(Symbols::new())
    }

    pub fn with_symbols(symbols: Symbols) -> Self {
        Self { symbols }
    }

    /// Reduce step in the parallel builder. The interner is shared via
    /// `Arc`, so absorbing another graph is a no-op — every worker's
    /// `CodeGraph` already points at the same `ThreadedRodeo`.
    pub fn merge(&mut self, _other: CodeGraph) {}
}
