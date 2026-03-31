use std::collections::{HashSet, VecDeque};

use petgraph::Direction;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use super::cfg::{CfgEdge, CfgStatementKind, FunctionCfg};
use super::{CodeGraph, EdgeWeight, NodeWeight};

/// Per-function analysis result: resources that need graph edges.
struct ResourceEdge {
    /// The function node in the CodeGraph.
    function_node: NodeIndex,
    /// Resource type for the Acquires edge.
    resource_type: String,
    /// Whether the resource is properly released on all paths.
    released: bool,
}

/// Call-based release patterns for languages where the CFG builder may emit
/// resource releases as plain `Call` statements rather than `ResourceRelease`.
/// This covers Go (defer-based cleanup) and PHP (no resource annotations in CFG).
const CALL_BASED_RELEASE_NAMES: &[&str] = &[
    // Go
    "Close",
    "close",
    // PHP
    "fclose",
    "mysqli_close",
    "pg_close",
    "curl_close",
    // General
    "free",
    "release",
    "dispose",
    "Dispose",
    "destroy",
    "shutdown",
    "disconnect",
    "end",
];

/// Call-based acquire patterns for languages where the CFG builder may emit
/// resource acquisitions as plain `Call` statements.
const CALL_BASED_ACQUIRE_NAMES: &[&str] = &[
    // PHP
    "fopen",
    "mysqli_connect",
    "pg_connect",
    "curl_init",
    // Go
    "Open",
    "Dial",
    "Listen",
    "Connect",
];

pub struct ResourceAnalyzer;

impl ResourceAnalyzer {
    /// Walk every function CFG in the graph and add Acquires/ReleasedBy edges.
    pub fn analyze_all(graph: &mut CodeGraph) {
        // Collect function nodes and their CFGs (we need to borrow graph mutably later).
        let function_entries: Vec<(NodeIndex, FunctionCfg)> = graph
            .function_cfgs
            .iter()
            .map(|(&node_idx, cfg)| (node_idx, cfg.clone()))
            .collect();

        let mut edges_to_add: Vec<ResourceEdge> = Vec::new();

        for (func_node, cfg) in &function_entries {
            let results = analyze_function_resources(cfg);
            for (resource_type, released) in results {
                edges_to_add.push(ResourceEdge {
                    function_node: *func_node,
                    resource_type,
                    released,
                });
            }
        }

        // Now add edges to the graph.
        for edge in edges_to_add {
            // Create a CallSite node to represent the resource operation.
            let (file_path, line) = match &graph.graph[edge.function_node] {
                NodeWeight::Symbol {
                    file_path,
                    start_line,
                    ..
                } => (file_path.clone(), *start_line),
                _ => continue,
            };

            let resource_node = graph.graph.add_node(NodeWeight::CallSite {
                name: format!("acquire:{}", edge.resource_type),
                file_path,
                line,
            });

            // Function -> resource: Acquires edge
            graph.graph.add_edge(
                edge.function_node,
                resource_node,
                EdgeWeight::Acquires {
                    resource_type: edge.resource_type,
                },
            );

            // resource -> function: ReleasedBy edge (only if properly released)
            if edge.released {
                graph
                    .graph
                    .add_edge(resource_node, edge.function_node, EdgeWeight::ReleasedBy);
            }
        }
    }
}

