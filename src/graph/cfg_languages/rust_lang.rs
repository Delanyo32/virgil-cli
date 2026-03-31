use anyhow::Result;
use petgraph::graph::NodeIndex;
use tree_sitter::Node;

use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

use super::CfgBuilder;

/// Rust-specific CFG builder. Constructs intra-procedural control flow graphs
/// from tree-sitter ASTs for Rust functions (function_item nodes).
pub struct RustCfgBuilder;

impl CfgBuilder for RustCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let mut ctx = BuildCtx::new();

        // Find the function body block
        let body = function_node
            .child_by_field_name("body")
            .ok_or_else(|| anyhow::anyhow!("function has no body"))?;

        let entry = ctx.cfg.entry;
        let exit = ctx.new_block();
        ctx.cfg.exits.push(exit);

        let after = ctx.process_block(&body, entry, exit, source);
        // If the block fell through without a terminator, connect to exit
        if after != exit && !ctx.has_edge_from(after) {
            ctx.add_edge(after, exit, CfgEdge::Normal);
        }

        Ok(ctx.cfg)
    }
}

/// Internal context that accumulates the CFG during construction.
struct BuildCtx {
    cfg: FunctionCfg,
    /// Block where error-path edges from `?` operator go.
    /// Created lazily on first `?` encounter.
    error_exit: Option<NodeIndex>,
    /// Stack of loop headers for `break`/`continue` resolution.
    loop_stack: Vec<LoopInfo>,
}

struct LoopInfo {
    header: NodeIndex,
    /// Blocks that `break` out of this loop, need edges to the block after the loop.
    break_targets: Vec<NodeIndex>,
}

impl BuildCtx {
    fn new() -> Self {
        Self {
            cfg: FunctionCfg::new(),
            error_exit: None,
            loop_stack: Vec::new(),
        }
    }

    fn new_block(&mut self) -> NodeIndex {
        self.cfg.blocks.add_node(BasicBlock::new())
    }

    fn add_stmt(&mut self, block: NodeIndex, stmt: CfgStatement) {
        self.cfg.blocks[block].statements.push(stmt);
    }

    fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, edge: CfgEdge) {
        self.cfg.blocks.add_edge(from, to, edge);
    }

    fn has_edge_from(&self, block: NodeIndex) -> bool {
        self.cfg
            .blocks
            .edges_directed(block, petgraph::Direction::Outgoing)
            .next()
            .is_some()
    }

    /// Get or create the error exit block (for `?` operator paths).
    fn error_exit(&mut self) -> NodeIndex {
        if let Some(idx) = self.error_exit {
            idx
        } else {
            let block = self.new_block();
            self.add_stmt(
                block,
                CfgStatement {
                    kind: CfgStatementKind::Return {
                        value_vars: vec!["Err".to_string()],
                    },
                    line: 0,
                },
            );
            self.cfg.exits.push(block);
            self.error_exit = Some(block);
            block
        }
    }

    /// Process a `block` node (sequence of statements between `{` and `}`).
    /// Returns the block where control continues after all statements.
    /// If control is terminated (return, break, continue), returns the terminal
    /// block that already has outgoing edges.
    fn process_block(
        &mut self,
        block_node: &Node,
        mut current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        let mut cursor = block_node.walk();
        for child in block_node.named_children(&mut cursor) {
            if self.has_edge_from(current) {
                // Control was already terminated (return/break/continue); unreachable code.
                break;
            }
            current = self.process_statement(&child, current, exit, source);
        }
        current
    }

    /// Process a single statement-level node.
    /// Returns the block where control continues after this statement.
    fn process_statement(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        match node.kind() {
            "let_declaration" => self.process_let(node, current, source),
            "expression_statement" => {
                // Unwrap the expression inside
                if let Some(expr) = node.named_child(0) {
                    self.process_expression_stmt(&expr, current, exit, source)
                } else {
                    current
                }
            }
            "if_expression" => self.process_if(node, current, exit, source),
            "match_expression" => self.process_match(node, current, exit, source),
            "loop_expression" => self.process_loop(node, current, exit, source),
            "while_expression" => self.process_while(node, current, exit, source),
            "for_expression" => self.process_for(node, current, exit, source),
            "return_expression" => self.process_return(node, current, exit, source),
            "macro_invocation" => {
                self.process_macro(node, current, source);
                current
            }
            // Handle bare expressions that appear as direct block children
            "call_expression" | "method_call_expression" => {
                self.process_call_stmt(node, current, source);
                current
            }
            "try_expression" => self.process_try(node, current, source),
            "assignment_expression" => {
                self.process_assignment(node, current, source);
                current
            }
            "compound_assignment_expr" => {
                self.process_assignment(node, current, source);
                current
            }
            "block" => self.process_block(node, current, exit, source),
            // For any other node, check if it contains interesting sub-expressions
            _ => {
                self.scan_for_effects(node, current, source);
                current
            }
        }
    }

    /// Process an expression that appears as a statement (the child of expression_statement).
    fn process_expression_stmt(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        match node.kind() {
            "if_expression" => self.process_if(node, current, exit, source),
            "match_expression" => self.process_match(node, current, exit, source),
            "return_expression" => self.process_return(node, current, exit, source),
            "call_expression" | "method_call_expression" => {
                self.process_call_stmt(node, current, source);
                current
            }
            "try_expression" => self.process_try(node, current, source),
            "macro_invocation" => {
                self.process_macro(node, current, source);
                current
            }
            "assignment_expression" => {
                self.process_assignment(node, current, source);
                current
            }
            "compound_assignment_expr" => {
                self.process_assignment(node, current, source);
                current
            }
            _ => {
                self.scan_for_effects(node, current, source);
                current
            }
        }
    }

    // ── let_declaration ──

    fn process_let(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> NodeIndex {
        let line = node.start_position().row as u32;
        let target = node
            .child_by_field_name("pattern")
            .and_then(|n| n.utf8_text(source).ok())
            .unwrap_or("")
            .to_string();

        let mut source_vars = Vec::new();
        if let Some(value) = node.child_by_field_name("value") {
            collect_identifiers(&value, source, &mut source_vars);
        }

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target,
                    source_vars,
                },
                line,
            },
        );

        // Check if the value contains a `?` operator or a call
        if let Some(value) = node.child_by_field_name("value") {
            self.scan_nested_try_and_calls(&value, current, source);
        }

        current
    }

    // ── Assignment ──

    fn process_assignment(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        let line = node.start_position().row as u32;
        let target = node
            .child_by_field_name("left")
            .and_then(|n| n.utf8_text(source).ok())
            .unwrap_or("")
            .to_string();

        let mut source_vars = Vec::new();
        if let Some(right) = node.child_by_field_name("right") {
            collect_identifiers(&right, source, &mut source_vars);
        }

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target,
                    source_vars,
                },
                line,
            },
        );
    }

    // ── if_expression ──

    fn process_if(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        let line = node.start_position().row as u32;

        // Extract condition variables
        let mut condition_vars = Vec::new();
        if let Some(cond) = node.child_by_field_name("condition") {
            collect_identifiers(&cond, source, &mut condition_vars);
        }
        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line,
            },
        );

        let join = self.new_block();

        // True branch (consequence)
        let true_block = self.new_block();
        self.add_edge(current, true_block, CfgEdge::TrueBranch);
        if let Some(consequence) = node.child_by_field_name("consequence") {
            let after_true = self.process_block(&consequence, true_block, exit, source);
            if !self.has_edge_from(after_true) {
                self.add_edge(after_true, join, CfgEdge::Normal);
            }
        } else {
            self.add_edge(true_block, join, CfgEdge::Normal);
        }

        // False branch (alternative = else_clause)
        if let Some(alternative) = node.child_by_field_name("alternative") {
            let false_block = self.new_block();
            self.add_edge(current, false_block, CfgEdge::FalseBranch);

            // else_clause may contain a block or another if_expression (else if)
            let inner = alternative.named_child(0);
            if let Some(inner) = inner {
                let after_false = if inner.kind() == "if_expression" {
                    self.process_if(&inner, false_block, exit, source)
                } else {
                    self.process_block(&inner, false_block, exit, source)
                };
                if !self.has_edge_from(after_false) {
                    self.add_edge(after_false, join, CfgEdge::Normal);
                }
            } else {
                self.add_edge(false_block, join, CfgEdge::Normal);
            }
        } else {
            // No else — false branch goes directly to join
            self.add_edge(current, join, CfgEdge::FalseBranch);
        }

        join
    }

    // ── match_expression ──

    fn process_match(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        let line = node.start_position().row as u32;

        // Record the match value as a guard
        let mut condition_vars = Vec::new();
        if let Some(value) = node.child_by_field_name("value") {
            collect_identifiers(&value, source, &mut condition_vars);
        }
        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line,
            },
        );

        let join = self.new_block();

        // Find the match body
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for arm in body.named_children(&mut cursor) {
                if arm.kind() != "match_arm" {
                    continue;
                }
                let arm_block = self.new_block();
                self.add_edge(current, arm_block, CfgEdge::Normal);

                // Check for a guard on the match arm
                if let Some(guard) = find_child_by_kind(&arm, "match_arm_guard") {
                    let mut guard_vars = Vec::new();
                    collect_identifiers(&guard, source, &mut guard_vars);
                    self.add_stmt(
                        arm_block,
                        CfgStatement {
                            kind: CfgStatementKind::Guard {
                                condition_vars: guard_vars,
                            },
                            line: arm.start_position().row as u32,
                        },
                    );
                }

                // Process the arm body (the value/expression after =>)
                // In tree-sitter-rust, the match arm value is the last named child
                // that is not the pattern or guard.
                let arm_value = arm.child_by_field_name("value");
                let after_arm = if let Some(value) = arm_value {
                    self.process_arm_value(&value, arm_block, exit, source)
                } else {
                    arm_block
                };

                if !self.has_edge_from(after_arm) {
                    self.add_edge(after_arm, join, CfgEdge::Normal);
                }
            }
        }

        join
    }

    /// Process the value expression of a match arm.
    fn process_arm_value(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        match node.kind() {
            "block" => self.process_block(node, current, exit, source),
            "call_expression" | "method_call_expression" => {
                self.process_call_stmt(node, current, source);
                current
            }
            "if_expression" => self.process_if(node, current, exit, source),
            "match_expression" => self.process_match(node, current, exit, source),
            "return_expression" => self.process_return(node, current, exit, source),
            "macro_invocation" => {
                self.process_macro(node, current, source);
                current
            }
            "try_expression" => self.process_try(node, current, source),
            _ => {
                self.scan_for_effects(node, current, source);
                current
            }
        }
    }

    // ── loop_expression (infinite loop) ──

    fn process_loop(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        let header = self.new_block();
        let after_loop = self.new_block();
        self.add_edge(current, header, CfgEdge::Normal);

        self.loop_stack.push(LoopInfo {
            header,
            break_targets: Vec::new(),
        });

        // Loop body
        if let Some(body) = node.child_by_field_name("body") {
            let body_block = self.new_block();
            self.add_edge(header, body_block, CfgEdge::Normal);
            let after_body = self.process_block(&body, body_block, exit, source);
            // Back edge
            if !self.has_edge_from(after_body) {
                self.add_edge(after_body, header, CfgEdge::Normal);
            }
        }

        let loop_info = self.loop_stack.pop().unwrap();
        for brk in loop_info.break_targets {
            self.add_edge(brk, after_loop, CfgEdge::Normal);
        }

        // Infinite loop has no natural exit unless `break` is used
        after_loop
    }

    // ── while_expression ──

    fn process_while(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        let line = node.start_position().row as u32;

        let header = self.new_block();
        let after_loop = self.new_block();
        self.add_edge(current, header, CfgEdge::Normal);

        // Condition guard
        let mut condition_vars = Vec::new();
        if let Some(cond) = node.child_by_field_name("condition") {
            collect_identifiers(&cond, source, &mut condition_vars);
        }
        self.add_stmt(
            header,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line,
            },
        );

        // True branch: enter loop body
        self.loop_stack.push(LoopInfo {
            header,
            break_targets: Vec::new(),
        });

        if let Some(body) = node.child_by_field_name("body") {
            let body_block = self.new_block();
            self.add_edge(header, body_block, CfgEdge::TrueBranch);
            let after_body = self.process_block(&body, body_block, exit, source);
            // Back edge
            if !self.has_edge_from(after_body) {
                self.add_edge(after_body, header, CfgEdge::Normal);
            }
        }

        let loop_info = self.loop_stack.pop().unwrap();
        for brk in loop_info.break_targets {
            self.add_edge(brk, after_loop, CfgEdge::Normal);
        }

        // False branch: condition fails, exit loop
        self.add_edge(header, after_loop, CfgEdge::FalseBranch);

        after_loop
    }

    // ── for_expression ──

    fn process_for(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        let line = node.start_position().row as u32;

        let header = self.new_block();
        let after_loop = self.new_block();
        self.add_edge(current, header, CfgEdge::Normal);

        // The iterator expression is the "value" field; the pattern is "pattern"
        let mut iter_vars = Vec::new();
        if let Some(value) = node.child_by_field_name("value") {
            collect_identifiers(&value, source, &mut iter_vars);
        }
        self.add_stmt(
            header,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: iter_vars,
                },
                line,
            },
        );

        self.loop_stack.push(LoopInfo {
            header,
            break_targets: Vec::new(),
        });

        if let Some(body) = node.child_by_field_name("body") {
            let body_block = self.new_block();
            self.add_edge(header, body_block, CfgEdge::TrueBranch);
            let after_body = self.process_block(&body, body_block, exit, source);
            if !self.has_edge_from(after_body) {
                self.add_edge(after_body, header, CfgEdge::Normal);
            }
        }

        let loop_info = self.loop_stack.pop().unwrap();
        for brk in loop_info.break_targets {
            self.add_edge(brk, after_loop, CfgEdge::Normal);
        }

        // Iterator exhausted -> exit
        self.add_edge(header, after_loop, CfgEdge::FalseBranch);

        after_loop
    }

    // ── return_expression ──

    fn process_return(
        &mut self,
        node: &Node,
        current: NodeIndex,
        exit: NodeIndex,
        source: &[u8],
    ) -> NodeIndex {
        let line = node.start_position().row as u32;
        let mut value_vars = Vec::new();

        // The return value is the first named child (if any)
        if let Some(value) = node.named_child(0) {
            collect_identifiers(&value, source, &mut value_vars);
        }

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars },
                line,
            },
        );
        self.add_edge(current, exit, CfgEdge::Normal);
        current
    }

    // ── try_expression (? operator) ──

    fn process_try(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> NodeIndex {
        let _line = node.start_position().row as u32;

        // The inner expression is the first named child
        if let Some(inner) = node.named_child(0) {
            match inner.kind() {
                "call_expression" | "method_call_expression" => {
                    self.process_call_stmt(&inner, current, source);
                }
                _ => {
                    self.scan_for_effects(&inner, current, source);
                }
            }
        }

        // The `?` can branch to the error exit
        let err_exit = self.error_exit();
        self.add_edge(current, err_exit, CfgEdge::Exception);

        // Normal path continues in a new block
        let ok_block = self.new_block();
        self.add_edge(current, ok_block, CfgEdge::Normal);
        ok_block
    }

    // ── call_expression / method_call_expression ──

    fn process_call_stmt(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        let line = node.start_position().row as u32;
        let name = extract_call_name(node, source);
        let args = extract_call_args(node, source);

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Call { name, args },
                line,
            },
        );
    }

    // ── macro_invocation ──

    fn process_macro(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        let line = node.start_position().row as u32;
        let name = node
            .child_by_field_name("macro")
            .and_then(|n| n.utf8_text(source).ok())
            .unwrap_or("unknown_macro")
            .to_string();

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: format!("{}!", name),
                    args: Vec::new(),
                },
                line,
            },
        );
    }

    // ── break / continue ──

    fn process_break(&mut self, current: NodeIndex) {
        if let Some(loop_info) = self.loop_stack.last_mut() {
            loop_info.break_targets.push(current);
        }
        // `current` already has no outgoing edges, which signals "terminated" to the caller.
        // The edge to after_loop is added when the loop is popped.
    }

    fn process_continue(&mut self, current: NodeIndex) {
        if let Some(loop_info) = self.loop_stack.last() {
            self.add_edge(current, loop_info.header, CfgEdge::Normal);
        }
    }

    // ── Helpers ──

    /// Scan a node for nested `?` operators, calls, and break/continue inside expressions.
    fn scan_nested_try_and_calls(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        match node.kind() {
            "try_expression" => {
                // Record the inner effect, then add the error edge.
                // We don't split blocks here because we're inside a larger statement;
                // just record the exception edge from the current block.
                if let Some(inner) = node.named_child(0) {
                    if matches!(
                        inner.kind(),
                        "call_expression" | "method_call_expression"
                    ) {
                        self.process_call_stmt(&inner, current, source);
                    }
                }
                let err_exit = self.error_exit();
                self.add_edge(current, err_exit, CfgEdge::Exception);
            }
            "call_expression" | "method_call_expression" => {
                self.process_call_stmt(node, current, source);
            }
            "macro_invocation" => {
                self.process_macro(node, current, source);
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.scan_nested_try_and_calls(&child, current, source);
                }
            }
        }
    }

    /// Scan a node for side effects (calls, macros, try) without creating new blocks.
    fn scan_for_effects(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        match node.kind() {
            "call_expression" | "method_call_expression" => {
                self.process_call_stmt(node, current, source);
            }
            "macro_invocation" => {
                self.process_macro(node, current, source);
            }
            "break_expression" => {
                self.process_break(current);
            }
            "continue_expression" => {
                self.process_continue(current);
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.scan_for_effects(&child, current, source);
                }
            }
        }
    }
}

