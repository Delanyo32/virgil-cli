use anyhow::Result;
use petgraph::graph::NodeIndex;
use tree_sitter::Node;

use crate::languages::cfg::CfgBuilder;
use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

/// CFG builder for TypeScript, JavaScript, TSX, and JSX.
///
/// Walks tree-sitter ASTs top-down to produce intra-procedural control flow
/// graphs for individual functions. Handles the common control flow patterns:
/// if/else, for/while/do loops, switch/case, try/catch/finally, and return.
pub struct TypeScriptCfgBuilder;

impl CfgBuilder for TypeScriptCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let body = find_function_body(function_node).unwrap_or(*function_node);

        let mut ctx = BuildContext::new();

        // Extract parameter names from the function signature.
        // JS/TS grammars use "formal_parameters" for function/method declarations
        // and "formal_parameters" for arrow functions too.
        if let Some(params_node) = function_node.child_by_field_name("parameters") {
            let mut cursor = params_node.walk();
            for child in params_node.named_children(&mut cursor) {
                extract_param_name(&child, source, &mut ctx.cfg.param_names);
            }
        }

        let entry = ctx.cfg.entry;
        let exits = ctx.process_block(&body, entry, source);

        // Mark all dangling exits as function exits
        ctx.cfg.exits = exits;
        Ok(ctx.cfg)
    }
}

/// Find the statement_block body of a function/method/arrow function.
fn find_function_body<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    // function_declaration, method_definition, arrow_function all use "body" field
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    // Fallback: look for a statement_block child
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|&child| child.kind() == "statement_block")
}

/// Mutable context used while building the CFG.
struct BuildContext {
    cfg: FunctionCfg,
}

impl BuildContext {
    fn new() -> Self {
        Self {
            cfg: FunctionCfg::new(),
        }
    }

    /// Add a new empty basic block to the graph.
    fn new_block(&mut self) -> NodeIndex {
        self.cfg.blocks.add_node(BasicBlock::new())
    }

    /// Add a statement to an existing block.
    fn push_stmt(&mut self, block: NodeIndex, stmt: CfgStatement) {
        self.cfg.blocks[block].statements.push(stmt);
    }