/// Analyze a single function's CFG for resource lifecycle issues.
///
/// Returns a list of (resource_type, released) pairs. Each entry represents a
/// distinct resource acquisition found in the CFG. `released` is true only if
/// the resource is released on *every* path from acquisition to function exit.
fn analyze_function_resources(cfg: &FunctionCfg) -> Vec<(String, bool)> {
    // Phase 1: Collect all resource acquire/release events from the CFG.
    let mut acquires: Vec<AcquireInfo> = Vec::new();
    let mut releases: Vec<ReleaseInfo> = Vec::new();
    let mut call_acquires: Vec<CallAcquireInfo> = Vec::new();
    let mut call_releases: Vec<CallReleaseInfo> = Vec::new();

    for block_idx in cfg.blocks.node_indices() {
        let block = &cfg.blocks[block_idx];
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            match &stmt.kind {
                CfgStatementKind::ResourceAcquire {
                    target,
                    resource_type,
                } => {
                    acquires.push(AcquireInfo {
                        block: block_idx,
                        stmt_idx,
                        variable: target.clone(),
                        resource_type: resource_type.clone(),
                        line: stmt.line,
                    });
                }
                CfgStatementKind::ResourceRelease {
                    target,
                    resource_type: _,
                } => {
                    releases.push(ReleaseInfo {
                        block: block_idx,
                        variable: target.clone(),
                    });
                }
                CfgStatementKind::Call { name, args } => {
                    // Check for call-based release patterns (Go defer, PHP, etc.)
                    let base_name = name.rsplit('.').next().unwrap_or(name);
                    if CALL_BASED_RELEASE_NAMES.contains(&base_name) {
                        // The released variable is typically the first arg or the
                        // receiver (before the dot).
                        let target = if name.contains('.') {
                            // Method call: receiver.Close() -> receiver is the target
                            name.split('.').next().unwrap_or("").to_string()
                        } else {
                            // Free-function: fclose(fp) -> first arg is the target
                            args.first().cloned().unwrap_or_default()
                        };
                        if !target.is_empty() {
                            call_releases.push(CallReleaseInfo {
                                block: block_idx,
                                variable: target,
                            });
                        }
                    }
                    // Check for call-based acquire patterns
                    if CALL_BASED_ACQUIRE_NAMES.contains(&base_name) {
                        call_acquires.push(CallAcquireInfo {
                            block: block_idx,
                            stmt_idx,
                            call_name: base_name.to_string(),
                            line: stmt.line,
                        });
                    }
                }
                CfgStatementKind::Assignment {
                    target,
                    source_vars,
                } if !source_vars.is_empty() => {
                    // Check if the assignment sources a call-based acquire.
                    // This is handled by looking at the *previous* statement
                    // being a call acquire -- but we can't easily track that
                    // without more context. We'll handle call_acquires separately.
                    let _ = target;
                }
                _ => {}
            }
        }
    }

    // Phase 2: Build the set of blocks reachable via Cleanup edges from each block.
    // Cleanup blocks (Go defer, C# using, Java try-with-resources, Python with)
    // contain releases that always execute.
    let cleanup_releases = collect_cleanup_releases(cfg);

    // Phase 3: For each resource acquire, determine if it's released on all paths
    // to function exits.
    let mut results: Vec<(String, bool)> = Vec::new();

    // Handle explicit ResourceAcquire statements
    for acq in &acquires {
        let released_on_all_paths = is_released_on_all_paths(
            cfg,
            acq.block,
            &acq.variable,
            &releases,
            &call_releases,
            &cleanup_releases,
        );
        results.push((acq.resource_type.clone(), released_on_all_paths));
    }

    // Handle call-based acquires: try to find the target variable from the
    // assignment in the same block or surrounding context.
    for call_acq in &call_acquires {
        let target_var = find_assignment_target_for_call(cfg, call_acq.block, call_acq.stmt_idx);
        let variable = target_var.unwrap_or_else(|| format!("<anon:{}>", call_acq.call_name));

        let released_on_all_paths = is_released_on_all_paths(
            cfg,
            call_acq.block,
            &variable,
            &releases,
            &call_releases,
            &cleanup_releases,
        );
        results.push((call_acq.call_name.clone(), released_on_all_paths));
    }

    results
}

struct AcquireInfo {
    block: NodeIndex,
    #[allow(dead_code)]
    stmt_idx: usize,
    variable: String,
    resource_type: String,
    #[allow(dead_code)]
    line: u32,
}

struct ReleaseInfo {
    block: NodeIndex,
    variable: String,
}

struct CallAcquireInfo {
    block: NodeIndex,
    stmt_idx: usize,
    call_name: String,
    #[allow(dead_code)]
    line: u32,
}

