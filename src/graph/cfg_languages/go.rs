use anyhow::Result;
use tree_sitter::Node;

use super::CfgBuilder;
use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};
use petgraph::graph::NodeIndex;

pub struct GoCfgBuilder;

impl CfgBuilder for GoCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let mut builder = GoFunctionBuilder::new(source);

        // Extract parameter names from the function signature.
        // Go uses a "parameters" field on function_declaration/method_declaration.
        // Each child is a parameter_declaration (name + type) or
        // variadic_parameter_declaration.
        if let Some(params_node) = function_node.child_by_field_name("parameters") {
            let mut cursor = params_node.walk();
            for child in params_node.named_children(&mut cursor) {
                match child.kind() {
                    "parameter_declaration" | "variadic_parameter_declaration" => {
                        // The "name" field may be a single identifier or an
                        // identifier_list for multi-name params (a, b int).
                        if let Some(name_node) = child.child_by_field_name("name") {
                            match name_node.kind() {
                                "identifier" => {
                                    let name =
                                        name_node.utf8_text(source).unwrap_or("").to_string();
                                    if !name.is_empty() && name != "_" {
                                        builder.cfg.param_names.push(name);
                                    }
                                }
                                "identifier_list" => {
                                    let mut id_cursor = name_node.walk();
                                    for ident in name_node.named_children(&mut id_cursor) {
                                        if ident.kind() == "identifier" {
                                            let name =
                                                ident.utf8_text(source).unwrap_or("").to_string();
                                            if !name.is_empty() && name != "_" {
                                                builder.cfg.param_names.push(name);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Find the function body block
        let body = function_node
            .child_by_field_name("body")
            .ok_or_else(|| anyhow::anyhow!("Go function has no body"))?;

        let entry = builder.cfg.entry;
        builder.process_block(&body, entry, None, None);

        // Any block with no outgoing edges that is not already in exits becomes an exit
        let all_nodes: Vec<NodeIndex> = builder.cfg.blocks.node_indices().collect();
        for idx in all_nodes {
            let has_outgoing = builder
                .cfg
                .blocks
                .edges_directed(idx, petgraph::Direction::Outgoing)
                .next()
                .is_some();
            if !has_outgoing && !builder.cfg.exits.contains(&idx) {
                builder.cfg.exits.push(idx);
            }
        }

        // If the entry block has no statements and no successors, mark it as an exit
        // (empty function body)
        if builder.cfg.exits.is_empty() {
            builder.cfg.exits.push(builder.cfg.entry);
        }

        Ok(builder.cfg)
    }
}

/// Internal builder state for constructing a Go function's CFG.
struct GoFunctionBuilder<'a> {
    cfg: FunctionCfg,
    source: &'a [u8],
    /// Deferred statements collected in order (defer uses LIFO, but we just
    /// record them and attach cleanup edges from return/exit blocks).
    deferred: Vec<NodeIndex>,
}

impl<'a> GoFunctionBuilder<'a> {
    fn new(source: &'a [u8]) -> Self {
        Self {
            cfg: FunctionCfg::new(),
            source,
            deferred: Vec::new(),
        }
    }

    fn add_block(&mut self) -> NodeIndex {
        self.cfg.blocks.add_node(BasicBlock::new())
    }

    fn add_stmt(&mut self, block: NodeIndex, stmt: CfgStatement) {
        self.cfg.blocks[block].statements.push(stmt);
    }

    fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, edge: CfgEdge) {
        self.cfg.blocks.add_edge(from, to, edge);
    }

    fn node_text(&self, node: &Node) -> String {
        node.utf8_text(self.source).unwrap_or("").to_string()
    }

    fn line(&self, node: &Node) -> u32 {
        node.start_position().row as u32 + 1
    }

    /// Process a block (sequence of statements). Returns the last active block,
    /// or None if control flow has terminated (return, etc).
    ///
    /// `break_target` and `continue_target` are set when inside a for loop.
    fn process_block(
        &mut self,
        block_node: &Node,
        mut current: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) -> Option<NodeIndex> {
        let mut cursor = block_node.walk();
        for child in block_node.named_children(&mut cursor) {
            match self.process_statement(&child, current, break_target, continue_target) {
                Some(next) => current = next,
                None => return None, // control terminated
            }
        }
        Some(current)
    }

    /// Process a single statement. Returns the block to continue from,
    /// or None if control flow terminated.
    fn process_statement(
        &mut self,
        node: &Node,
        current: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) -> Option<NodeIndex> {
        match node.kind() {
            "if_statement" => self.process_if(node, current, break_target, continue_target),
            "for_statement" => self.process_for(node, current),
            "expression_switch_statement" => {
                self.process_switch(node, current, break_target, continue_target)
            }
            "type_switch_statement" => {
                self.process_switch(node, current, break_target, continue_target)
            }
            "select_statement" => self.process_select(node, current, break_target, continue_target),
            "return_statement" => self.process_return(node, current),
            "defer_statement" => self.process_defer(node, current),
            "go_statement" => self.process_go(node, current),
            "short_var_declaration" => {
                self.process_short_var_decl(node, current);
                Some(current)
            }
            "assignment_statement" => {
                self.process_assignment(node, current);
                Some(current)
            }
            "expression_statement" => {
                self.process_expression_stmt(node, current);
                Some(current)
            }
            "var_declaration" => {
                self.process_var_decl(node, current);
                Some(current)
            }
            "block" => self.process_block(node, current, break_target, continue_target),
            "break_statement" => {
                if let Some(target) = break_target {
                    self.add_edge(current, target, CfgEdge::Normal);
                }
                None
            }
            "continue_statement" => {
                if let Some(target) = continue_target {
                    self.add_edge(current, target, CfgEdge::Normal);
                }
                None
            }
            "goto_statement" | "fallthrough_statement" => {
                // Best-effort: treat as normal flow continuation
                Some(current)
            }
            "labeled_statement" => {
                // Process the inner statement
                let mut child_cursor = node.walk();
                for child in node.named_children(&mut child_cursor) {
                    if child.kind() != "label_name" {
                        return self.process_statement(
                            &child,
                            current,
                            break_target,
                            continue_target,
                        );
                    }
                }
                Some(current)
            }
            _ => {
                // For any other node that contains a call expression, try to
                // extract it (e.g., standalone function calls in expression position).
                if let Some(call) = find_call_expression(node) {
                    let (name, args) = extract_call_info(&call, self.source);
                    self.add_stmt(
                        current,
                        CfgStatement {
                            kind: CfgStatementKind::Call { name, args },
                            line: self.line(node),
                        },
                    );
                }
                Some(current)
            }
        }
    }

    // ── if_statement ──

    fn process_if(
        &mut self,
        node: &Node,
        current: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) -> Option<NodeIndex> {
        // Extract condition variables
        let condition_vars = node
            .child_by_field_name("condition")
            .map(|c| extract_identifiers(&c, self.source))
            .unwrap_or_default();

        // If there is an initializer (if x := foo(); x > 0), process it first
        if let Some(init) = node.child_by_field_name("initializer") {
            self.process_statement(&init, current, break_target, continue_target);
        }

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard { condition_vars },
                line: self.line(node),
            },
        );

        let merge = self.add_block();

        // True branch (consequence)
        let true_block = self.add_block();
        self.add_edge(current, true_block, CfgEdge::TrueBranch);
        if let Some(consequence) = node.child_by_field_name("consequence") {
            if let Some(end) =
                self.process_block(&consequence, true_block, break_target, continue_target)
            {
                self.add_edge(end, merge, CfgEdge::Normal);
            }
        } else {
            self.add_edge(true_block, merge, CfgEdge::Normal);
        }

        // False branch (alternative) -- could be else block or else-if
        if let Some(alternative) = node.child_by_field_name("alternative") {
            let false_block = self.add_block();
            self.add_edge(current, false_block, CfgEdge::FalseBranch);
            if alternative.kind() == "if_statement" {
                // else-if chain
                if let Some(end) =
                    self.process_if(&alternative, false_block, break_target, continue_target)
                {
                    self.add_edge(end, merge, CfgEdge::Normal);
                }
            } else {
                // else block
                if let Some(end) =
                    self.process_block(&alternative, false_block, break_target, continue_target)
                {
                    self.add_edge(end, merge, CfgEdge::Normal);
                }
            }
        } else {
            // No else -- false branch goes directly to merge
            self.add_edge(current, merge, CfgEdge::FalseBranch);
        }

        Some(merge)
    }

    // ── for_statement ──

    fn process_for(&mut self, node: &Node, current: NodeIndex) -> Option<NodeIndex> {
        // for [init]; [condition]; [update] { body }
        // Also handles: for { ... }, for condition { ... }, for range ...

        // Process initializer if present
        if let Some(init) = node.child_by_field_name("initializer") {
            // init is processed in current block
            self.process_statement(&init, current, None, None);
        }

        let header = self.add_block();
        self.add_edge(current, header, CfgEdge::Normal);

        // Condition guard
        let condition_vars = node
            .child_by_field_name("condition")
            .map(|c| extract_identifiers(&c, self.source))
            .unwrap_or_default();

        if !condition_vars.is_empty() {
            self.add_stmt(
                header,
                CfgStatement {
                    kind: CfgStatementKind::Guard { condition_vars },
                    line: self.line(node),
                },
            );
        }

        let after_loop = self.add_block();
        let body_block = self.add_block();

        // Header -> body (true) and header -> after (false)
        self.add_edge(header, body_block, CfgEdge::TrueBranch);
        self.add_edge(header, after_loop, CfgEdge::FalseBranch);

        // For infinite loops (no condition), still link to body
        // The FalseBranch edge handles the case where the condition is empty
        // (compiler treats it as always true, but we keep the edge for analysis).

        // Process body with break/continue targets
        if let Some(body) = node.child_by_field_name("body")
            && let Some(end) = self.process_block(&body, body_block, Some(after_loop), Some(header))
        {
            // Process update if present
            if let Some(update) = node.child_by_field_name("update") {
                self.process_statement(&update, end, None, None);
            }
            // Back edge to header
            self.add_edge(end, header, CfgEdge::Normal);
        }

        Some(after_loop)
    }

    // ── expression_switch_statement / type_switch_statement ──

    fn process_switch(
        &mut self,
        node: &Node,
        current: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) -> Option<NodeIndex> {
        // Extract switch value
        let condition_vars = node
            .child_by_field_name("value")
            .map(|c| extract_identifiers(&c, self.source))
            .unwrap_or_default();

        // Switch initializer (switch x := expr; x { ... })
        if let Some(init) = node.child_by_field_name("initializer") {
            self.process_statement(&init, current, break_target, continue_target);
        }

        if !condition_vars.is_empty() {
            self.add_stmt(
                current,
                CfgStatement {
                    kind: CfgStatementKind::Guard { condition_vars },
                    line: self.line(node),
                },
            );
        }

        let merge = self.add_block();
        let mut has_default = false;

        // Iterate over case clauses (expression_case, default_case, type_case)
        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            match child.kind() {
                "expression_case" | "type_case" => {
                    let case_block = self.add_block();
                    // Each case gets a TrueBranch from the switch header
                    self.add_edge(current, case_block, CfgEdge::TrueBranch);

                    // Process the case body statements
                    let end =
                        self.process_case_body(&child, case_block, Some(merge), continue_target);
                    if let Some(end) = end {
                        self.add_edge(end, merge, CfgEdge::Normal);
                    }
                }
                "default_case" => {
                    has_default = true;
                    let default_block = self.add_block();
                    self.add_edge(current, default_block, CfgEdge::FalseBranch);

                    let end =
                        self.process_case_body(&child, default_block, Some(merge), continue_target);
                    if let Some(end) = end {
                        self.add_edge(end, merge, CfgEdge::Normal);
                    }
                }
                _ => {}
            }
        }

        // If no default case, the switch can fall through directly
        if !has_default {
            self.add_edge(current, merge, CfgEdge::FalseBranch);
        }

        Some(merge)
    }

    /// Process the body statements inside a case/default clause.
    fn process_case_body(
        &mut self,
        clause_node: &Node,
        mut current: NodeIndex,
        break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) -> Option<NodeIndex> {
        let mut child_cursor = clause_node.walk();
        let mut skip_value = true; // skip the case value expression(s)
        for child in clause_node.named_children(&mut child_cursor) {
            // In expression_case, the first named children are the case values,
            // followed by body statements. We identify body statements as those
            // that are NOT expression_list nodes at the start.
            if skip_value
                && matches!(
                    child.kind(),
                    "expression_list" | "type_identifier" | "qualified_type" | "pointer_type"
                )
            {
                continue;
            }
            skip_value = false;

            match self.process_statement(&child, current, break_target, continue_target) {
                Some(next) => current = next,
                None => return None,
            }
        }
        Some(current)
    }

    // ── select_statement ──

    fn process_select(
        &mut self,
        node: &Node,
        current: NodeIndex,
        _break_target: Option<NodeIndex>,
        continue_target: Option<NodeIndex>,
    ) -> Option<NodeIndex> {
        // select is like switch but for channel operations
        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Guard {
                    condition_vars: vec!["<select>".to_string()],
                },
                line: self.line(node),
            },
        );

        let merge = self.add_block();
        let mut has_default = false;

        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            match child.kind() {
                "communication_case" => {
                    let case_block = self.add_block();
                    self.add_edge(current, case_block, CfgEdge::TrueBranch);

                    // The communication clause contains a send/receive operation
                    // followed by body statements
                    let end =
                        self.process_case_body(&child, case_block, Some(merge), continue_target);
                    if let Some(end) = end {
                        self.add_edge(end, merge, CfgEdge::Normal);
                    }
                }
                "default_case" => {
                    has_default = true;
                    let default_block = self.add_block();
                    self.add_edge(current, default_block, CfgEdge::FalseBranch);

                    let end =
                        self.process_case_body(&child, default_block, Some(merge), continue_target);
                    if let Some(end) = end {
                        self.add_edge(end, merge, CfgEdge::Normal);
                    }
                }
                _ => {}
            }
        }

        if !has_default {
            // select without default blocks until a case is ready;
            // for CFG purposes, add a fallthrough edge
            self.add_edge(current, merge, CfgEdge::FalseBranch);
        }

        Some(merge)
    }

    // ── return_statement ──

    fn process_return(&mut self, node: &Node, current: NodeIndex) -> Option<NodeIndex> {
        let value_vars = node
            .child_by_field_name("expression_list")
            .map(|el| extract_identifiers(&el, self.source))
            .or_else(|| {
                // Return values may be direct children
                let mut vars = Vec::new();
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    vars.extend(extract_identifiers(&child, self.source));
                }
                if vars.is_empty() { None } else { Some(vars) }
            })
            .unwrap_or_default();

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Return { value_vars },
                line: self.line(node),
            },
        );

        // Add cleanup edges to deferred blocks (LIFO order)
        let deferred: Vec<NodeIndex> = self.deferred.iter().rev().copied().collect();
        for deferred_block in deferred {
            self.add_edge(current, deferred_block, CfgEdge::Cleanup);
        }

        self.cfg.exits.push(current);
        None // control terminates
    }

    // ── defer_statement ──

    fn process_defer(&mut self, node: &Node, current: NodeIndex) -> Option<NodeIndex> {
        // Create a separate block for the deferred call
        let defer_block = self.add_block();

        // Extract the deferred call
        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            if child.kind() == "call_expression" {
                let (name, args) = extract_call_info(&child, self.source);
                self.add_stmt(
                    defer_block,
                    CfgStatement {
                        kind: CfgStatementKind::Call { name, args },
                        line: self.line(node),
                    },
                );
                break;
            }
            // defer might wrap a method call or other expression
            if let Some(call) = find_call_expression(&child) {
                let (name, args) = extract_call_info(&call, self.source);
                self.add_stmt(
                    defer_block,
                    CfgStatement {
                        kind: CfgStatementKind::Call { name, args },
                        line: self.line(node),
                    },
                );
                break;
            }
        }

        self.deferred.push(defer_block);

        // In the current block, record the defer registration as a Cleanup edge
        self.add_edge(current, defer_block, CfgEdge::Cleanup);

        Some(current)
    }

    // ── go_statement ──

    fn process_go(&mut self, node: &Node, current: NodeIndex) -> Option<NodeIndex> {
        // go func() -- spawns a goroutine, record as a Call
        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            if child.kind() == "call_expression" {
                let (name, args) = extract_call_info(&child, self.source);
                self.add_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Call {
                            name: format!("go {}", name),
                            args,
                        },
                        line: self.line(node),
                    },
                );
                return Some(current);
            }
        }

        // Fallback: record the whole go statement text
        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Call {
                    name: "go <unknown>".to_string(),
                    args: vec![],
                },
                line: self.line(node),
            },
        );
        Some(current)
    }

    // ── short_var_declaration (:=) ──

    fn process_short_var_decl(&mut self, node: &Node, current: NodeIndex) {
        let target = node
            .child_by_field_name("left")
            .map(|n| self.node_text(&n))
            .unwrap_or_default();

        let source_vars = node
            .child_by_field_name("right")
            .map(|n| extract_identifiers(&n, self.source))
            .unwrap_or_default();

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target,
                    source_vars,
                },
                line: self.line(node),
            },
        );
    }

    // ── assignment_statement (=, +=, etc.) ──

    fn process_assignment(&mut self, node: &Node, current: NodeIndex) {
        let target = node
            .child_by_field_name("left")
            .map(|n| self.node_text(&n))
            .unwrap_or_default();

        let source_vars = node
            .child_by_field_name("right")
            .map(|n| extract_identifiers(&n, self.source))
            .unwrap_or_default();

        self.add_stmt(
            current,
            CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target,
                    source_vars,
                },
                line: self.line(node),
            },
        );
    }

    // ── expression_statement (standalone calls, etc.) ──

    fn process_expression_stmt(&mut self, node: &Node, current: NodeIndex) {
        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            if child.kind() == "call_expression" {
                let (name, args) = extract_call_info(&child, self.source);
                self.add_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Call { name, args },
                        line: self.line(node),
                    },
                );
                return;
            }
        }
        // Not a call -- check if there's a nested call anywhere
        if let Some(call) = find_call_expression(node) {
            let (name, args) = extract_call_info(&call, self.source);
            self.add_stmt(
                current,
                CfgStatement {
                    kind: CfgStatementKind::Call { name, args },
                    line: self.line(node),
                },
            );
        }
    }

    // ── var_declaration ──

    fn process_var_decl(&mut self, node: &Node, current: NodeIndex) {
        let mut child_cursor = node.walk();
        for child in node.named_children(&mut child_cursor) {
            if child.kind() == "var_spec" {
                let target = child
                    .child_by_field_name("name")
                    .map(|n| self.node_text(&n))
                    .unwrap_or_default();

                let source_vars = child
                    .child_by_field_name("value")
                    .map(|n| extract_identifiers(&n, self.source))
                    .unwrap_or_default();

                self.add_stmt(
                    current,
                    CfgStatement {
                        kind: CfgStatementKind::Assignment {
                            target,
                            source_vars,
                        },
                        line: self.line(node),
                    },
                );
            }
        }
    }
}