    /// Add an edge between two blocks.
    fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, edge: CfgEdge) {
        self.cfg.blocks.add_edge(from, to, edge);
    }

    /// Process a statement_block (or single expression body) and return the
    /// set of exit block indices that flow out of this block.
    fn process_block(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Vec<NodeIndex> {
        if node.kind() == "statement_block" {
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            let mut exits = vec![current];

            for child in &children {
                if exits.is_empty() {
                    break; // unreachable code after return
                }
                // Merge all current exits into one block for the next statement
                let block = if exits.len() == 1 {
                    exits[0]
                } else {
                    let merge = self.new_block();
                    for &ex in &exits {
                        self.add_edge(ex, merge, CfgEdge::Normal);
                    }
                    merge
                };
                exits = self.process_statement(child, block, source);
            }
            exits
        } else {
            // Single expression body (e.g., arrow function `x => x + 1`)
            self.process_expression_statement(node, current, source);
            vec![current]
        }
    }

    /// Process a single statement node. Returns exit block indices.
    fn process_statement(
        &mut self,
        node: &Node,
        current: NodeIndex,
        source: &[u8],
    ) -> Vec<NodeIndex> {
        match node.kind() {
            "if_statement" => self.process_if(node, current, source),
            "for_statement" | "for_in_statement" => self.process_for(node, current, source),
            "while_statement" => self.process_while(node, current, source),
            "do_statement" => self.process_do_while(node, current, source),
            "switch_statement" => self.process_switch(node, current, source),
            "try_statement" => self.process_try(node, current, source),
            "return_statement" => self.process_return(node, current, source),
            "throw_statement" => {
                // throw terminates the block, no exits
                let vars = extract_vars_from_children(node, source);
                self.push_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Return { value_vars: vars },
                        line: node.start_position().row as u32 + 1,
                    },
                );
                vec![] // no normal exits
            }
            "variable_declaration" | "lexical_declaration" => {
                self.process_variable_declaration(node, current, source);
                vec![current]
            }
            "expression_statement" => {
                self.process_expression_statement(node, current, source);
                vec![current]
            }
            "statement_block" => self.process_block(node, current, source),
            // Skip comments, type declarations, etc.
            _ => vec![current],
        }
    }

    // ── if/else ──

    fn process_if(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Vec<NodeIndex> {
        // Add guard for the condition
        let condition_vars = node
            .child_by_field_name("condition")
            .map(|c| extract_vars(&c, source))
            .unwrap_or_default();

        self.push_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        let mut exits = Vec::new();

        // True branch (consequence)
        if let Some(consequence) = node.child_by_field_name("consequence") {
            let true_block = self.new_block();
            self.add_edge(current, true_block, CfgEdge::TrueBranch);
            let true_exits = self.process_block_or_statement(&consequence, true_block, source);
            exits.extend(true_exits);
        }

        // False branch (alternative) — may be another if_statement (else if)
        if let Some(alternative) = node.child_by_field_name("alternative") {
            let false_block = self.new_block();
            self.add_edge(current, false_block, CfgEdge::FalseBranch);
            // else_clause wraps the actual statement
            let inner = if alternative.kind() == "else_clause" {
                alternative.named_child(0)
            } else {
                Some(alternative)
            };
            if let Some(inner) = inner {
                let false_exits = self.process_block_or_statement(&inner, false_block, source);
                exits.extend(false_exits);
            } else {
                exits.push(false_block);
            }
        } else {
            // No else: false branch falls through
            let false_block = self.new_block();
            self.add_edge(current, false_block, CfgEdge::FalseBranch);
            exits.push(false_block);
        }

        exits
    }

    // ── for loop ──

    fn process_for(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Vec<NodeIndex> {
        // Initializer in current block
        if let Some(init) = node.child_by_field_name("initializer") {
            self.process_expression_statement(&init, current, source);
        }

        // Header block with condition guard
        let header = self.new_block();
        self.add_edge(current, header, CfgEdge::Normal);

        let condition_vars = node
            .child_by_field_name("condition")
            .or_else(|| node.child_by_field_name("left")) // for-in/for-of
            .map(|c| extract_vars(&c, source))
            .unwrap_or_default();

        self.push_stmt(
            header,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        // Body
        let body_block = self.new_block();
        self.add_edge(header, body_block, CfgEdge::TrueBranch);

        if let Some(body) = node.child_by_field_name("body") {
            let body_exits = self.process_block_or_statement(&body, body_block, source);
            // Back edge from body exits to header
            for &ex in &body_exits {
                self.add_edge(ex, header, CfgEdge::Normal);
            }
        }

        // Exit: false branch from header
        let exit_block = self.new_block();
        self.add_edge(header, exit_block, CfgEdge::FalseBranch);
        vec![exit_block]
    }

    // ── while loop ──

    fn process_while(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Vec<NodeIndex> {
        let header = self.new_block();
        self.add_edge(current, header, CfgEdge::Normal);

        let condition_vars = node
            .child_by_field_name("condition")
            .map(|c| extract_vars(&c, source))
            .unwrap_or_default();

        self.push_stmt(
            header,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        let body_block = self.new_block();
        self.add_edge(header, body_block, CfgEdge::TrueBranch);

        if let Some(body) = node.child_by_field_name("body") {
            let body_exits = self.process_block_or_statement(&body, body_block, source);
            for &ex in &body_exits {
                self.add_edge(ex, header, CfgEdge::Normal);
            }
        }

        let exit_block = self.new_block();
        self.add_edge(header, exit_block, CfgEdge::FalseBranch);
        vec![exit_block]
    }

    // ── do-while loop ──

    fn process_do_while(
        &mut self,
        node: &Node,
        current: NodeIndex,
        source: &[u8],
    ) -> Vec<NodeIndex> {
        // Body executes first
        let body_block = self.new_block();
        self.add_edge(current, body_block, CfgEdge::Normal);

        let body_exits = if let Some(body) = node.child_by_field_name("body") {
            self.process_block_or_statement(&body, body_block, source)
        } else {
            vec![body_block]
        };

        // Condition check after body
        let cond_block = self.new_block();
        for &ex in &body_exits {
            self.add_edge(ex, cond_block, CfgEdge::Normal);
        }

        let condition_vars = node
            .child_by_field_name("condition")
            .map(|c| extract_vars(&c, source))
            .unwrap_or_default();

        self.push_stmt(
            cond_block,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        // True branch loops back to body
        self.add_edge(cond_block, body_block, CfgEdge::TrueBranch);

        // False branch exits
        let exit_block = self.new_block();
        self.add_edge(cond_block, exit_block, CfgEdge::FalseBranch);
        vec![exit_block]
    }

    // ── switch/case ──

    fn process_switch(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Vec<NodeIndex> {
        // Guard for the switch expression
        let condition_vars = node
            .child_by_field_name("value")
            .map(|c| extract_vars(&c, source))
            .unwrap_or_default();

        self.push_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        let mut exits = Vec::new();
        let mut has_default = false;

        // Find switch_body
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            let cases: Vec<Node> = body.children(&mut cursor).collect();

            for case_node in &cases {
                if case_node.kind() != "switch_case" && case_node.kind() != "switch_default" {
                    continue;
                }
                if case_node.kind() == "switch_default" {
                    has_default = true;
                }

                let case_block = self.new_block();
                self.add_edge(current, case_block, CfgEdge::Normal);

                // Process case body statements
                let mut case_exits = vec![case_block];
                let mut child_cursor = case_node.walk();
                for child in case_node.children(&mut child_cursor) {
                    // Skip the case value node itself
                    if child.kind() == ":" || !child.is_named() {
                        continue;
                    }
                    // Skip the case value expression (first named child for switch_case)
                    if child.kind() != "statement_block"
                        && child.kind() != "expression_statement"
                        && child.kind() != "return_statement"
                        && child.kind() != "break_statement"
                        && child.kind() != "if_statement"
                        && child.kind() != "variable_declaration"
                        && child.kind() != "lexical_declaration"
                        && child.kind() != "throw_statement"
                        && child.kind() != "for_statement"
                        && child.kind() != "while_statement"
                        && child.kind() != "do_statement"
                        && child.kind() != "switch_statement"
                        && child.kind() != "try_statement"
                    {
                        continue;
                    }

                    if case_exits.is_empty() {
                        break;
                    }
                    let block = if case_exits.len() == 1 {
                        case_exits[0]
                    } else {
                        let merge = self.new_block();
                        for &ex in &case_exits {
                            self.add_edge(ex, merge, CfgEdge::Normal);
                        }
                        merge
                    };

                    if child.kind() == "break_statement" {
                        // break exits the switch
                        exits.push(block);
                        case_exits = vec![];
                    } else {
                        case_exits = self.process_statement(&child, block, source);
                    }
                }
                exits.extend(case_exits);
            }
        }

        // If no default case, the switch expression itself can fall through
        if !has_default {
            let fallthrough = self.new_block();
            self.add_edge(current, fallthrough, CfgEdge::Normal);
            exits.push(fallthrough);
        }

        if exits.is_empty() {
            // All branches returned/threw — no exits
            exits
        } else {
            exits
        }
    }

    // ── try/catch/finally ──

    fn process_try(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Vec<NodeIndex> {
        let mut exits = Vec::new();

        // Try body
        let try_block = self.new_block();
        self.add_edge(current, try_block, CfgEdge::Normal);

        let try_exits = if let Some(body) = node.child_by_field_name("body") {
            self.process_block(&body, try_block, source)
        } else {
            vec![try_block]
        };

        // Catch handler
        let catch_exits = if let Some(handler) = node.child_by_field_name("handler") {
            let catch_block = self.new_block();
            // Exception edge from try block to catch
            self.add_edge(try_block, catch_block, CfgEdge::Exception);

            // Find the catch body (statement_block inside catch_clause)
            if let Some(catch_body) = handler.child_by_field_name("body") {
                self.process_block(&catch_body, catch_block, source)
            } else {
                vec![catch_block]
            }
        } else {
            vec![]
        };

        // Finally
        if let Some(finalizer) = node.child_by_field_name("finalizer") {
            // finally_clause wraps a statement_block
            let finally_body = finalizer.child_by_field_name("body").or_else(|| {
                // Some grammars have the block as a direct child
                let mut cursor = finalizer.walk();
                finalizer
                    .children(&mut cursor)
                    .find(|c| c.kind() == "statement_block")
            });

            if let Some(body) = finally_body {
                // All paths flow through finally
                let all_exits: Vec<NodeIndex> = try_exits
                    .iter()
                    .chain(catch_exits.iter())
                    .copied()
                    .collect();
                for &ex in &all_exits {
                    let finally_block = self.new_block();
                    self.add_edge(ex, finally_block, CfgEdge::Cleanup);
                    let finally_exits = self.process_block(&body, finally_block, source);
                    exits.extend(finally_exits);
                }
            } else {
                exits.extend(try_exits);
                exits.extend(catch_exits);
            }
        } else {
            exits.extend(try_exits);
            exits.extend(catch_exits);
        }

        exits
    }

    // ── return ──

    fn process_return(&mut self, node: &Node, current: NodeIndex, source: &[u8]) -> Vec<NodeIndex> {
        let value_vars = node
            .child_by_field_name("value")
            .or_else(|| node.named_child(0))
            .map(|v| extract_vars(&v, source))
            .unwrap_or_default();

        self.push_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars },
                line: node.start_position().row as u32 + 1,
            },
        );

        // Return terminates the block — it becomes an exit
        self.cfg.exits.push(current);
        vec![] // no normal successors
    }

    // ── Variable declarations ──

    fn process_variable_declaration(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "variable_declarator" {
                continue;
            }

            let target = child
                .child_by_field_name("name")
                .and_then(|n| extract_identifier_text(&n, source))
                .unwrap_or_default();

            if target.is_empty() {
                continue;
            }

            let value_node = child.child_by_field_name("value");

            // Check if the value is a call expression, new expression, or await
            if let Some(ref val) = value_node
                && let Some(stmt) = try_extract_call_or_resource(val, source, &target)
            {
                self.push_stmt(
                    current,
                    CfgStatement {
                        kind: stmt,
                        line: child.start_position().row as u32 + 1,
                    },
                );
                continue;
            }

            let source_vars = value_node
                .map(|v| extract_vars(&v, source))
                .unwrap_or_default();

            self.push_stmt(
                current,
                CfgStatement {
                    kind: CfgStatementKind::Assignment {
                        target,
                        source_vars,
                    },
                    line: child.start_position().row as u32 + 1,
                },
            );
        }
    }

    // ── Expression statements ──

    fn process_expression_statement(&mut self, node: &Node, current: NodeIndex, source: &[u8]) {
        // Unwrap expression_statement wrapper
        let expr = if node.kind() == "expression_statement" {
            match node.named_child(0) {
                Some(inner) => inner,
                None => return,
            }
        } else {
            *node
        };

        match expr.kind() {
            "assignment_expression" => {
                let target = expr
                    .child_by_field_name("left")
                    .and_then(|n| extract_identifier_text(&n, source))
                    .unwrap_or_default();

                if let Some(right) = expr.child_by_field_name("right") {
                    // Check if RHS is a call
                    if let Some(stmt) = try_extract_call_or_resource(&right, source, &target) {
                        self.push_stmt(
                            current,
                            CfgStatement {
                                kind: stmt,
                                line: expr.start_position().row as u32 + 1,
                            },
                        );
                        return;
                    }

                    let source_vars = extract_vars(&right, source);
                    if !target.is_empty() {
                        self.push_stmt(
                            current,
                            CfgStatement {
                                kind: CfgStatementKind::Assignment {
                                    target,
                                    source_vars,
                                },
                                line: expr.start_position().row as u32 + 1,
                            },
                        );
                    }
                }
            }
            "augmented_assignment_expression" => {
                let target = expr
                    .child_by_field_name("left")
                    .and_then(|n| extract_identifier_text(&n, source))
                    .unwrap_or_default();

                if !target.is_empty() {
                    let right = expr.child_by_field_name("right");
                    let mut source_vars =
                        right.map(|r| extract_vars(&r, source)).unwrap_or_default();
                    // Augmented assignment reads the target too (e.g., x += y reads x and y)
                    if !source_vars.contains(&target) {
                        source_vars.push(target.clone());
                    }
                    self.push_stmt(
                        current,
                        CfgStatement {
                            kind: CfgStatementKind::Assignment {
                                target,
                                source_vars,
                            },
                            line: expr.start_position().row as u32 + 1,
                        },
                    );
                }
            }
            "call_expression" | "new_expression" => {
                let (name, args) = extract_call_info(&expr, source);
                self.push_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Call { name, args },
                        line: expr.start_position().row as u32 + 1,
                    },
                );
            }
            "await_expression" => {
                // Treat await as synchronous — unwrap to inner expression
                if let Some(inner) = expr.named_child(0) {
                    let inner_node = if inner.kind() == "expression_statement" {
                        inner.named_child(0).unwrap_or(inner)
                    } else {
                        inner
                    };
                    if inner_node.kind() == "call_expression"
                        || inner_node.kind() == "new_expression"
                    {
                        let (name, args) = extract_call_info(&inner_node, source);
                        self.push_stmt(
                            current,
                            CfgStatement {
                                kind: CfgStatementKind::Call { name, args },
                                line: expr.start_position().row as u32 + 1,
                            },
                        );
                    }
                }
            }
            // .then() chains: treat as calls
            "member_expression" => {
                // Unlikely standalone, but handle gracefully
            }
            _ => {
                // Other expressions: try to find nested calls
                collect_nested_calls(&expr, source, current, &mut self.cfg);
            }
        }
    }

    /// Helper: process either a statement_block or a single statement.
    fn process_block_or_statement(
        &mut self,
        node: &Node,
        current: NodeIndex,
        source: &[u8],
    ) -> Vec<NodeIndex> {
        if node.kind() == "statement_block" {
            self.process_block(node, current, source)
        } else {
            self.process_statement(node, current, source)
        }
    }
}