struct CallReleaseInfo {
    block: NodeIndex,
    variable: String,
}

/// Collect the set of variable names released in Cleanup-edge blocks.
/// These always execute (defer, using, with, try-with-resources), so
/// any resource released in a cleanup block is considered released.
fn collect_cleanup_releases(cfg: &FunctionCfg) -> HashSet<String> {
    let mut released = HashSet::new();

    for edge in cfg.blocks.edge_references() {
        if !matches!(edge.weight(), CfgEdge::Cleanup) {
            continue;
        }
        let cleanup_block_idx = edge.target();
        let cleanup_block = &cfg.blocks[cleanup_block_idx];
        for stmt in &cleanup_block.statements {
            match &stmt.kind {
                CfgStatementKind::ResourceRelease { target, .. } => {
                    released.insert(target.clone());
                }
                CfgStatementKind::Call { name, args } => {
                    let base_name = name.rsplit('.').next().unwrap_or(name);
                    if CALL_BASED_RELEASE_NAMES.contains(&base_name) {
                        let target = if name.contains('.') {
                            name.split('.').next().unwrap_or("").to_string()
                        } else {
                            args.first().cloned().unwrap_or_default()
                        };
                        if !target.is_empty() {
                            released.insert(target);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    released
}

/// Check whether a resource variable is released on all paths from `acquire_block`
/// to every function exit.
///
/// Uses a simplified BFS approach: we track which blocks have been visited with
/// the resource still live (not yet released). If we reach an exit block without
/// finding a release, the resource leaks on that path.
fn is_released_on_all_paths(
    cfg: &FunctionCfg,
    acquire_block: NodeIndex,
    variable: &str,
    explicit_releases: &[ReleaseInfo],
    call_releases: &[CallReleaseInfo],
    cleanup_releases: &HashSet<String>,
) -> bool {
    // Fast path: if the variable is released in a cleanup block, it's always released.
    if cleanup_releases.contains(variable) {
        return true;
    }

    // Build a set of blocks that release this variable.
    let release_blocks: HashSet<NodeIndex> = explicit_releases
        .iter()
        .filter(|r| r.variable == variable)
        .map(|r| r.block)
        .chain(
            call_releases
                .iter()
                .filter(|r| r.variable == variable)
                .map(|r| r.block),
        )
        .collect();

    // If there are no releases and no cleanup releases, it's definitely leaked.
    if release_blocks.is_empty() {
        return false;
    }

    // BFS from acquire_block: walk forward through CFG edges.
    // If we reach any exit without hitting a release block, the resource leaks.
    let exits: HashSet<NodeIndex> = cfg.exits.iter().copied().collect();

    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut visited: HashSet<NodeIndex> = HashSet::new();

    queue.push_back(acquire_block);
    visited.insert(acquire_block);

    while let Some(block_idx) = queue.pop_front() {
        // If this block releases the variable, don't continue on this path.
        if block_idx != acquire_block && release_blocks.contains(&block_idx) {
            continue;
        }

        // If this is an exit block, the resource was not released on this path.
        if exits.contains(&block_idx) {
            // Check if the exit block itself contains the release.
            if release_blocks.contains(&block_idx) {
                continue;
            }
            return false;
        }

        // Walk successors (all edge types).
        for edge in cfg.blocks.edges_directed(block_idx, Direction::Outgoing) {
            let next = edge.target();
            if visited.insert(next) {
                queue.push_back(next);
            }
        }
    }

    true
}

/// Try to find the assignment target for a call-based acquire.
/// Looks at the statements in the same block: if the call appears as a source
/// in an Assignment, the target of that assignment is the resource variable.
fn find_assignment_target_for_call(
    cfg: &FunctionCfg,
    block: NodeIndex,
    call_stmt_idx: usize,
) -> Option<String> {
    let block_data = &cfg.blocks[block];
    // Check if there's an assignment right before or at the same index
    // that references this call. In practice, the CFG builder often emits
    // the call as part of an Assignment (e.g., `f := os.Open(...)` becomes
    // Assignment { target: "f", source_vars: ["os", "Open", ...] }).
    // We look for an Assignment immediately preceding or at the call position.
    if call_stmt_idx > 0
        && let CfgStatementKind::Assignment { target, .. } =
            &block_data.statements[call_stmt_idx - 1].kind
    {
        return Some(target.clone());
    }
    // Also check the statement at the same index (if it was folded into an assignment).
    if let Some(stmt) = block_data.statements.get(call_stmt_idx)
        && let CfgStatementKind::Assignment { target, .. } = &stmt.kind
    {
        return Some(target.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

    /// Helper: build a simple linear CFG with given statements.
    fn linear_cfg(statements: Vec<CfgStatement>) -> FunctionCfg {
        let mut cfg = FunctionCfg::new();
        for stmt in statements {
            cfg.blocks[cfg.entry].statements.push(stmt);
        }
        cfg.exits.push(cfg.entry);
        cfg
    }

    #[test]
    fn resource_acquired_and_released_in_same_block() {
        let cfg = linear_cfg(vec![
            CfgStatement {
                kind: CfgStatementKind::ResourceAcquire {
                    target: "fp".into(),
                    resource_type: "fopen".into(),
                },
                line: 1,
            },
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "process".into(),
                    args: vec!["fp".into()],
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::ResourceRelease {
                    target: "fp".into(),
                    resource_type: "fclose".into(),
                },
                line: 3,
            },
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars: vec![] },
                line: 4,
            },
        ]);

        let results = analyze_function_resources(&cfg);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "fopen");
        assert!(results[0].1, "resource should be marked as released");
    }

    #[test]
    fn resource_leaked_no_release() {
        let cfg = linear_cfg(vec![
            CfgStatement {
                kind: CfgStatementKind::ResourceAcquire {
                    target: "ptr".into(),
                    resource_type: "malloc".into(),
                },
                line: 1,
            },
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars: vec![] },
                line: 2,
            },
        ]);

        let results = analyze_function_resources(&cfg);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "malloc");
        assert!(!results[0].1, "resource should be marked as leaked");
    }

    #[test]
    fn resource_released_via_cleanup_edge() {
        // Simulates a Python `with` or C# `using` pattern:
        // entry -> body -> cleanup(release) -> exit
        let mut cfg = FunctionCfg::new();

        // Entry block: acquire resource
        cfg.blocks[cfg.entry].statements.push(CfgStatement {
            kind: CfgStatementKind::ResourceAcquire {
                target: "f".into(),
                resource_type: "open".into(),
            },
            line: 1,
        });

        // Body block
        let body = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks.add_edge(cfg.entry, body, CfgEdge::Normal);
        cfg.blocks[body].statements.push(CfgStatement {
            kind: CfgStatementKind::Call {
                name: "process".into(),
                args: vec!["f".into()],
            },
            line: 2,
        });

        // Cleanup block (reached via Cleanup edge)
        let cleanup = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks.add_edge(body, cleanup, CfgEdge::Cleanup);
        cfg.blocks[cleanup].statements.push(CfgStatement {
            kind: CfgStatementKind::ResourceRelease {
                target: "f".into(),
                resource_type: "close".into(),
            },
            line: 3,
        });

        cfg.exits.push(cleanup);

        let results = analyze_function_resources(&cfg);
        assert_eq!(results.len(), 1);
        assert!(
            results[0].1,
            "resource released via cleanup should be marked as released"
        );
    }

    #[test]
    fn resource_leaked_on_one_branch() {
        // if (cond) { release(fp); } else { /* leak */ }
        let mut cfg = FunctionCfg::new();

        // Entry: acquire
        cfg.blocks[cfg.entry].statements.push(CfgStatement {
            kind: CfgStatementKind::ResourceAcquire {
                target: "fp".into(),
                resource_type: "fopen".into(),
            },
            line: 1,
        });
        cfg.blocks[cfg.entry].statements.push(CfgStatement {
            kind: CfgStatementKind::Guard {
                condition_vars: vec!["cond".into()],
            },
            line: 2,
        });

        // True branch: releases
        let true_block = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks
            .add_edge(cfg.entry, true_block, CfgEdge::TrueBranch);
        cfg.blocks[true_block].statements.push(CfgStatement {
            kind: CfgStatementKind::ResourceRelease {
                target: "fp".into(),
                resource_type: "fclose".into(),
            },
            line: 3,
        });

        // False branch: no release
        let false_block = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks
            .add_edge(cfg.entry, false_block, CfgEdge::FalseBranch);

        // Join block -> exit
        let join = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks.add_edge(true_block, join, CfgEdge::Normal);
        cfg.blocks.add_edge(false_block, join, CfgEdge::Normal);
        cfg.exits.push(join);

        let results = analyze_function_resources(&cfg);
        assert_eq!(results.len(), 1);
        assert!(
            !results[0].1,
            "resource should be marked as leaked (not released on false branch)"
        );
    }

    #[test]
    fn call_based_release_go_defer() {
        // Simulates Go: f := os.Open(); defer f.Close()
        let mut cfg = FunctionCfg::new();

        // Entry: call-based acquire (recorded as plain Call in Go)
        cfg.blocks[cfg.entry].statements.push(CfgStatement {
            kind: CfgStatementKind::Assignment {
                target: "f".into(),
                source_vars: vec!["os".into()],
            },
            line: 1,
        });
        cfg.blocks[cfg.entry].statements.push(CfgStatement {
            kind: CfgStatementKind::Call {
                name: "os.Open".into(),
                args: vec!["file.txt".into()],
            },
            line: 1,
        });

        // Cleanup block with defer Close
        let cleanup = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks.add_edge(cfg.entry, cleanup, CfgEdge::Cleanup);
        cfg.blocks[cleanup].statements.push(CfgStatement {
            kind: CfgStatementKind::Call {
                name: "f.Close".into(),
                args: vec![],
            },
            line: 2,
        });

        // Return
        cfg.blocks[cfg.entry].statements.push(CfgStatement {
            kind: CfgStatementKind::Return { value_vars: vec![] },
            line: 3,
        });
        cfg.exits.push(cfg.entry);

        let results = analyze_function_resources(&cfg);
        // The Open call is detected as a call-based acquire, and f.Close in
        // cleanup is a call-based release.
        assert!(
            results.iter().all(|(_, released)| *released),
            "Go defer pattern should mark resources as released"
        );
    }

    #[test]
    fn multiple_resources_independent_tracking() {
        let cfg = linear_cfg(vec![
            CfgStatement {
                kind: CfgStatementKind::ResourceAcquire {
                    target: "fp1".into(),
                    resource_type: "fopen".into(),
                },
                line: 1,
            },
            CfgStatement {
                kind: CfgStatementKind::ResourceAcquire {
                    target: "fp2".into(),
                    resource_type: "fopen".into(),
                },
                line: 2,
            },
            CfgStatement {
                kind: CfgStatementKind::ResourceRelease {
                    target: "fp1".into(),
                    resource_type: "fclose".into(),
                },
                line: 3,
            },
            // fp2 is NOT released
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars: vec![] },
                line: 4,
            },
        ]);

        let results = analyze_function_resources(&cfg);
        assert_eq!(results.len(), 2);
        // First resource (fp1) is released
        assert!(results[0].1, "fp1 should be released");
        // Second resource (fp2) is leaked
        assert!(!results[1].1, "fp2 should be leaked");
    }

    #[test]
    fn no_resources_returns_empty() {
        let cfg = linear_cfg(vec![
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "println".into(),
                    args: vec!["hello".into()],
                },
                line: 1,
            },
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars: vec![] },
                line: 2,
            },
        ]);

        let results = analyze_function_resources(&cfg);
        assert!(results.is_empty());
    }
}
