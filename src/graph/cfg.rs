use petgraph::graph::{DiGraph, NodeIndex};

/// A single statement within a basic block.
#[derive(Debug, Clone)]
pub struct CfgStatement {
    pub kind: CfgStatementKind,
    pub line: u32,
}

/// The kind of statement in the CFG.
#[derive(Debug, Clone)]
pub enum CfgStatementKind {
    /// Variable assignment: `target = expr(source_vars...)`
    Assignment {
        target: String,
        source_vars: Vec<String>,
    },
    /// Function/method call
    Call {
        name: String,
        args: Vec<String>,
    },
    /// Return statement
    Return {
        value_vars: Vec<String>,
    },
    /// Guard/condition check (if condition, match guard, etc.)
    Guard {
        condition_vars: Vec<String>,
    },
    /// Resource acquisition (malloc, fopen, new Stream, etc.)
    ResourceAcquire {
        target: String,
        resource_type: String,
    },
    /// Resource release (free, fclose, .close(), etc.)
    ResourceRelease {
        target: String,
        resource_type: String,
    },
    /// Phi node for merging values at join points
    PhiNode {
        target: String,
        sources: Vec<String>,
    },
}

/// A basic block in the CFG — a sequence of statements with no branches.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub statements: Vec<CfgStatement>,
}

impl BasicBlock {
    pub fn new() -> Self {
        Self {
            statements: Vec::new(),
        }
    }
}

/// Edge types in the control flow graph.
#[derive(Debug, Clone)]
pub enum CfgEdge {
    /// Normal sequential flow
    Normal,
    /// True branch of a conditional
    TrueBranch,
    /// False branch of a conditional
    FalseBranch,
    /// Exception/error path
    Exception,
    /// Cleanup path (Go defer, C++ RAII destructors)
    Cleanup,
}

/// A complete CFG for a single function.
#[derive(Debug, Clone)]
pub struct FunctionCfg {
    pub blocks: DiGraph<BasicBlock, CfgEdge>,
    pub entry: NodeIndex,
    pub exits: Vec<NodeIndex>,
}

impl FunctionCfg {
    pub fn new() -> Self {
        let mut blocks = DiGraph::new();
        let entry = blocks.add_node(BasicBlock::new());
        Self {
            blocks,
            entry,
            exits: Vec::new(),
        }
    }
}