// ── Helper functions ──

/// Try to interpret a value node as a call expression and produce the
/// appropriate CfgStatementKind. Handles:
/// - Direct calls: `foo(a, b)`
/// - `new` expressions: `new Foo(a)`
/// - `.then()` chains: `fetch().then(cb)` -> Call to `then`
/// - `await expr`: unwrap to inner call
/// - Resource patterns: `new Stream()`, `open()`, etc.
fn try_extract_call_or_resource(
    value: &Node,
    source: &[u8],
    target: &str,
) -> Option<CfgStatementKind> {
    let node = unwrap_await(value);

    match node.kind() {
        "call_expression" | "new_expression" => {
            let (name, args) = extract_call_info(&node, source);

            // Detect resource acquire patterns
            if is_resource_acquire(&name, node.kind() == "new_expression") {
                return Some(CfgStatementKind::ResourceAcquire {
                    target: target.to_string(),
                    resource_type: name,
                });
            }

            // Detect resource release patterns
            if is_resource_release(&name) {
                return Some(CfgStatementKind::ResourceRelease {
                    target: target.to_string(),
                    resource_type: name,
                });
            }

            Some(CfgStatementKind::Call { name, args })
        }
        _ => None,
    }
}

/// Unwrap `await_expression` to get the inner expression.
fn unwrap_await<'a>(node: &Node<'a>) -> Node<'a> {
    if node.kind() == "await_expression" {
        node.named_child(0).unwrap_or(*node)
    } else {
        *node
    }
}