// ── Free-standing helpers ──

/// Extract the name of a function/method being called.
fn extract_call_name(node: &Node, source: &[u8]) -> String {
    if node.kind() == "method_call_expression" {
        // method_call_expression has field "name" for the method name
        // and field "value" for the receiver
        if let Some(name_node) = node.child_by_field_name("name") {
            return name_node
                .utf8_text(source)
                .unwrap_or("unknown")
                .to_string();
        }
    }

    // call_expression: function child is the callee
    if let Some(func) = node.child_by_field_name("function") {
        let text = func.utf8_text(source).unwrap_or("unknown");
        // For qualified paths like `Vec::new`, take the last segment
        if let Some(pos) = text.rfind("::") {
            return text[pos + 2..].to_string();
        }
        return text.to_string();
    }

    "unknown".to_string()
}

/// Extract argument variable names from a call's arguments node.
fn extract_call_args(node: &Node, source: &[u8]) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(arg_list) = node.child_by_field_name("arguments") {
        let mut cursor = arg_list.walk();
        for child in arg_list.named_children(&mut cursor) {
            collect_identifiers(&child, source, &mut args);
        }
    }
    args
}

/// Collect all identifier names reachable from a node (shallow variable references).
fn collect_identifiers(node: &Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            let s = text.to_string();
            if !s.is_empty() && !is_keyword(&s) {
                out.push(s);
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_identifiers(&child, source, out);
    }
}

