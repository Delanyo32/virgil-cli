use anyhow::Result;
use petgraph::graph::NodeIndex;
use tree_sitter::Node;

use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

use super::CfgBuilder;

pub struct JavaCfgBuilder;

impl CfgBuilder for JavaCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let mut ctx = BuildCtx::new();

        // The entry block is already created by FunctionCfg::new().
        let body = find_body(function_node);
        let Some(body) = body else {
            // Abstract/interface method with no body -- single empty block is the CFG.
            ctx.cfg.exits.push(ctx.cfg.entry);
            return Ok(ctx.cfg);
        };

        let entry = ctx.cfg.entry;
        let exit = ctx.new_block();

        ctx.build_block(&body, source, entry, exit, None, None);

        // Any block that falls through to `exit` is a normal exit.
        ctx.cfg.exits.push(exit);

        // Collect return-statement blocks added along the way.
        let extra_exits: Vec<NodeIndex> = ctx.return_exits.clone();
        ctx.cfg.exits.extend(extra_exits);

        // Deduplicate exits.
        ctx.cfg.exits.sort_by_key(|n| n.index());
        ctx.cfg.exits.dedup();

        Ok(ctx.cfg)
    }
}

// ---------------------------------------------------------------------------
// Internal builder context
// ---------------------------------------------------------------------------

struct BuildCtx {
    cfg: FunctionCfg,
    /// Blocks that end with a `return` statement.
    return_exits: Vec<NodeIndex>,
}

impl BuildCtx {
    fn new() -> Self {
        Self {
            cfg: FunctionCfg::new(),
            return_exits: Vec::new(),
        }
    }

    fn new_block(&mut self) -> NodeIndex {
        self.cfg.blocks.add_node(BasicBlock::new())
    }

    fn push_stmt(&mut self, block: NodeIndex, stmt: CfgStatement) {
        self.cfg.blocks[block].statements.push(stmt);
    }

    fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, edge: CfgEdge) {
        self.cfg.blocks.add_edge(from, to, edge);
    }

    // -- Recursive statement processing ------------------------------------

    /// Process children of a compound statement (`block` node) sequentially.
    /// Returns the block that the last statement falls through to (may differ
    /// from `current` when control-flow nodes split blocks).
    fn build_block(
        &mut self,
        node: &Node,
        source: &[u8],
        current: NodeIndex,
        after: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        let mut cur = current;
        for (i, child) in children.iter().enumerate() {
            let is_last = i == children.len() - 1;
            let next_block = if is_last { after } else { self.new_block() };
            self.process_statement(
                child,
                source,
                cur,
                next_block,
                break_target,
                continue_target,
            );
            cur = next_block;
        }
    }

    /// Process a single statement node. Adds CFG nodes/edges and statements.
    fn process_statement(
        &mut self,
        node: &Node,
        source: &[u8],
        current: NodeIndex,
        after: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) {
        match node.kind() {
            "block" => {
                self.build_block(node, source, current, after, break_target, continue_target);
            }

            // ── if ────────────────────────────────────────────────────────
            "if_statement" => {
                self.build_if(node, source, current, after, break_target, continue_target);
            }

            // ── loops ─────────────────────────────────────────────────────
            "for_statement" => {
                self.build_for(node, source, current, after);
            }
            "enhanced_for_statement" => {
                self.build_enhanced_for(node, source, current, after);
            }
            "while_statement" => {
                self.build_while(node, source, current, after);
            }
            "do_statement" => {
                self.build_do_while(node, source, current, after);
            }

            // ── switch ────────────────────────────────────────────────────
            "switch_expression" | "switch_statement" => {
                self.build_switch(node, source, current, after);
            }

            // ── try / try-with-resources ──────────────────────────────────
            "try_statement" => {
                self.build_try(node, source, current, after, break_target, continue_target);
            }
            "try_with_resources_statement" => {
                self.build_try_with_resources(
                    node,
                    source,
                    current,
                    after,
                    break_target,
                    continue_target,
                );
            }

            // ── return ────────────────────────────────────────────────────
            "return_statement" => {
                let vars = collect_identifiers_in(node, source);
                self.push_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Return { value_vars: vars },
                        line: node.start_position().row as u32,
                    },
                );
                self.return_exits.push(current);
                // No edge to `after` -- control does not fall through.
            }

            // ── break / continue ──────────────────────────────────────────
            "break_statement" => {
                if let Some(bt) = break_target {
                    self.add_edge(current, bt, CfgEdge::Normal);
                }
                // No fall-through.
            }
            "continue_statement" => {
                if let Some(ct) = continue_target {
                    self.add_edge(current, ct, CfgEdge::Normal);
                }
            }

            // ── throw ─────────────────────────────────────────────────────
            "throw_statement" => {
                let vars = collect_identifiers_in(node, source);
                self.push_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Return { value_vars: vars },
                        line: node.start_position().row as u32,
                    },
                );
                // Throw does not fall through -- treat similarly to return.
                self.return_exits.push(current);
            }

            // ── local variable declaration ────────────────────────────────
            "local_variable_declaration" => {
                self.process_local_var_decl(node, source, current);
                self.add_edge(current, after, CfgEdge::Normal);
            }

            // ── expression_statement (contains calls, assignments, etc.) ──
            "expression_statement" => {
                self.process_expression_stmt(node, source, current);
                self.add_edge(current, after, CfgEdge::Normal);
            }

            // ── Other nodes: fall through with a Normal edge. ─────────────
            _ => {
                // Attempt to extract a statement anyway for common expressions
                // that may appear as direct children (e.g., assignment_expression).
                self.try_extract_expression(node, source, current);
                self.add_edge(current, after, CfgEdge::Normal);
            }
        }
    }

    // ── if_statement ──────────────────────────────────────────────────────

    fn build_if(
        &mut self,
        node: &Node,
        source: &[u8],
        current: NodeIndex,
        after: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) {
        // Guard on condition
        let cond_vars = node
            .child_by_field_name("condition")
            .map(|c| collect_identifiers_in(&c, source))
            .unwrap_or_default();
        self.push_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: cond_vars,
                },
                line: node.start_position().row as u32,
            },
        );

        // True branch
        let true_block = self.new_block();
        self.add_edge(current, true_block, CfgEdge::TrueBranch);
        if let Some(consequence) = node.child_by_field_name("consequence") {
            self.process_statement(
                &consequence,
                source,
                true_block,
                after,
                break_target,
                continue_target,
            );
        } else {
            self.add_edge(true_block, after, CfgEdge::Normal);
        }

        // False branch (else / else-if)
        let false_block = self.new_block();
        self.add_edge(current, false_block, CfgEdge::FalseBranch);
        if let Some(alternative) = node.child_by_field_name("alternative") {
            self.process_statement(
                &alternative,
                source,
                false_block,
                after,
                break_target,
                continue_target,
            );
        } else {
            self.add_edge(false_block, after, CfgEdge::Normal);
        }
    }

    // ── for_statement ─────────────────────────────────────────────────────

    fn build_for(&mut self, node: &Node, source: &[u8], current: NodeIndex, after: NodeIndex) {
        // Initializer runs in `current`
        if let Some(init) = node.child_by_field_name("init") {
            self.try_extract_expression(&init, source, current);
        }

        let cond_block = self.new_block();
        let body_block = self.new_block();
        let update_block = self.new_block();

        self.add_edge(current, cond_block, CfgEdge::Normal);

        // Condition guard
        let cond_vars = node
            .child_by_field_name("condition")
            .map(|c| collect_identifiers_in(&c, source))
            .unwrap_or_default();
        self.push_stmt(
            cond_block,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: cond_vars,
                },
                line: node.start_position().row as u32,
            },
        );

        self.add_edge(cond_block, body_block, CfgEdge::TrueBranch);
        self.add_edge(cond_block, after, CfgEdge::FalseBranch);

        // Body
        if let Some(body) = node.child_by_field_name("body") {
            self.process_statement(
                &body,
                source,
                body_block,
                update_block,
                Some(after),
                Some(update_block),
            );
        } else {
            self.add_edge(body_block, update_block, CfgEdge::Normal);
        }

        // Update
        if let Some(update) = node.child_by_field_name("update") {
            self.try_extract_expression(&update, source, update_block);
        }
        // Back edge
        self.add_edge(update_block, cond_block, CfgEdge::Normal);
    }

    // ── enhanced_for_statement ────────────────────────────────────────────

    fn build_enhanced_for(
        &mut self,
        node: &Node,
        source: &[u8],
        current: NodeIndex,
        after: NodeIndex,
    ) {
        let cond_block = self.new_block();
        let body_block = self.new_block();

        // The iterable is evaluated in current, guard in cond_block
        let iter_vars = node
            .child_by_field_name("value")
            .map(|v| collect_identifiers_in(&v, source))
            .unwrap_or_default();
        self.push_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: iter_vars,
                },
                line: node.start_position().row as u32,
            },
        );

        self.add_edge(current, cond_block, CfgEdge::Normal);
        self.add_edge(cond_block, body_block, CfgEdge::TrueBranch);
        self.add_edge(cond_block, after, CfgEdge::FalseBranch);

        if let Some(body) = node.child_by_field_name("body") {
            self.process_statement(
                &body,
                source,
                body_block,
                cond_block,
                Some(after),
                Some(cond_block),
            );
        } else {
            self.add_edge(body_block, cond_block, CfgEdge::Normal);
        }
    }

    // ── while_statement ───────────────────────────────────────────────────

    fn build_while(&mut self, node: &Node, source: &[u8], current: NodeIndex, after: NodeIndex) {
        let cond_block = self.new_block();
        let body_block = self.new_block();

        self.add_edge(current, cond_block, CfgEdge::Normal);

        let cond_vars = node
            .child_by_field_name("condition")
            .map(|c| collect_identifiers_in(&c, source))
            .unwrap_or_default();
        self.push_stmt(
            cond_block,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: cond_vars,
                },
                line: node.start_position().row as u32,
            },
        );

        self.add_edge(cond_block, body_block, CfgEdge::TrueBranch);
        self.add_edge(cond_block, after, CfgEdge::FalseBranch);

        if let Some(body) = node.child_by_field_name("body") {
            self.process_statement(
                &body,
                source,
                body_block,
                cond_block,
                Some(after),
                Some(cond_block),
            );
        } else {
            self.add_edge(body_block, cond_block, CfgEdge::Normal);
        }
    }

    // ── do_statement ──────────────────────────────────────────────────────

    fn build_do_while(&mut self, node: &Node, source: &[u8], current: NodeIndex, after: NodeIndex) {
        let body_block = self.new_block();
        let cond_block = self.new_block();

        self.add_edge(current, body_block, CfgEdge::Normal);

        if let Some(body) = node.child_by_field_name("body") {
            self.process_statement(
                &body,
                source,
                body_block,
                cond_block,
                Some(after),
                Some(cond_block),
            );
        } else {
            self.add_edge(body_block, cond_block, CfgEdge::Normal);
        }

        let cond_vars = node
            .child_by_field_name("condition")
            .map(|c| collect_identifiers_in(&c, source))
            .unwrap_or_default();
        self.push_stmt(
            cond_block,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: cond_vars,
                },
                line: node.start_position().row as u32,
            },
        );

        // Back edge on true, exit on false
        self.add_edge(cond_block, body_block, CfgEdge::TrueBranch);
        self.add_edge(cond_block, after, CfgEdge::FalseBranch);
    }

    // ── switch_expression / switch_statement ──────────────────────────────

    fn build_switch(&mut self, node: &Node, source: &[u8], current: NodeIndex, after: NodeIndex) {
        // Guard on the condition
        let cond_vars = node
            .child_by_field_name("condition")
            .map(|c| collect_identifiers_in(&c, source))
            .unwrap_or_default();
        self.push_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: cond_vars,
                },
                line: node.start_position().row as u32,
            },
        );

        // Find the switch_block child which contains switch_block_statement_group
        // or switch_rule children.
        let body = node.child_by_field_name("body");

        let mut cursor = node.walk();
        let case_nodes: Vec<Node> = if let Some(ref b) = body {
            let mut c2 = b.walk();
            b.children(&mut c2).collect()
        } else {
            node.children(&mut cursor)
                .filter(|c| c.kind() == "switch_block_statement_group" || c.kind() == "switch_rule")
                .collect()
        };

        let mut has_default = false;
        for case in &case_nodes {
            if case.kind() == "switch_block_statement_group" || case.kind() == "switch_rule" {
                let case_block = self.new_block();
                self.add_edge(current, case_block, CfgEdge::Normal);
                self.build_block(case, source, case_block, after, Some(after), None);
                // Check for default label
                let mut lc = case.walk();
                for child in case.children(&mut lc) {
                    if child.kind() == "switch_label" {
                        let text = child.utf8_text(source).unwrap_or("");
                        if text.starts_with("default") {
                            has_default = true;
                        }
                    }
                }
            }
        }

        // If no default case, current can fall through to after directly.
        if !has_default {
            self.add_edge(current, after, CfgEdge::Normal);
        }
    }

    // ── try_statement ─────────────────────────────────────────────────────

    fn build_try(
        &mut self,
        node: &Node,
        source: &[u8],
        current: NodeIndex,
        after: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) {
        let try_block = self.new_block();
        self.add_edge(current, try_block, CfgEdge::Normal);

        // Find body, catches, finally among children
        let body = node.child_by_field_name("body");
        let mut catches: Vec<Node> = Vec::new();
        let mut finally: Option<Node> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "catch_clause" => catches.push(child),
                "finally_clause" => finally = Some(child),
                _ => {}
            }
        }

        let finally_block = if finally.is_some() {
            Some(self.new_block())
        } else {
            None
        };

        let merge_target = finally_block.unwrap_or(after);

        // Try body
        if let Some(b) = body {
            self.process_statement(
                &b,
                source,
                try_block,
                merge_target,
                break_target,
                continue_target,
            );
        } else {
            self.add_edge(try_block, merge_target, CfgEdge::Normal);
        }

        // Catch clauses -- each gets an Exception edge from try body block
        for catch in &catches {
            let catch_block = self.new_block();
            self.add_edge(try_block, catch_block, CfgEdge::Exception);

            let catch_body = catch.child_by_field_name("body");
            if let Some(cb) = catch_body {
                self.process_statement(
                    &cb,
                    source,
                    catch_block,
                    merge_target,
                    break_target,
                    continue_target,
                );
            } else {
                self.add_edge(catch_block, merge_target, CfgEdge::Normal);
            }
        }

        // Finally clause
        if let (Some(fin), Some(fin_block)) = (finally, finally_block) {
            let fin_body = fin.child_by_field_name("body");
            if let Some(fb) = fin_body {
                self.build_block(&fb, source, fin_block, after, break_target, continue_target);
            } else {
                // Try the first child that is a block
                let mut fc = fin.walk();
                let fin_child_block = fin.children(&mut fc).find(|c| c.kind() == "block");
                if let Some(ref fcb) = fin_child_block {
                    self.build_block(fcb, source, fin_block, after, break_target, continue_target);
                } else {
                    self.add_edge(fin_block, after, CfgEdge::Normal);
                }
            }
        }
    }

    // ── try_with_resources_statement ──────────────────────────────────────

    fn build_try_with_resources(
        &mut self,
        node: &Node,
        source: &[u8],
        current: NodeIndex,
        after: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) {
        // Extract resource declarations from the resource_specification child
        let mut resources: Vec<(String, String)> = Vec::new(); // (name, type)
        if let Some(spec) = node.child_by_field_name("resources") {
            let mut sc = spec.walk();
            for child in spec.children(&mut sc) {
                if child.kind() == "resource" {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("")
                        .to_string();
                    let rtype = child
                        .child_by_field_name("type")
                        .and_then(|t| t.utf8_text(source).ok())
                        .unwrap_or("AutoCloseable")
                        .to_string();
                    if !name.is_empty() {
                        resources.push((name, rtype));
                    }
                }
            }
        }

        // Emit ResourceAcquire for each resource
        for (name, rtype) in &resources {
            self.push_stmt(
                current,
                CfgStatement {
                    kind: CfgStatementKind::ResourceAcquire {
                        target: name.clone(),
                        resource_type: rtype.clone(),
                    },
                    line: node.start_position().row as u32,
                },
            );
        }

        // Build the try body and catches same as regular try
        let try_block = self.new_block();
        self.add_edge(current, try_block, CfgEdge::Normal);

        let body = node.child_by_field_name("body");
        let mut catches: Vec<Node> = Vec::new();
        let mut finally: Option<Node> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "catch_clause" => catches.push(child),
                "finally_clause" => finally = Some(child),
                _ => {}
            }
        }

        // Cleanup block for auto-close (always runs, like finally)
        let cleanup_block = self.new_block();

        let finally_block = if finally.is_some() {
            Some(self.new_block())
        } else {
            None
        };

        let merge_after_body = cleanup_block;

        // Try body
        if let Some(b) = body {
            self.process_statement(
                &b,
                source,
                try_block,
                merge_after_body,
                break_target,
                continue_target,
            );
        } else {
            self.add_edge(try_block, merge_after_body, CfgEdge::Normal);
        }

        // Catch clauses
        for catch in &catches {
            let catch_block = self.new_block();
            self.add_edge(try_block, catch_block, CfgEdge::Exception);

            let catch_body = catch.child_by_field_name("body");
            if let Some(cb) = catch_body {
                self.process_statement(
                    &cb,
                    source,
                    catch_block,
                    merge_after_body,
                    break_target,
                    continue_target,
                );
            } else {
                self.add_edge(catch_block, merge_after_body, CfgEdge::Normal);
            }
        }

        // Cleanup block: emit ResourceRelease for each resource (reverse order)
        for (name, rtype) in resources.iter().rev() {
            self.push_stmt(
                cleanup_block,
                CfgStatement {
                    kind: CfgStatementKind::ResourceRelease {
                        target: name.clone(),
                        resource_type: rtype.clone(),
                    },
                    line: node.end_position().row as u32,
                },
            );
        }

        let cleanup_target = finally_block.unwrap_or(after);
        self.add_edge(cleanup_block, cleanup_target, CfgEdge::Cleanup);

        // Finally clause
        if let (Some(fin), Some(fin_block)) = (finally, finally_block) {
            let fin_body = fin.child_by_field_name("body");
            if let Some(fb) = fin_body {
                self.build_block(&fb, source, fin_block, after, break_target, continue_target);
            } else {
                let mut fc = fin.walk();
                let fin_child_block = fin.children(&mut fc).find(|c| c.kind() == "block");
                if let Some(ref fcb) = fin_child_block {
                    self.build_block(fcb, source, fin_block, after, break_target, continue_target);
                } else {
                    self.add_edge(fin_block, after, CfgEdge::Normal);
                }
            }
        }
    }

    // ── local_variable_declaration ────────────────────────────────────────

    fn process_local_var_decl(&mut self, node: &Node, source: &[u8], block: NodeIndex) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                let target = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .unwrap_or("")
                    .to_string();
                let source_vars = child
                    .child_by_field_name("value")
                    .map(|v| collect_identifiers_in(&v, source))
                    .unwrap_or_default();

                if !target.is_empty() {
                    // Check if the value is a call expression
                    let is_call = child
                        .child_by_field_name("value")
                        .map(|v| {
                            v.kind() == "method_invocation"
                                || v.kind() == "object_creation_expression"
                        })
                        .unwrap_or(false);

                    if is_call && let Some(value_node) = child.child_by_field_name("value") {
                        let call_name = extract_call_name(&value_node, source);
                        let args = extract_call_args(&value_node, source);
                        self.push_stmt(
                            block,
                            CfgStatement {
                                kind: CfgStatementKind::Call {
                                    name: call_name,
                                    args,
                                },
                                line: node.start_position().row as u32,
                            },
                        );
                    }

                    self.push_stmt(
                        block,
                        CfgStatement {
                            kind: CfgStatementKind::Assignment {
                                target,
                                source_vars,
                            },
                            line: node.start_position().row as u32,
                        },
                    );
                }
            }
        }
    }

    // ── expression_statement ──────────────────────────────────────────────

    fn process_expression_stmt(&mut self, node: &Node, source: &[u8], block: NodeIndex) {
        // expression_statement wraps a single expression child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.try_extract_expression(&child, source, block);
        }
    }

    // ── Generic expression extraction ─────────────────────────────────────

    fn try_extract_expression(&mut self, node: &Node, source: &[u8], block: NodeIndex) {
        match node.kind() {
            "method_invocation" => {
                let name = extract_call_name(node, source);
                let args = extract_call_args(node, source);
                self.push_stmt(
                    block,
                    CfgStatement {
                        kind: CfgStatementKind::Call { name, args },
                        line: node.start_position().row as u32,
                    },
                );
            }
            "object_creation_expression" => {
                let name = extract_call_name(node, source);
                let args = extract_call_args(node, source);
                self.push_stmt(
                    block,
                    CfgStatement {
                        kind: CfgStatementKind::Call { name, args },
                        line: node.start_position().row as u32,
                    },
                );
            }
            "assignment_expression" => {
                let target = node
                    .child_by_field_name("left")
                    .and_then(|n| n.utf8_text(source).ok())
                    .unwrap_or("")
                    .to_string();
                let source_vars = node
                    .child_by_field_name("right")
                    .map(|r| collect_identifiers_in(&r, source))
                    .unwrap_or_default();
                if !target.is_empty() {
                    self.push_stmt(
                        block,
                        CfgStatement {
                            kind: CfgStatementKind::Assignment {
                                target,
                                source_vars,
                            },
                            line: node.start_position().row as u32,
                        },
                    );
                }
            }
            "update_expression" => {
                // i++, --j, etc.
                let vars = collect_identifiers_in(node, source);
                if let Some(target) = vars.first() {
                    self.push_stmt(
                        block,
                        CfgStatement {
                            kind: CfgStatementKind::Assignment {
                                target: target.clone(),
                                source_vars: vec![target.clone()],
                            },
                            line: node.start_position().row as u32,
                        },
                    );
                }
            }
            _ => {
                // Walk children for nested calls/assignments we can capture
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "method_invocation"
                        | "object_creation_expression"
                        | "assignment_expression" => {
                            self.try_extract_expression(&child, source, block);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the `block` (body) child of a method/constructor declaration.
fn find_body<'a>(node: &'a Node) -> Option<Node<'a>> {
    node.child_by_field_name("body")
}

/// Extract the callee name from a method_invocation or object_creation_expression.
fn extract_call_name(node: &Node, source: &[u8]) -> String {
    match node.kind() {
        "method_invocation" => {
            // Try the `name` field first, then `object.name` form
            if let Some(name_node) = node.child_by_field_name("name") {
                return name_node
                    .utf8_text(source)
                    .unwrap_or("<unknown>")
                    .to_string();
            }
            // Fallback: full text of the function child
            node.child_by_field_name("object")
                .and_then(|n| n.utf8_text(source).ok())
                .unwrap_or("<unknown>")
                .to_string()
        }
        "object_creation_expression" => {
            // `new Foo(...)` -- the type is the "type" field
            node.child_by_field_name("type")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|t| format!("new {}", t))
                .unwrap_or_else(|| "new <unknown>".to_string())
        }
        _ => "<unknown>".to_string(),
    }
}

/// Extract argument variable names from a call's argument_list.
fn extract_call_args(node: &Node, source: &[u8]) -> Vec<String> {
    let args_node = node.child_by_field_name("arguments");
    let Some(args) = args_node else {
        return Vec::new();
    };
    collect_identifiers_in(&args, source)
}

/// Collect all `identifier` leaf nodes under a subtree.
fn collect_identifiers_in(node: &Node, source: &[u8]) -> Vec<String> {
    let mut result = Vec::new();
    collect_identifiers_recursive(node, source, &mut result);
    result
}

fn collect_identifiers_recursive(node: &Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "identifier" {
        if let Ok(text) = node.utf8_text(source) {
            let s = text.to_string();
            if !s.is_empty() && !out.contains(&s) {
                out.push(s);
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers_recursive(&child, source, out);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;

    /// Parse Java source, find the first method/constructor body, build its CFG.
    fn build_java_cfg(source: &str) -> FunctionCfg {
        let mut parser = create_parser(Language::Java).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");

        let root = tree.root_node();
        let method = find_method_node(root).expect("no method found in source");

        let builder = JavaCfgBuilder;
        builder
            .build_cfg(&method, source.as_bytes())
            .expect("build_cfg failed")
    }

    fn find_method_node(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        if node.kind() == "method_declaration" || node.kind() == "constructor_declaration" {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_method_node(child) {
                return Some(found);
            }
        }
        None
    }

    fn count_edges_of_kind(cfg: &FunctionCfg, kind: &str) -> usize {
        cfg.blocks
            .edge_indices()
            .filter(|&e| {
                let w = &cfg.blocks[e];
                match (w, kind) {
                    (CfgEdge::Normal, "Normal") => true,
                    (CfgEdge::TrueBranch, "TrueBranch") => true,
                    (CfgEdge::FalseBranch, "FalseBranch") => true,
                    (CfgEdge::Exception, "Exception") => true,
                    (CfgEdge::Cleanup, "Cleanup") => true,
                    _ => false,
                }
            })
            .count()
    }

    fn has_statement_kind(
        cfg: &FunctionCfg,
        predicate: impl Fn(&CfgStatementKind) -> bool,
    ) -> bool {
        cfg.blocks.node_indices().any(|idx| {
            cfg.blocks[idx]
                .statements
                .iter()
                .any(|s| predicate(&s.kind))
        })
    }

    #[test]
    fn empty_method() {
        let cfg = build_java_cfg("class Foo { void bar() {} }");
        // Entry + exit at minimum
        assert!(cfg.blocks.node_count() >= 2);
        assert!(!cfg.exits.is_empty());
    }

    #[test]
    fn simple_return() {
        let cfg = build_java_cfg(
            r#"class Foo {
                int bar() {
                    return 42;
                }
            }"#,
        );
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Return { .. }
        )));
    }

    #[test]
    fn local_variable_assignment() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    int x = 10;
                }
            }"#,
        );
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Assignment { target, .. } if target == "x"
        )));
    }

    #[test]
    fn method_invocation() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    System.out.println("hello");
                }
            }"#,
        );
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Call { .. }
        )));
    }

    #[test]
    fn if_statement_branches() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar(int x) {
                    if (x > 0) {
                        return;
                    } else {
                        return;
                    }
                }
            }"#,
        );
        assert!(count_edges_of_kind(&cfg, "TrueBranch") >= 1);
        assert!(count_edges_of_kind(&cfg, "FalseBranch") >= 1);
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Guard { .. }
        )));
    }

    #[test]
    fn while_loop_back_edge() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    while (true) {
                        int x = 1;
                    }
                }
            }"#,
        );
        // Should have true/false branches and a back edge (Normal from body end to cond)
        assert!(count_edges_of_kind(&cfg, "TrueBranch") >= 1);
        assert!(count_edges_of_kind(&cfg, "FalseBranch") >= 1);
    }

    #[test]
    fn for_loop() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    for (int i = 0; i < 10; i++) {
                        int x = i;
                    }
                }
            }"#,
        );
        assert!(count_edges_of_kind(&cfg, "TrueBranch") >= 1);
        assert!(count_edges_of_kind(&cfg, "FalseBranch") >= 1);
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Guard { .. }
        )));
    }

    #[test]
    fn enhanced_for_loop() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar(java.util.List<String> items) {
                    for (String item : items) {
                        System.out.println(item);
                    }
                }
            }"#,
        );
        assert!(count_edges_of_kind(&cfg, "TrueBranch") >= 1);
        assert!(count_edges_of_kind(&cfg, "FalseBranch") >= 1);
    }

    #[test]
    fn do_while_loop() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    do {
                        int x = 1;
                    } while (x > 0);
                }
            }"#,
        );
        assert!(count_edges_of_kind(&cfg, "TrueBranch") >= 1);
        assert!(count_edges_of_kind(&cfg, "FalseBranch") >= 1);
    }

    #[test]
    fn try_catch() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    try {
                        int x = 1;
                    } catch (Exception e) {
                        int y = 2;
                    }
                }
            }"#,
        );
        assert!(count_edges_of_kind(&cfg, "Exception") >= 1);
    }

    #[test]
    fn try_catch_finally() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    try {
                        int x = 1;
                    } catch (Exception e) {
                        int y = 2;
                    } finally {
                        int z = 3;
                    }
                }
            }"#,
        );
        assert!(count_edges_of_kind(&cfg, "Exception") >= 1);
        // The finally block should exist as an additional node
        assert!(cfg.blocks.node_count() >= 4);
    }

    #[test]
    fn try_with_resources() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() throws Exception {
                    try (java.io.InputStream is = new java.io.FileInputStream("f")) {
                        int x = is.read();
                    }
                }
            }"#,
        );
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::ResourceAcquire { .. }
        )));
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::ResourceRelease { .. }
        )));
        assert!(count_edges_of_kind(&cfg, "Cleanup") >= 1);
    }

    #[test]
    fn object_creation_expression() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    Object obj = new Object();
                }
            }"#,
        );
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Call { name, .. } if name.starts_with("new ")
        )));
    }

    #[test]
    fn switch_statement() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar(int x) {
                    switch (x) {
                        case 1:
                            int a = 1;
                            break;
                        case 2:
                            int b = 2;
                            break;
                        default:
                            int c = 3;
                    }
                }
            }"#,
        );
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Guard { .. }
        )));
        // Multiple case blocks should exist
        assert!(cfg.blocks.node_count() >= 4);
    }

    #[test]
    fn assignment_expression() {
        let cfg = build_java_cfg(
            r#"class Foo {
                void bar() {
                    int x;
                    x = 42;
                }
            }"#,
        );
        assert!(has_statement_kind(&cfg, |k| matches!(
            k,
            CfgStatementKind::Assignment { target, .. } if target == "x"
        )));
    }

    #[test]
    fn abstract_method_no_body() {
        // Abstract methods have no body -- CFG should be a single entry/exit block.
        let source = "abstract class Foo { abstract void bar(); }";
        let mut parser = create_parser(Language::Java).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");
        let root = tree.root_node();
        let method = find_method_node(root).expect("no method found");

        let builder = JavaCfgBuilder;
        let cfg = builder
            .build_cfg(&method, source.as_bytes())
            .expect("build");
        assert_eq!(cfg.blocks.node_count(), 1);
        assert_eq!(cfg.exits.len(), 1);
        assert_eq!(cfg.exits[0], cfg.entry);
    }
}