/// Extract call name and argument variables from a call_expression or new_expression.
fn extract_call_info(node: &Node, source: &[u8]) -> (String, Vec<String>) {
    let name = node
        .child_by_field_name("function")
        .or_else(|| {
            // new_expression: first child is "new", second is the constructor
            if node.kind() == "new_expression" {
                // Try named children — skip "new" keyword
                let mut cursor = node.walk();
                node.named_children(&mut cursor)
                    .find(|c| c.kind() != "arguments")
            } else {
                None
            }
        })
        .map(|n| extract_callable_name(&n, source))
        .unwrap_or_default();

    let args = node
        .child_by_field_name("arguments")
        .map(|args_node| extract_vars(&args_node, source))
        .unwrap_or_default();

    (name, args)
}

/// Extract the name from a callable expression (identifier, member_expression, etc.).
fn extract_callable_name(node: &Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" | "property_identifier" | "shorthand_property_identifier" => {
            node.utf8_text(source).unwrap_or("").to_string()
        }
        "member_expression" => {
            // For a.b.c, we want "c" as the method name
            // but for `.then()` we want "then"
            node.child_by_field_name("property")
                .and_then(|p| p.utf8_text(source).ok())
                .unwrap_or_else(|| {
                    // Fallback: full text with last segment
                    let text = node.utf8_text(source).unwrap_or("");
                    text.rsplit('.').next().unwrap_or(text)
                })
                .to_string()
        }
        // `new Foo()` — the type name
        "type_identifier" | "nested_identifier" => node.utf8_text(source).unwrap_or("").to_string(),
        _ => node.utf8_text(source).unwrap_or("").to_string(),
    }
}