/// Find a direct child by kind name.
fn find_child_by_kind<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Filter out Rust keywords that tree-sitter may parse as identifiers in some contexts.
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "true"
            | "false"
            | "self"
            | "Self"
            | "super"
            | "crate"
            | "mut"
            | "ref"
            | "in"
            | "as"
            | "if"
            | "else"
            | "match"
            | "loop"
            | "while"
            | "for"
            | "return"
            | "break"
            | "continue"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;
    use petgraph::visit::EdgeRef;
    use petgraph::Direction;

    fn build_cfg_for(code: &str) -> FunctionCfg {
        let mut parser = create_parser(Language::Rust).expect("create parser");
        let tree = parser.parse(code.as_bytes(), None).expect("parse");
        let root = tree.root_node();

        // Find the first function_item
        let func = find_function_item(root).expect("no function_item found");
        let builder = RustCfgBuilder;
        builder.build_cfg(&func, code.as_bytes()).expect("build_cfg")
    }

    fn find_function_item(node: Node) -> Option<Node> {
        if node.kind() == "function_item" {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(found) = find_function_item(child) {
                return Some(found);
            }
        }
        None
    }

    fn block_stmt_kinds(cfg: &FunctionCfg, idx: NodeIndex) -> Vec<String> {
        cfg.blocks[idx]
            .statements
            .iter()
            .map(|s| match &s.kind {
                CfgStatementKind::Assignment { target, .. } => format!("assign:{}", target),
                CfgStatementKind::Call { name, .. } => format!("call:{}", name),
                CfgStatementKind::Return { .. } => "return".to_string(),
                CfgStatementKind::Guard { .. } => "guard".to_string(),
                CfgStatementKind::ResourceAcquire { .. } => "acquire".to_string(),
                CfgStatementKind::ResourceRelease { .. } => "release".to_string(),
                CfgStatementKind::PhiNode { .. } => "phi".to_string(),
            })
            .collect()
    }

    fn out_edges(cfg: &FunctionCfg, idx: NodeIndex) -> Vec<(CfgEdge, NodeIndex)> {
        cfg.blocks
            .edges_directed(idx, Direction::Outgoing)
            .map(|e| (e.weight().clone(), e.target()))
            .collect()
    }

    fn edge_kinds(cfg: &FunctionCfg, idx: NodeIndex) -> Vec<String> {
        out_edges(cfg, idx)
            .iter()
            .map(|(e, _)| format!("{:?}", e))
            .collect()
    }

    #[test]
    fn simple_function() {
        let cfg = build_cfg_for(
            r#"
            fn foo() {
                let x = 1;
                let y = x;
            }
        "#,
        );
        assert!(!cfg.exits.is_empty());
        // Entry block should have the two assignments
        let kinds = block_stmt_kinds(&cfg, cfg.entry);
        assert_eq!(kinds.len(), 2);
        assert!(kinds[0].starts_with("assign:"));
        assert!(kinds[1].starts_with("assign:"));
    }

    #[test]
    fn function_with_return() {
        let cfg = build_cfg_for(
            r#"
            fn bar() -> i32 {
                let x = 42;
                return x;
            }
        "#,
        );
        let kinds = block_stmt_kinds(&cfg, cfg.entry);
        assert!(kinds.contains(&"return".to_string()));
    }

    #[test]
    fn if_expression() {
        let cfg = build_cfg_for(
            r#"
            fn check(x: i32) {
                if x > 0 {
                    let a = 1;
                } else {
                    let b = 2;
                }
            }
        "#,
        );
        // Entry should have a guard
        let kinds = block_stmt_kinds(&cfg, cfg.entry);
        assert!(kinds.contains(&"guard".to_string()));

        // Entry should have TrueBranch and FalseBranch edges
        let edges = edge_kinds(&cfg, cfg.entry);
        assert!(edges.contains(&"TrueBranch".to_string()));
        assert!(edges.contains(&"FalseBranch".to_string()));
    }

    #[test]
    fn while_loop() {
        let cfg = build_cfg_for(
            r#"
            fn loopy() {
                let mut i = 0;
                while i < 10 {
                    i += 1;
                }
            }
        "#,
        );
        assert!(cfg.blocks.node_count() >= 1, "should have at least one block");
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn for_loop() {
        let cfg = build_cfg_for(
            r#"
            fn iterate() {
                for item in vec![1, 2, 3] {
                    println!("{}", item);
                }
            }
        "#,
        );
        assert!(cfg.blocks.node_count() >= 1, "should have at least one block");
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn match_expression() {
        let cfg = build_cfg_for(
            r#"
            fn matcher(x: i32) -> &'static str {
                match x {
                    0 => "zero",
                    1 => "one",
                    _ => "other",
                }
            }
        "#,
        );
        // Entry should have a guard for the match value
        let kinds = block_stmt_kinds(&cfg, cfg.entry);
        assert!(kinds.contains(&"guard".to_string()));
        // Should have edges to match arms
        let edges = out_edges(&cfg, cfg.entry);
        assert!(edges.len() >= 3); // at least 3 arms
    }

    #[test]
    fn try_operator() {
        let cfg = build_cfg_for(
            r#"
            fn fallible() -> Result<(), String> {
                let x = some_fn()?;
                Ok(())
            }
        "#,
        );
        // Should have an Exception edge somewhere
        let has_exception = cfg.blocks.edge_indices().any(|e| {
            matches!(cfg.blocks.edge_weight(e), Some(CfgEdge::Exception))
        });
        assert!(has_exception, "expected Exception edge for ? operator");
    }

    #[test]
    fn call_expression() {
        let cfg = build_cfg_for(
            r#"
            fn caller() {
                do_something(1, 2);
            }
        "#,
        );
        let kinds = block_stmt_kinds(&cfg, cfg.entry);
        assert!(
            kinds.iter().any(|k| k.starts_with("call:")),
            "expected a call statement, got {:?}",
            kinds
        );
    }

    #[test]
    fn method_call() {
        let cfg = build_cfg_for(
            r#"
            fn user() {
                let v = Vec::new();
                v.push(42);
            }
        "#,
        );
        assert!(cfg.blocks.node_count() >= 1, "should have at least one block");
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn macro_invocation() {
        let cfg = build_cfg_for(
            r#"
            fn noisy() {
                println!("hello");
            }
        "#,
        );
        let kinds = block_stmt_kinds(&cfg, cfg.entry);
        assert!(
            kinds.iter().any(|k| k == "call:println!"),
            "expected call:println!, got {:?}",
            kinds
        );
    }

    #[test]
    fn infinite_loop_with_break() {
        let cfg = build_cfg_for(
            r#"
            fn inf() {
                loop {
                    break;
                }
            }
        "#,
        );
        assert!(cfg.blocks.node_count() >= 1, "should have at least one block");
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }
}
