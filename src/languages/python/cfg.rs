use anyhow::Result;
use petgraph::graph::NodeIndex;
use tree_sitter::Node;

use crate::languages::cfg::CfgBuilder;
use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

/// CFG builder for Python function bodies.
pub struct PythonCfgBuilder;

impl CfgBuilder for PythonCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let mut ctx = BuildCtx::new();

        // Extract parameter names from the function signature
        if let Some(params_node) = function_node.child_by_field_name("parameters") {
            let mut cursor = params_node.walk();
            for child in params_node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => {
                        let name = child.utf8_text(source).unwrap_or("").to_string();
                        if !name.is_empty() {
                            ctx.cfg.param_names.push(name);
                        }
                    }
                    "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                        // first named child is the identifier
                        if let Some(ident) = child.named_child(0)
                            && ident.kind() == "identifier"
                        {
                            let name = ident.utf8_text(source).unwrap_or("").to_string();
                            if !name.is_empty() {
                                ctx.cfg.param_names.push(name);
                            }
                        }
                    }
                    "list_splat_pattern" | "dictionary_splat_pattern" => {
                        // *args / **kwargs — get the inner identifier
                        if let Some(ident) = child.named_child(0) {
                            let name = ident.utf8_text(source).unwrap_or("").to_string();
                            if !name.is_empty() {
                                ctx.cfg.param_names.push(name);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Find the body block of the function
        let body = function_node
            .child_by_field_name("body")
            .ok_or_else(|| anyhow::anyhow!("function has no body"))?;

        let exit = ctx.process_block(&body, ctx.cfg.entry, source);

        // If the last block didn't end with a return, mark it as an implicit exit
        if let Some(exit) = exit
            && !ctx.cfg.exits.contains(&exit)
        {
            ctx.cfg.exits.push(exit);
        }

        // If no explicit exits were recorded, the entry is the exit
        if ctx.cfg.exits.is_empty() {
            ctx.cfg.exits.push(ctx.cfg.entry);
        }

        Ok(ctx.cfg)
    }
}

// ---------------------------------------------------------------------------
// Build context — holds the mutable CFG during construction
// ---------------------------------------------------------------------------

struct BuildCtx {
    cfg: FunctionCfg,
    /// Stack of loop headers for `break` / `continue` resolution
    loop_stack: Vec<LoopInfo>,
}

struct LoopInfo {
    header: NodeIndex,
    /// Collect break targets; they get wired to the block after the loop
    break_targets: Vec<NodeIndex>,
}

impl BuildCtx {
    fn new() -> Self {
        Self {
            cfg: FunctionCfg::new(),
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

    // ------------------------------------------------------------------
    // Top-level: process a `block` node (sequence of statements)
    // Returns the "current" block at the end, or None if all paths returned.
    // ------------------------------------------------------------------

    fn process_block(
        &mut self,
        block_node: &Node,
        mut current: NodeIndex,
        source: &[u8],
    ) -> Option<NodeIndex> {
        let mut cursor = block_node.walk();
        for child in block_node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            match self.process_statement(&child, current, source) {
                Some(next) => current = next,
                None => return None, // control flow terminated (return/break/continue)
            }
        }
        Some(current)
    }

    // ------------------------------------------------------------------
    // Dispatch a single statement node. Returns the "current" block
    // afterward, or None if flow is terminated.
    // ------------------------------------------------------------------

    fn process_statement(
        &mut self,
        node: &Node,
        current: NodeIndex,
        source: &[u8],
    ) -> Option<NodeIndex> {
        match node.kind() {
            "return_statement" => self.process_return(node, current, source),
            "if_statement" => self.process_if(node, current, source),
            "for_statement" => self.process_for(node, current, source),
            "while_statement" => self.process_while(node, current, source),
            "try_statement" => self.process_try(node, current, source),
            "with_statement" => self.process_with(node, current, source),
            "break_statement" => self.process_break(current),
            "continue_statement" => self.process_continue(current),
            "expression_statement" => {
                self.process_expression_statement(node, current, source);
                Some(current)
            }
            "assignment" | "augmented_assignment" => {
                self.process_assignment(node, current, source);
                Some(current)
            }
            // Comprehensions at statement level (list/dict/set/generator)
            "list_comprehension"
            | "dictionary_comprehension"
            | "set_comprehension"
            | "generator_expression" => {
                self.process_comprehension(node, current, source);
                Some(current)
            }
            // Nested function/class definitions -- treat as opaque statements
            "function_definition" | "class_definition" | "decorated_definition" => Some(current),
            // pass, assert, raise, global, nonlocal, delete -- no-op for CFG
            "pass_statement" | "assert_statement" | "global_statement" | "nonlocal_statement"
            | "delete_statement" => Some(current),
            "raise_statement" => {
                // Treat raise like return -- terminates flow
                let vars = collect_identifiers_in_subtree(node, source);
                self.add_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Call {
                            name: "raise".to_string(),
                            args: vars,
                        },
                        line: node.start_position().row as u32 + 1,
                    },
                );
                self.cfg.exits.push(current);
                None
            }
            _ => {
                // Unknown statement -- absorb into current block
                Some(current)
            }
        }
    }

    // ------------------------------------------------------------------
    // return_statement
    // ------------------------------------------------------------------

    fn process_return(
        &mut self,
        node: &Node,
        current: NodeIndex,
        source: &[u8],
    ) -> Option<NodeIndex> {
        let value_vars = if let Some(value) = node.child(1) {
            collect_identifiers_in_subtree(&value, source)
        } else {
            Vec::new()
        };

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars },
                line: node.start_position().row as u32 + 1,
            },
        );
        self.cfg.exits.push(current);
        None // flow terminates
    }

    // ------------------------------------------------------------------
    // if_statement with elif / else
    //
    // Structure:
    //   if_statement
    //     condition: <expr>
    //     consequence: block
    //     alternative: elif_clause | else_clause  (optional)
    // ------------------------------------------------------------------

    fn process_if(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Option<NodeIndex> {
        let join = self.new_block();

        // Condition
        let condition_vars = node
            .child_by_field_name("condition")
            .map(|c| collect_identifiers_in_subtree(&c, source))
            .unwrap_or_default();

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        // True branch (consequence)
        let true_block = self.new_block();
        self.add_edge(current, true_block, CfgEdge::TrueBranch);

        let true_exit = if let Some(body) = node.child_by_field_name("consequence") {
            self.process_block(&body, true_block, source)
        } else {
            Some(true_block)
        };
        if let Some(te) = true_exit {
            self.add_edge(te, join, CfgEdge::Normal);
        }

        // False branch: walk alternative chain (elif / else)
        let false_exit = self.process_if_alternatives(node, current, source);
        match false_exit {
            AltResult::Exited(Some(block)) => {
                self.add_edge(block, join, CfgEdge::Normal);
            }
            AltResult::Exited(None) => {
                // All paths in alternatives returned/broke
            }
            AltResult::NoAlternative(from) => {
                // No else -- false branch goes directly to join
                self.add_edge(from, join, CfgEdge::FalseBranch);
            }
        }

        // If both branches terminated, join is unreachable but we still return it
        // so the caller can continue (dead code is harmless in the CFG)
        Some(join)
    }

    fn process_if_alternatives(
        &mut self,
        node: &Node,
        guard_block: NodeIndex,
        source: &[u8],
    ) -> AltResult {
        let alt = node.child_by_field_name("alternative");
        let Some(alt) = alt else {
            return AltResult::NoAlternative(guard_block);
        };

        match alt.kind() {
            "else_clause" => {
                let else_block = self.new_block();
                self.add_edge(guard_block, else_block, CfgEdge::FalseBranch);
                let body = alt.child_by_field_name("body").or_else(|| {
                    // else body is the block child
                    let mut c = alt.walk();
                    alt.children(&mut c).find(|ch| ch.kind() == "block")
                });
                let exit = if let Some(body) = body {
                    self.process_block(&body, else_block, source)
                } else {
                    Some(else_block)
                };
                AltResult::Exited(exit)
            }
            "elif_clause" => {
                let elif_guard = self.new_block();
                self.add_edge(guard_block, elif_guard, CfgEdge::FalseBranch);

                // Condition
                let condition_vars = alt
                    .child_by_field_name("condition")
                    .map(|c| collect_identifiers_in_subtree(&c, source))
                    .unwrap_or_default();

                self.add_stmt(
                    elif_guard,
                    CfgStatement {
                        kind: CfgStatementKind::Guard { condition_vars },
                        line: alt.start_position().row as u32 + 1,
                    },
                );

                // True branch
                let true_block = self.new_block();
                self.add_edge(elif_guard, true_block, CfgEdge::TrueBranch);
                let true_exit = if let Some(body) = alt.child_by_field_name("consequence") {
                    self.process_block(&body, true_block, source)
                } else {
                    Some(true_block)
                };

                // Recurse for further elif/else
                let false_exit = self.process_if_alternatives(&alt, elif_guard, source);

                // We need to merge true_exit and false_exit results.
                // The caller will wire them to the join.
                // Return a mini-join for this level.
                let mini_join = self.new_block();
                if let Some(te) = true_exit {
                    self.add_edge(te, mini_join, CfgEdge::Normal);
                }
                match false_exit {
                    AltResult::Exited(Some(block)) => {
                        self.add_edge(block, mini_join, CfgEdge::Normal);
                    }
                    AltResult::Exited(None) => {}
                    AltResult::NoAlternative(from) => {
                        self.add_edge(from, mini_join, CfgEdge::FalseBranch);
                    }
                }
                AltResult::Exited(Some(mini_join))
            }
            _ => AltResult::NoAlternative(guard_block),
        }
    }

    // ------------------------------------------------------------------
    // for_statement
    //
    //   for_statement
    //     left: pattern_list / identifier
    //     right: expression
    //     body: block
    //     alternative: else_clause  (optional, for-else)
    // ------------------------------------------------------------------

    fn process_for(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Option<NodeIndex> {
        // Header block with guard (iteration check)
        let header = self.new_block();
        self.add_edge(current, header, CfgEdge::Normal);

        let iter_vars = node
            .child_by_field_name("right")
            .map(|r| collect_identifiers_in_subtree(&r, source))
            .unwrap_or_default();

        // Assignment of loop variable
        if let Some(left) = node.child_by_field_name("left") {
            let target = text_of(&left, source);
            self.add_stmt(
                header,
                CfgStatement {
                    kind: CfgStatementKind::Assignment {
                        target,
                        source_vars: iter_vars.clone(),
                    },
                    line: node.start_position().row as u32 + 1,
                },
            );
        }

        self.add_stmt(
            header,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: iter_vars,
                },
                line: node.start_position().row as u32 + 1,
            },
        );

        let after_loop = self.new_block();

        // Push loop context
        self.loop_stack.push(LoopInfo {
            header,
            break_targets: Vec::new(),
        });

        // Body
        let body_block = self.new_block();
        self.add_edge(header, body_block, CfgEdge::TrueBranch);

        let body_exit = if let Some(body) = node.child_by_field_name("body") {
            self.process_block(&body, body_block, source)
        } else {
            Some(body_block)
        };

        // Back edge: body -> header
        if let Some(be) = body_exit {
            self.add_edge(be, header, CfgEdge::Normal);
        }

        // Pop loop context, wire break targets
        let loop_info = self.loop_stack.pop().unwrap();
        for brk in &loop_info.break_targets {
            self.add_edge(*brk, after_loop, CfgEdge::Normal);
        }

        // for-else: runs if the loop completes without break
        let alt = node.child_by_field_name("alternative").or_else(|| {
            let mut c = node.walk();
            node.children(&mut c).find(|ch| ch.kind() == "else_clause")
        });
        if let Some(alt) = alt {
            let else_block = self.new_block();
            self.add_edge(header, else_block, CfgEdge::FalseBranch);
            let else_body = alt.child_by_field_name("body").or_else(|| {
                let mut c = alt.walk();
                alt.children(&mut c).find(|ch| ch.kind() == "block")
            });
            let else_exit = if let Some(body) = else_body {
                self.process_block(&body, else_block, source)
            } else {
                Some(else_block)
            };
            if let Some(ee) = else_exit {
                self.add_edge(ee, after_loop, CfgEdge::Normal);
            }
        } else {
            // No else -- fall through when iteration exhausted
            self.add_edge(header, after_loop, CfgEdge::FalseBranch);
        }

        Some(after_loop)
    }

    // ------------------------------------------------------------------
    // while_statement
    //
    //   while_statement
    //     condition: expression
    //     body: block
    //     alternative: else_clause  (optional, while-else)
    // ------------------------------------------------------------------

    fn process_while(
        &mut self,
        node: &Node,
        current: NodeIndex,
        source: &[u8],
    ) -> Option<NodeIndex> {
        let header = self.new_block();
        self.add_edge(current, header, CfgEdge::Normal);

        let condition_vars = node
            .child_by_field_name("condition")
            .map(|c| collect_identifiers_in_subtree(&c, source))
            .unwrap_or_default();

        self.add_stmt(
            header,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        let after_loop = self.new_block();

        // Push loop context
        self.loop_stack.push(LoopInfo {
            header,
            break_targets: Vec::new(),
        });

        // Body
        let body_block = self.new_block();
        self.add_edge(header, body_block, CfgEdge::TrueBranch);

        let body_exit = if let Some(body) = node.child_by_field_name("body") {
            self.process_block(&body, body_block, source)
        } else {
            Some(body_block)
        };

        // Back edge
        if let Some(be) = body_exit {
            self.add_edge(be, header, CfgEdge::Normal);
        }

        // Pop loop context
        let loop_info = self.loop_stack.pop().unwrap();
        for brk in &loop_info.break_targets {
            self.add_edge(*brk, after_loop, CfgEdge::Normal);
        }

        // while-else
        let alt = node.child_by_field_name("alternative").or_else(|| {
            let mut c = node.walk();
            node.children(&mut c).find(|ch| ch.kind() == "else_clause")
        });
        if let Some(alt) = alt {
            let else_block = self.new_block();
            self.add_edge(header, else_block, CfgEdge::FalseBranch);
            let else_body = alt.child_by_field_name("body").or_else(|| {
                let mut c = alt.walk();
                alt.children(&mut c).find(|ch| ch.kind() == "block")
            });
            let else_exit = if let Some(body) = else_body {
                self.process_block(&body, else_block, source)
            } else {
                Some(else_block)
            };
            if let Some(ee) = else_exit {
                self.add_edge(ee, after_loop, CfgEdge::Normal);
            }
        } else {
            self.add_edge(header, after_loop, CfgEdge::FalseBranch);
        }

        Some(after_loop)
    }

    // ------------------------------------------------------------------
    // try_statement
    //
    //   try_statement
    //     body: block
    //     (except_clause body: block)*
    //     (else_clause body: block)?     -- runs if no exception
    //     (finally_clause body: block)?  -- always runs
    // ------------------------------------------------------------------

    fn process_try(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Option<NodeIndex> {
        let after_try = self.new_block();

        // Collect except, else, finally clauses
        let mut except_clauses: Vec<Node> = Vec::new();
        let mut else_clause: Option<Node> = None;
        let mut finally_clause: Option<Node> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "except_clause" | "except_group_clause" => except_clauses.push(child),
                "else_clause" => else_clause = Some(child),
                "finally_clause" => finally_clause = Some(child),
                _ => {}
            }
        }

        // Try body
        let try_block = self.new_block();
        self.add_edge(current, try_block, CfgEdge::Normal);

        let body = node.child_by_field_name("body").or_else(|| {
            let mut c = node.walk();
            node.children(&mut c).find(|ch| ch.kind() == "block")
        });
        let try_exit = if let Some(body) = body {
            self.process_block(&body, try_block, source)
        } else {
            Some(try_block)
        };

        // Exception edges: try_block -> each except handler
        // (In a real CFG every statement in the try body could throw;
        //  we approximate by adding a single exception edge from the try block.)
        let mut handler_exits: Vec<Option<NodeIndex>> = Vec::new();
        for exc in &except_clauses {
            let handler = self.new_block();
            self.add_edge(try_block, handler, CfgEdge::Exception);

            // Extract exception variable if present (e.g., `except ValueError as e:`)
            // In tree-sitter-python, except_clause children are positional:
            //   optional type expr, optional `as` + identifier, then block
            if let Some(name) = extract_except_target(exc, source) {
                self.add_stmt(
                    handler,
                    CfgStatement {
                        kind: CfgStatementKind::Assignment {
                            target: name,
                            source_vars: Vec::new(),
                        },
                        line: exc.start_position().row as u32 + 1,
                    },
                );
            }

            let exc_body = exc.child_by_field_name("body").or_else(|| {
                let mut c = exc.walk();
                exc.children(&mut c).find(|ch| ch.kind() == "block")
            });
            let exc_exit = if let Some(body) = exc_body {
                self.process_block(&body, handler, source)
            } else {
                Some(handler)
            };
            handler_exits.push(exc_exit);
        }

        // Else clause: runs if no exception (connects from successful try_exit)
        let normal_exit = if let Some(else_node) = else_clause {
            let else_block = self.new_block();
            if let Some(te) = try_exit {
                self.add_edge(te, else_block, CfgEdge::Normal);
            }
            let else_body = else_node.child_by_field_name("body").or_else(|| {
                let mut c = else_node.walk();
                else_node.children(&mut c).find(|ch| ch.kind() == "block")
            });
            if let Some(body) = else_body {
                self.process_block(&body, else_block, source)
            } else {
                Some(else_block)
            }
        } else {
            try_exit
        };

        // Finally clause
        if let Some(finally_node) = finally_clause {
            let finally_block = self.new_block();

            // All paths flow into finally
            if let Some(ne) = normal_exit {
                self.add_edge(ne, finally_block, CfgEdge::Cleanup);
            }
            for h in handler_exits.iter().flatten() {
                self.add_edge(*h, finally_block, CfgEdge::Cleanup);
            }

            let finally_body = finally_node.child_by_field_name("body").or_else(|| {
                let mut c = finally_node.walk();
                finally_node
                    .children(&mut c)
                    .find(|ch| ch.kind() == "block")
            });
            let finally_exit = if let Some(body) = finally_body {
                self.process_block(&body, finally_block, source)
            } else {
                Some(finally_block)
            };

            if let Some(fe) = finally_exit {
                self.add_edge(fe, after_try, CfgEdge::Normal);
            }
        } else {
            // No finally -- wire normal and handler exits directly to after_try
            if let Some(ne) = normal_exit {
                self.add_edge(ne, after_try, CfgEdge::Normal);
            }
            for h in handler_exits.iter().flatten() {
                self.add_edge(*h, after_try, CfgEdge::Normal);
            }
        }

        Some(after_try)
    }

    // ------------------------------------------------------------------
    // with_statement (context managers)
    //
    //   with_statement
    //     (with_clause
    //       (with_item
    //         value: expression
    //         (as_pattern (identifier))))
    //     body: block
    //
    // Modeled as: ResourceAcquire -> body -> ResourceRelease
    // ------------------------------------------------------------------

    fn process_with(
        &mut self,
        node: &Node,
        current: NodeIndex,
        source: &[u8],
    ) -> Option<NodeIndex> {
        // Extract with items: resource expressions and their bound names
        let mut items: Vec<(String, String)> = Vec::new(); // (target, resource_expr)
        let mut walk = node.walk();
        for child in node.children(&mut walk) {
            if child.kind() == "with_clause" {
                let mut clause_walk = child.walk();
                for item in child.children(&mut clause_walk) {
                    if item.kind() == "with_item" {
                        let resource = item
                            .child_by_field_name("value")
                            .map(|v| text_of(&v, source))
                            .unwrap_or_default();
                        // The target variable (after `as`)
                        let target = extract_with_target(&item, source);
                        let resource_type = extract_resource_type(&resource);
                        items.push((target, resource_type));
                    }
                }
            }
        }

        // Emit ResourceAcquire for each item
        for (target, resource_type) in &items {
            self.add_stmt(
                current,
                CfgStatement {
                    kind: CfgStatementKind::ResourceAcquire {
                        target: target.clone(),
                        resource_type: resource_type.clone(),
                    },
                    line: node.start_position().row as u32 + 1,
                },
            );
        }

        // Process body
        let body_exit = if let Some(body) = node.child_by_field_name("body") {
            self.process_block(&body, current, source)
        } else {
            Some(current)
        };

        // Emit ResourceRelease (cleanup) for each item, in reverse order
        if let Some(exit) = body_exit {
            let cleanup = self.new_block();
            self.add_edge(exit, cleanup, CfgEdge::Cleanup);

            for (target, resource_type) in items.iter().rev() {
                self.add_stmt(
                    cleanup,
                    CfgStatement {
                        kind: CfgStatementKind::ResourceRelease {
                            target: target.clone(),
                            resource_type: resource_type.clone(),
                        },
                        line: node.end_position().row as u32 + 1,
                    },
                );
            }
            Some(cleanup)
        } else {
            None
        }
    }

    // ------------------------------------------------------------------
    // break / continue
    // ------------------------------------------------------------------

    fn process_break(&mut self, current: NodeIndex) -> Option<NodeIndex> {
        if let Some(loop_info) = self.loop_stack.last_mut() {
            loop_info.break_targets.push(current);
        }
        None // terminates this path
    }

    fn process_continue(&mut self, current: NodeIndex) -> Option<NodeIndex> {
        if let Some(loop_info) = self.loop_stack.last() {
            self.add_edge(current, loop_info.header, CfgEdge::Normal);
        }
        None // terminates this path
    }

    // ------------------------------------------------------------------
    // expression_statement — may contain a bare call or assignment
    // ------------------------------------------------------------------

    fn process_expression_statement(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            match child.kind() {
                "assignment" | "augmented_assignment" => {
                    self.process_assignment(&child, current, source);
                }
                "call" => {
                    self.emit_call(&child, current, source);
                }
                _ => {
                    // Check for nested calls (e.g., method chains)
                    self.emit_nested_calls(&child, current, source);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // assignment / augmented_assignment
    // ------------------------------------------------------------------

    fn process_assignment(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        let target = node
            .child_by_field_name("left")
            .map(|n| text_of(&n, source))
            .unwrap_or_default();

        let right = node.child_by_field_name("right");
        let source_vars = right
            .map(|r| collect_identifiers_in_subtree(&r, source))
            .unwrap_or_default();

        // Check if RHS is a call — emit both Call and Assignment
        if let Some(rhs) = right {
            if rhs.kind() == "call" {
                self.emit_call(&rhs, current, source);
            } else {
                // Check for nested calls in RHS
                self.emit_nested_calls(&rhs, current, source);
            }
        }

        if !target.is_empty() {
            self.add_stmt(
                current,
                CfgStatement {
                    kind: CfgStatementKind::Assignment {
                        target,
                        source_vars,
                    },
                    line: node.start_position().row as u32 + 1,
                },
            );
        }
    }

    // ------------------------------------------------------------------
    // Call emission
    // ------------------------------------------------------------------

    fn emit_call(&mut self, call_node: &Node, block: NodeIndex, source: &[u8]) {
        let name = call_node
            .child_by_field_name("function")
            .map(|f| text_of(&f, source))
            .unwrap_or_default();

        let args = call_node
            .child_by_field_name("arguments")
            .map(|a| collect_identifiers_in_subtree(&a, source))
            .unwrap_or_default();

        if !name.is_empty() {
            self.add_stmt(
                block,
                CfgStatement {
                    kind: CfgStatementKind::Call { name, args },
                    line: call_node.start_position().row as u32 + 1,
                },
            );
        }
    }

    /// Walk a subtree looking for `call` nodes and emit Call statements.
    fn emit_nested_calls(&mut self, node: &Node, block: NodeIndex, source: &[u8]) {
        if node.kind() == "call" {
            self.emit_call(node, block, source);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                self.emit_nested_calls(&child, block, source);
            }
        }
    }

    // ------------------------------------------------------------------
    // Comprehensions — modeled as a simple loop
    //
    // `[expr for x in iter if cond]` becomes:
    //   guard(iter) -> assignment(x) -> guard(cond) -> call-like body
    // ------------------------------------------------------------------

    fn process_comprehension(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "for_in_clause" {
                let iter_vars = child
                    .child_by_field_name("value")
                    .map(|v| collect_identifiers_in_subtree(&v, source))
                    .unwrap_or_default();

                self.add_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Guard {
                            condition_vars: iter_vars.clone(),
                        },
                        line: child.start_position().row as u32 + 1,
                    },
                );

                if let Some(left) = child.child_by_field_name("left") {
                    self.add_stmt(
                        current,
                        CfgStatement {
                            kind: CfgStatementKind::Assignment {
                                target: text_of(&left, source),
                                source_vars: iter_vars,
                            },
                            line: child.start_position().row as u32 + 1,
                        },
                    );
                }
            } else if child.kind() == "if_clause" {
                let cond_vars = collect_identifiers_in_subtree(&child, source);
                self.add_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Guard {
                            condition_vars: cond_vars,
                        },
                        line: child.start_position().row as u32 + 1,
                    },
                );
            }
        }

        // Emit any calls in the comprehension body expression
        self.emit_nested_calls(node, current, source);
    }
}