/// Extract an identifier name from a node (handles identifier, member_expression).
fn extract_identifier_text(node: &Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "property_identifier" | "shorthand_property_identifier" => {
            Some(node.utf8_text(source).ok()?.to_string())
        }
        "member_expression" => {
            // Take the last property: `a.b.c` -> `c`
            let prop = node.child_by_field_name("property")?;
            Some(prop.utf8_text(source).ok()?.to_string())
        }
        "subscript_expression" => {
            // `a[b]` -> `a`
            let obj = node.child_by_field_name("object")?;
            extract_identifier_text(&obj, source)
        }
        _ => {
            // Fallback: try the raw text if it looks like an identifier
            let text = node.utf8_text(source).ok()?.trim().to_string();
            if text
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                && !text.is_empty()
            {
                Some(text)
            } else {
                None
            }
        }
    }
}

/// Recursively extract all variable references (identifiers) from an expression node.
fn extract_vars(node: &Node, source: &[u8]) -> Vec<String> {
    let mut vars = Vec::new();
    collect_vars_recursive(node, source, &mut vars);
    vars.sort();
    vars.dedup();
    vars
}

fn collect_vars_recursive(node: &Node, source: &[u8], vars: &mut Vec<String>) {
    match node.kind() {
        "identifier" => {
            if let Ok(text) = node.utf8_text(source) {
                let text = text.trim();
                // Skip keywords that tree-sitter may classify as identifiers
                if !text.is_empty() && !is_js_keyword(text) {
                    vars.push(text.to_string());
                }
            }
        }
        "property_identifier" | "shorthand_property_identifier" => {
            if let Ok(text) = node.utf8_text(source) {
                let text = text.trim();
                if !text.is_empty() {
                    vars.push(text.to_string());
                }
            }
        }
        "member_expression" => {
            // For `a.b`, extract `a` as a variable (the object being accessed)
            if let Some(obj) = node.child_by_field_name("object") {
                collect_vars_recursive(&obj, source, vars);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_vars_recursive(&child, source, vars);
            }
        }
    }
}