// ── Helpers ──

/// Extract identifier names from an expression subtree.
fn extract_identifiers(node: &Node, source: &[u8]) -> Vec<String> {
    let mut ids = Vec::new();
    collect_identifiers(node, source, &mut ids);
    ids
}

fn collect_identifiers(node: &Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            let s = text.to_string();
            if !s.is_empty() && !is_go_keyword(&s) {
                out.push(s);
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers(&child, source, out);
    }
}

fn is_go_keyword(s: &str) -> bool {
    matches!(
        s,
        "break"
            | "case"
            | "chan"
            | "const"
            | "continue"
            | "default"
            | "defer"
            | "else"
            | "fallthrough"
            | "for"
            | "func"
            | "go"
            | "goto"
            | "if"
            | "import"
            | "interface"
            | "map"
            | "package"
            | "range"
            | "return"
            | "select"
            | "struct"
            | "switch"
            | "type"
            | "var"
            | "nil"
            | "true"
            | "false"
    )
}

/// Recursively find the first call_expression in a subtree.
fn find_call_expression<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "call_expression" {
        return Some(*node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_call_expression(&child) {
            return Some(found);
        }
    }
    None
}

/// Extract function name and argument identifiers from a call_expression.
fn extract_call_info(call_node: &Node, source: &[u8]) -> (String, Vec<String>) {
    let name = call_node
        .child_by_field_name("function")
        .map(|n| {
            let text = n.utf8_text(source).unwrap_or("");
            // For selector expressions like pkg.Func, keep the full text
            text.to_string()
        })
        .unwrap_or_else(|| "<unknown>".to_string());

    let args = call_node
        .child_by_field_name("arguments")
        .map(|args_node| extract_identifiers(&args_node, source))
        .unwrap_or_default();

    (name, args)
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::parser::create_parser;

    fn build_go_cfg(source: &str) -> FunctionCfg {
        let mut parser = create_parser(Language::Go).expect("create parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse");

        // Find the first function_declaration or method_declaration
        let root = tree.root_node();
        let func_node = find_function_node(root).expect("should find a function");

        let builder = GoCfgBuilder;
        builder
            .build_cfg(&func_node, source.as_bytes())
            .expect("build cfg")
    }

    fn find_function_node(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        if node.kind() == "function_declaration" || node.kind() == "method_declaration" {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_function_node(child) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn simple_function() {
        let cfg = build_go_cfg(
            r#"
package main
func hello() {
    x := 1
    y := x + 2
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn if_statement() {
        let cfg = build_go_cfg(
            r#"
package main
func test() {
    x := 1
    if x > 0 {
        x = 2
    } else {
        x = 3
    }
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn for_loop() {
        let cfg = build_go_cfg(
            r#"
package main
func test() {
    for i := 0; i < 10; i++ {
        fmt.Println(i)
    }
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn return_statement() {
        let cfg = build_go_cfg(
            r#"
package main
func add(a, b int) int {
    return a + b
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn defer_statement() {
        let cfg = build_go_cfg(
            r#"
package main
func test() {
    f := os.Open("file.txt")
    defer f.Close()
    process(f)
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn go_statement() {
        let cfg = build_go_cfg(
            r#"
package main
func test() {
    go handleRequest(conn)
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn switch_statement() {
        let cfg = build_go_cfg(
            r#"
package main
func test(x int) {
    switch x {
    case 1:
        fmt.Println("one")
    case 2:
        fmt.Println("two")
    default:
        fmt.Println("other")
    }
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn select_statement() {
        let cfg = build_go_cfg(
            r#"
package main
func test(ch1, ch2 chan int) {
    select {
    case v := <-ch1:
        process(v)
    case ch2 <- 42:
        done()
    }
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn if_else_if_chain() {
        let cfg = build_go_cfg(
            r#"
package main
func test(x int) {
    if x > 10 {
        big()
    } else if x > 5 {
        medium()
    } else {
        small()
    }
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn empty_function() {
        let cfg = build_go_cfg(
            r#"
package main
func empty() {}
"#,
        );
        assert!(!cfg.exits.is_empty());
    }

    #[test]
    fn assignment_statement() {
        let cfg = build_go_cfg(
            r#"
package main
func test() {
    x := 1
    x = x + 2
}
"#,
        );
        assert!(
            cfg.blocks.node_count() >= 1,
            "should have at least one block"
        );
        assert!(!cfg.exits.is_empty(), "should have at least one exit");
    }

    #[test]
    fn call_expression() {
        let cfg = build_go_cfg(
            r#"
package main
func test() {
    fmt.Println("hello")
}
"#,
        );
        let has_call = cfg.blocks[cfg.entry].statements.iter().any(
            |s| matches!(&s.kind, CfgStatementKind::Call { name, .. } if name == "fmt.Println"),
        );
        assert!(has_call);
    }
}