// ---------------------------------------------------------------------------
// Alternative result for if/elif/else chains
// ---------------------------------------------------------------------------

enum AltResult {
    /// A block exit from the alternative (Some = live block, None = all paths terminated)
    Exited(Option<NodeIndex>),
    /// No alternative was present; the given block is the guard that needs a FalseBranch edge
    NoAlternative(NodeIndex),
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/// Get the text content of a node.
fn text_of(node: &Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// Collect all `identifier` leaf nodes in a subtree, deduped, preserving order.
fn collect_identifiers_in_subtree(node: &Node, source: &[u8]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_identifiers_recursive(node, source, &mut result, &mut seen);
    result
}

fn collect_identifiers_recursive(
    node: &Node,
    source: &[u8],
    out: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    if node.kind() == "identifier" {
        let text = text_of(node, source);
        if !text.is_empty() && seen.insert(text.clone()) {
            out.push(text);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers_recursive(&child, source, out, seen);
    }
}

/// Extract the bound variable from an `except_clause` node.
///
/// Python tree-sitter structure for `except ValueError as e:`:
///   except_clause -> type expr, "as" keyword, identifier, ":", block
/// The identifier after `as` is the bound exception variable.
fn extract_except_target(node: &Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let mut found_as = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "as" {
            found_as = true;
            continue;
        }
        if found_as && child.kind() == "identifier" {
            let name = text_of(&child, source);
            if !name.is_empty() {
                return Some(name);
            }
        }
        // Also try: some grammars nest it as `as_pattern`
        if child.kind() == "as_pattern" {
            let mut inner = child.walk();
            for inner_child in child.children(&mut inner) {
                if inner_child.kind() == "identifier" || inner_child.kind() == "as_pattern_target" {
                    let name = text_of(&inner_child, source);
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }
    }
    None
}

/// Extract the target variable name from a `with_item` node.
///
/// Python tree-sitter structure:
///   with_item -> value: expression, and optionally an `as_pattern` child
///   with an `as_pattern_target` or `identifier` inside.
fn extract_with_target(item: &Node, source: &[u8]) -> String {
    // Try to find the `as` target
    let mut cursor = item.walk();
    for child in item.children(&mut cursor) {
        match child.kind() {
            "as_pattern" | "as_pattern_target" => {
                // Look for identifier inside
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "identifier" {
                        return text_of(&inner, source);
                    }
                }
                return text_of(&child, source);
            }
            "identifier" => {
                // Direct identifier after `as`
                return text_of(&child, source);
            }
            _ => {}
        }
    }
    // No target (bare `with expr:`)
    String::new()
}

/// Infer a resource type string from the resource expression text.
///
/// Examples: `open("f.txt")` -> `"file"`, `Lock()` -> `"lock"`,
///           `db.connect()` -> `"connection"`, otherwise `"context_manager"`.
fn extract_resource_type(resource_expr: &str) -> String {
    let lower = resource_expr.to_lowercase();
    if lower.starts_with("open(") || lower.contains(".open(") {
        "file".to_string()
    } else if lower.contains("lock") {
        "lock".to_string()
    } else if lower.contains("connect") || lower.contains("session") {
        "connection".to_string()
    } else if lower.contains("socket") {
        "socket".to_string()
    } else if lower.contains("cursor") {
        "cursor".to_string()
    } else if lower.contains("tempfile") || lower.contains("temporary") {
        "tempfile".to_string()
    } else {
        "context_manager".to_string()
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

    fn build_cfg_for(source: &str) -> FunctionCfg {
        let full = format!("def test_fn():\n{}", indent(source));
        let mut parser = create_parser(Language::Python).expect("create parser");
        let tree = parser.parse(full.as_bytes(), None).expect("parse");
        let root = tree.root_node();

        // Find the function_definition
        let func = find_first_child_of_kind(root, "function_definition")
            .expect("no function_definition found");

        let builder = PythonCfgBuilder;
        builder
            .build_cfg(&func, full.as_bytes())
            .expect("build_cfg")
    }

    fn indent(s: &str) -> String {
        s.lines()
            .map(|l| format!("    {}", l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn find_first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_first_child_of_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }

    fn block_count(cfg: &FunctionCfg) -> usize {
        cfg.blocks.node_count()
    }

    fn has_edge_kind(cfg: &FunctionCfg, kind: &CfgEdge) -> bool {
        cfg.blocks.edge_indices().any(|e| {
            std::mem::discriminant(cfg.blocks.edge_weight(e).unwrap())
                == std::mem::discriminant(kind)
        })
    }

    fn count_stmts_of_kind(cfg: &FunctionCfg, check: impl Fn(&CfgStatementKind) -> bool) -> usize {
        cfg.blocks
            .node_indices()
            .flat_map(|idx| cfg.blocks[idx].statements.iter())
            .filter(|s| check(&s.kind))
            .count()
    }

    // --- Basic tests ---

    #[test]
    fn empty_function() {
        let cfg = build_cfg_for("pass");
        assert!(block_count(&cfg) >= 1);
        assert!(!cfg.exits.is_empty());
    }

    #[test]
    fn simple_return() {
        let cfg = build_cfg_for("return 42");
        let returns = count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Return { .. }));
        assert_eq!(returns, 1);
        assert!(!cfg.exits.is_empty());
    }

    #[test]
    fn assignment() {
        let cfg = build_cfg_for("x = 1");
        let assigns =
            count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Assignment { .. }));
        assert!(assigns >= 1);
    }

    #[test]
    fn function_call() {
        let cfg = build_cfg_for("print(x)");
        let calls = count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Call { .. }));
        assert_eq!(calls, 1);
    }