/// Extract variables from direct children of a node (non-recursive, shallow).
fn extract_vars_from_children(node: &Node, source: &[u8]) -> Vec<String> {
    let mut vars = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_vars_recursive(&child, source, &mut vars);
    }
    vars.sort();
    vars.dedup();
    vars
}

/// Collect nested call expressions from an arbitrary expression.
fn collect_nested_calls(node: &Node, source: &[u8], block: NodeIndex, cfg: &mut FunctionCfg) {
    match node.kind() {
        "call_expression" | "new_expression" => {
            let (name, args) = extract_call_info(node, source);
            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::Call { name, args },
                line: node.start_position().row as u32 + 1,
            });
        }
        "await_expression" => {
            if let Some(inner) = node.named_child(0) {
                collect_nested_calls(&inner, source, block, cfg);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_nested_calls(&child, source, block, cfg);
            }
        }
    }
}

/// Check if a call name looks like a resource acquisition.
fn is_resource_acquire(name: &str, is_new: bool) -> bool {
    if is_new {
        let lower = name.to_lowercase();
        return lower.contains("stream")
            || lower.contains("socket")
            || lower.contains("connection")
            || lower.contains("reader")
            || lower.contains("writer")
            || lower.contains("client")
            || lower.contains("pool");
    }
    matches!(
        name,
        "open"
            | "createReadStream"
            | "createWriteStream"
            | "connect"
            | "createConnection"
            | "createServer"
            | "createPool"
            | "fopen"
    )
}

/// Check if a call name looks like a resource release.
fn is_resource_release(name: &str) -> bool {
    matches!(
        name,
        "close"
            | "destroy"
            | "end"
            | "release"
            | "disconnect"
            | "dispose"
            | "unref"
            | "shutdown"
            | "fclose"
            | "free"
    )
}

