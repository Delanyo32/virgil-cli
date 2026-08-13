pub mod builder;

/// A node in the import resolution result. Most languages resolve to a file;
/// Go resolves to a package directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GraphNode {
    File(String),
    Package(String),
}