    #[test]
    fn if_else() {
        let cfg = build_cfg_for("if x:\n    y = 1\nelse:\n    y = 2");
        assert!(has_edge_kind(&cfg, &CfgEdge::TrueBranch));
        assert!(has_edge_kind(&cfg, &CfgEdge::FalseBranch));
        let guards = count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Guard { .. }));
        assert!(guards >= 1);
    }

    #[test]
    fn if_elif_else() {
        let cfg = build_cfg_for("if a:\n    x = 1\nelif b:\n    x = 2\nelse:\n    x = 3");
        let guards = count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Guard { .. }));
        assert!(guards >= 2);
    }

    #[test]
    fn for_loop() {
        let cfg = build_cfg_for("for i in items:\n    process(i)");
        // Should have a back edge (Normal edge from body back to header)
        assert!(has_edge_kind(&cfg, &CfgEdge::TrueBranch));
        assert!(has_edge_kind(&cfg, &CfgEdge::FalseBranch));
        let guards = count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Guard { .. }));
        assert!(guards >= 1);
    }

    #[test]
    fn while_loop() {
        let cfg = build_cfg_for("while cond:\n    do_stuff()");
        assert!(has_edge_kind(&cfg, &CfgEdge::TrueBranch));
        assert!(has_edge_kind(&cfg, &CfgEdge::FalseBranch));
    }

    #[test]
    fn try_except() {
        let cfg = build_cfg_for("try:\n    risky()\nexcept ValueError as e:\n    handle(e)");
        assert!(has_edge_kind(&cfg, &CfgEdge::Exception));
    }

    #[test]
    fn try_finally() {
        let cfg = build_cfg_for("try:\n    risky()\nfinally:\n    cleanup()");
        assert!(has_edge_kind(&cfg, &CfgEdge::Cleanup));
    }

    #[test]
    fn try_except_finally() {
        let cfg = build_cfg_for(
            "try:\n    risky()\nexcept Exception as e:\n    handle(e)\nfinally:\n    cleanup()",
        );
        assert!(has_edge_kind(&cfg, &CfgEdge::Exception));
        assert!(has_edge_kind(&cfg, &CfgEdge::Cleanup));
    }

    #[test]
    fn with_statement() {
        let cfg = build_cfg_for("with open('f') as fh:\n    data = fh.read()");
        let acquires = count_stmts_of_kind(&cfg, |k| {
            matches!(k, CfgStatementKind::ResourceAcquire { .. })
        });
        let releases = count_stmts_of_kind(&cfg, |k| {
            matches!(k, CfgStatementKind::ResourceRelease { .. })
        });
        assert_eq!(acquires, 1);
        assert_eq!(releases, 1);
        assert!(has_edge_kind(&cfg, &CfgEdge::Cleanup));
    }

    #[test]
    fn break_in_loop() {
        let cfg = build_cfg_for("for i in items:\n    if done:\n        break\n    process(i)");
        // Should have at least the loop structure + break wiring
        assert!(block_count(&cfg) >= 4);
    }

    #[test]
    fn continue_in_loop() {
        let cfg = build_cfg_for("for i in items:\n    if skip:\n        continue\n    process(i)");
        assert!(block_count(&cfg) >= 4);
    }

    #[test]
    fn multiple_returns() {
        let cfg = build_cfg_for("if x:\n    return 1\nreturn 2");
        let returns = count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Return { .. }));
        assert_eq!(returns, 2);
        assert!(cfg.exits.len() >= 2);
    }

    #[test]
    fn assignment_with_call() {
        let cfg = build_cfg_for("result = compute(a, b)");
        let calls = count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Call { .. }));
        let assigns =
            count_stmts_of_kind(&cfg, |k| matches!(k, CfgStatementKind::Assignment { .. }));
        assert_eq!(calls, 1);
        assert!(assigns >= 1);
    }

    #[test]
    fn resource_type_detection() {
        assert_eq!(extract_resource_type("open('file.txt')"), "file");
        assert_eq!(extract_resource_type("Lock()"), "lock");
        assert_eq!(extract_resource_type("db.connect()"), "connection");
        assert_eq!(extract_resource_type("foo()"), "context_manager");
    }
}