/// Extract a parameter name from a formal_parameters child node and push it
/// into `out`. Handles plain identifiers, typed/optional parameters (TS), and
/// rest parameters (`...args`).
fn extract_param_name(node: &Node, source: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                let name = name.trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
            }
        }
        // TypeScript: required_parameter, optional_parameter, rest_parameter
        // The pattern identifier is always the first named child.
        "required_parameter" | "optional_parameter" | "rest_parameter" => {
            if let Some(inner) = node.named_child(0)
                && inner.kind() == "identifier"
                && let Ok(name) = inner.utf8_text(source)
            {
                let name = name.trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
            }
        }
        // JavaScript rest element: `...args`
        "spread_element" => {
            if let Some(inner) = node.named_child(0)
                && inner.kind() == "identifier"
                && let Ok(name) = inner.utf8_text(source)
            {
                let name = name.trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
            }
        }
        // Assignment pattern for default parameters: `x = default`
        "assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left")
                && left.kind() == "identifier"
                && let Ok(name) = left.utf8_text(source)
            {
                let name = name.trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
            }
        }
        _ => {}
    }
}

/// Check if a string is a JS/TS keyword (to filter out from variable extraction).
fn is_js_keyword(s: &str) -> bool {
    matches!(
        s,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "void"
            | "typeof"
            | "instanceof"
            | "new"
            | "delete"
            | "this"
            | "super"
            | "class"
            | "function"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "default"
            | "break"
            | "continue"
            | "throw"
            | "try"
            | "catch"
            | "finally"
            | "const"
            | "let"
            | "var"
            | "import"
            | "export"
            | "from"
            | "async"
            | "await"
            | "yield"
            | "in"
            | "of"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_build(code: &str) -> FunctionCfg {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();

        // Find the first function node
        let func = find_first_function(root).expect("no function found in test code");
        let builder = TypeScriptCfgBuilder;
        builder.build_cfg(&func, code.as_bytes()).unwrap()
    }

    fn find_first_function(node: Node) -> Option<Node> {
        match node.kind() {
            "function_declaration" | "method_definition" | "arrow_function" => {
                return Some(node);
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_first_function(child) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn test_simple_function() {
        let cfg = parse_and_build("function foo() { const x = 1; return x; }");
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        // Smoke test: CFG built without panicking
    }

    #[test]
    fn test_if_else_branches() {
        let cfg =
            parse_and_build("function foo(x) { if (x > 0) { return 1; } else { return -1; } }");
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
    }

    #[test]
    fn test_while_loop() {
        let cfg =
            parse_and_build("function foo() { let i = 0; while (i < 10) { i++; } return i; }");
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
    }

    #[test]
    fn test_try_catch() {
        let cfg = parse_and_build(
            "function foo() { try { doSomething(); } catch (e) { handleError(e); } }",
        );
        // Should have an Exception edge
        let has_exception_edge = cfg
            .blocks
            .edge_indices()
            .any(|e| matches!(cfg.blocks.edge_weight(e), Some(CfgEdge::Exception)));
        assert!(has_exception_edge, "should have an Exception edge");
    }

    #[test]
    fn test_switch_case() {
        let cfg = parse_and_build(
            r#"function foo(x) {
                switch (x) {
                    case 1: return "one";
                    case 2: return "two";
                    default: return "other";
                }
            }"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
    }

    #[test]
    fn test_call_expression() {
        let cfg = parse_and_build("function foo() { console.log('hello'); bar(1, 2); }");
        // Entry block should have Call statements
        let entry = cfg.entry;
        let stmts = &cfg.blocks[entry].statements;
        let has_call = stmts
            .iter()
            .any(|s| matches!(&s.kind, CfgStatementKind::Call { .. }));
        assert!(
            has_call,
            "should have Call statements for console.log and bar"
        );
    }

    #[test]
    fn test_arrow_function() {
        let cfg = parse_and_build("const foo = (x) => { return x + 1; }");
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
    }

    #[test]
    fn test_for_loop() {
        let cfg = parse_and_build("function foo() { for (let i = 0; i < 10; i++) { doWork(i); } }");
        // Should have a FalseBranch edge (loop exit)
        let has_false_edge = cfg
            .blocks
            .edge_indices()
            .any(|e| matches!(cfg.blocks.edge_weight(e), Some(CfgEdge::FalseBranch)));
        assert!(has_false_edge, "for loop should have FalseBranch exit edge");
    }

    #[test]
    fn test_do_while() {
        let cfg = parse_and_build("function foo() { let i = 0; do { i++; } while (i < 10); }");
        let has_true_edge = cfg
            .blocks
            .edge_indices()
            .any(|e| matches!(cfg.blocks.edge_weight(e), Some(CfgEdge::TrueBranch)));
        assert!(has_true_edge, "do-while should have TrueBranch back edge");
    }
}
