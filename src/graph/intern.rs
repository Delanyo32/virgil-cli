//! String interning for the code graph.
//!
//! The graph stores millions of repeated strings — file paths, symbol names,
//! callee names — and naive `String` storage dominates resident memory on
//! large corpora. This module wraps a single shared `lasso::ThreadedRodeo`
//! behind a clonable `Symbols` handle. Every `Spur` (4-byte interner key)
//! resolves back to a `&str` borrowing from the rodeo.

use std::sync::Arc;

pub use lasso::Spur;
use lasso::ThreadedRodeo;

/// Shared interner. Cheap to clone (just bumps the `Arc`).
#[derive(Clone)]
pub struct Symbols {
    inner: Arc<ThreadedRodeo>,
}

impl Symbols {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ThreadedRodeo::new()),
        }
    }

    pub fn intern(&self, s: &str) -> Spur {
        self.inner.get_or_intern(s)
    }

    pub fn intern_static(&self, s: &'static str) -> Spur {
        self.inner.get_or_intern_static(s)
    }

    /// Resolve a `Spur` to the underlying string slice.
    pub fn resolve(&self, spur: Spur) -> &str {
        self.inner.resolve(&spur)
    }

    /// Look up an already-interned string without inserting. Returns `None`
    /// when the string has never been seen. Useful for "does this name exist
    /// at all" checks that should not pollute the interner with hot lookups.
    pub fn get(&self, s: &str) -> Option<Spur> {
        self.inner.get(s)
    }
}

impl Default for Symbols {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Symbols {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Symbols")
            .field("len", &self.inner.len())
            .finish()
    }
}
