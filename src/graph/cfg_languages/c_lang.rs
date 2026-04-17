use anyhow::Result;
use petgraph::graph::NodeIndex;
use tree_sitter::Node;

use super::CfgBuilder;
use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

/// CFG builder for C: if/else, for/while/do, switch with fallthrough,
/// return, variable declarations, call_expression, malloc/free, fopen/fclose.
/// Best-effort goto handling (skipped).
pub struct CCfgBuilder;

impl CfgBuilder for CCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let mut cfg = FunctionCfg::new();

        // Extract parameter names from the function signature.
        // In C's grammar, function_definition has a `declarator` field which is a
        // `function_declarator`. That node has a `parameters` field with a `parameter_list`.
        // Each child of the list is a `parameter_declaration` whose `declarator` field
        // may be a `pointer_declarator` (wrapping further declarators) or a plain `identifier`.
        if let Some(params_node) = find_c_parameter_list(function_node) {
            let mut cursor = params_node.walk();
            for child in params_node.named_children(&mut cursor) {
                if child.kind() == "parameter_declaration" {
                    if let Some(declarator) = child.child_by_field_name("declarator") {
                        let name = extract_c_param_ident(&declarator, source);
                        if !name.is_empty() {
                            cfg.param_names.push(name);
                        }
                    }
                }
            }
        }

        let body = find_compound_statement(function_node);
        let body = match body {
            Some(b) => b,
            None => {
                cfg.exits.push(cfg.entry);
                return Ok(cfg);
            }
        };

        let entry = cfg.entry;
        let exit = build_block(&mut cfg, entry, &body, source);
        if let Some(exit_idx) = exit {
            cfg.exits.push(exit_idx);
        }

        // Deduplicate exits
        cfg.exits.sort_by_key(|n| n.index());
        cfg.exits.dedup();

        Ok(cfg)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_compound_statement<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|&child| child.kind() == "compound_statement")
}

/// Find the `parameter_list` node for a C `function_definition`.
/// Structure: function_definition.declarator (function_declarator).parameters (parameter_list).
fn find_c_parameter_list<'a>(function_node: &Node<'a>) -> Option<Node<'a>> {
    // Walk the declarator chain to find a function_declarator, then get its parameters field.
    let declarator = function_node.child_by_field_name("declarator")?;
    find_function_declarator_params(&declarator)
}

/// Recursively descend through declarator wrappers (pointer_declarator, etc.)
/// to find a `function_declarator` and return its `parameters` field.
fn find_function_declarator_params<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "function_declarator" {
        return node.child_by_field_name("parameters");
    }
    // pointer_declarator and similar wrappers have a `declarator` field
    if let Some(inner) = node.child_by_field_name("declarator") {
        return find_function_declarator_params(&inner);
    }
    None
}

/// Process a compound_statement (or block) and return the last live block index,
/// or None if every path returned / broke.
fn build_block(
    cfg: &mut FunctionCfg,
    mut current: NodeIndex,
    block_node: &Node,
    source: &[u8],
) -> Option<NodeIndex> {
    let mut cursor = block_node.walk();
    for child in block_node.children(&mut cursor) {
        match child.kind() {
            // ── branching ───────────────────────────────────────────
            "if_statement" => match build_if(cfg, current, &child, source) {
                Some(join) => current = join,
                None => return None,
            },

            // ── loops ───────────────────────────────────────────────
            "for_statement" | "while_statement" => {
                current = build_loop(cfg, current, &child, source);
            }
            "do_statement" => {
                current = build_do_while(cfg, current, &child, source);
            }

            // ── switch ──────────────────────────────────────────────
            "switch_statement" => match build_switch(cfg, current, &child, source) {
                Some(join) => current = join,
                None => return None,
            },

            // ── return ──────────────────────────────────────────────
            "return_statement" => {
                let vars = collect_identifiers(&child, source);
                cfg.blocks[current].statements.push(CfgStatement {
                    kind: CfgStatementKind::Return { value_vars: vars },
                    line: line_of(&child),
                });
                cfg.exits.push(current);
                return None; // path terminates
            }

            // ── goto (best-effort: skip) ────────────────────────────
            "goto_statement" | "labeled_statement" => { /* skip */ }

            // ── declarations & expressions ───────────────────────────
            "declaration" => {
                emit_declaration(cfg, current, &child, source);
            }
            "expression_statement" => {
                emit_expression_stmt(cfg, current, &child, source);
            }

            // ── nested compound_statement (bare blocks) ─────────────
            "compound_statement" => match build_block(cfg, current, &child, source) {
                Some(cont) => current = cont,
                None => return None,
            },

            _ => {}
        }
    }
    Some(current)
}

// ── If / else ──────────────────────────────────────────────────────────────

fn build_if(
    cfg: &mut FunctionCfg,
    current: NodeIndex,
    node: &Node,
    source: &[u8],
) -> Option<NodeIndex> {
    // Emit guard for condition
    let cond_vars = node
        .child_by_field_name("condition")
        .map(|c| collect_identifiers(&c, source))
        .unwrap_or_default();

    cfg.blocks[current].statements.push(CfgStatement {
        kind: CfgStatementKind::Guard {
            condition_vars: cond_vars,
        },
        line: line_of(node),
    });

    let true_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks
        .add_edge(current, true_block, CfgEdge::TrueBranch);

    // Process consequence
    let consequence = node.child_by_field_name("consequence");
    let true_exit = match consequence {
        Some(cons) => {
            if cons.kind() == "compound_statement" {
                build_block(cfg, true_block, &cons, source)
            } else {
                emit_any_statement(cfg, true_block, &cons, source);
                Some(true_block)
            }
        }
        None => Some(true_block),
    };

    // Process alternative (else / else-if)
    let alternative = node.child_by_field_name("alternative");
    let false_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks
        .add_edge(current, false_block, CfgEdge::FalseBranch);

    let false_exit = match alternative {
        Some(alt) => {
            if alt.kind() == "else_clause" {
                // The else clause wraps its body
                let body = alt.named_child(0);
                match body {
                    Some(b) if b.kind() == "compound_statement" => {
                        build_block(cfg, false_block, &b, source)
                    }
                    Some(b) if b.kind() == "if_statement" => build_if(cfg, false_block, &b, source),
                    Some(b) => {
                        emit_any_statement(cfg, false_block, &b, source);
                        Some(false_block)
                    }
                    None => Some(false_block),
                }
            } else {
                Some(false_block)
            }
        }
        None => Some(false_block),
    };

    // Join
    let join = cfg.blocks.add_node(BasicBlock::new());
    if let Some(te) = true_exit {
        cfg.blocks.add_edge(te, join, CfgEdge::Normal);
    }
    if let Some(fe) = false_exit {
        cfg.blocks.add_edge(fe, join, CfgEdge::Normal);
    }
    if true_exit.is_none() && false_exit.is_none() {
        return None;
    }
    Some(join)
}

// ── Loops (for / while) ────────────────────────────────────────────────────

fn build_loop(cfg: &mut FunctionCfg, current: NodeIndex, node: &Node, source: &[u8]) -> NodeIndex {
    let header = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(current, header, CfgEdge::Normal);

    // Guard for condition
    let cond_vars = node
        .child_by_field_name("condition")
        .map(|c| collect_identifiers(&c, source))
        .unwrap_or_default();

    cfg.blocks[header].statements.push(CfgStatement {
        kind: CfgStatementKind::Guard {
            condition_vars: cond_vars,
        },
        line: line_of(node),
    });

    // True branch -> loop body
    let body_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(header, body_block, CfgEdge::TrueBranch);

    let body = node.child_by_field_name("body");
    let body_exit = match body {
        Some(b) if b.kind() == "compound_statement" => build_block(cfg, body_block, &b, source),
        Some(b) => {
            emit_any_statement(cfg, body_block, &b, source);
            Some(body_block)
        }
        None => Some(body_block),
    };

    // Back edge
    if let Some(be) = body_exit {
        cfg.blocks.add_edge(be, header, CfgEdge::Normal);
    }

    // False branch -> exit
    let exit = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(header, exit, CfgEdge::FalseBranch);
    exit
}

// ── do-while ───────────────────────────────────────────────────────────────

fn build_do_while(
    cfg: &mut FunctionCfg,
    current: NodeIndex,
    node: &Node,
    source: &[u8],
) -> NodeIndex {
    let body_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(current, body_block, CfgEdge::Normal);

    let body = node.child_by_field_name("body");
    let body_exit = match body {
        Some(b) if b.kind() == "compound_statement" => build_block(cfg, body_block, &b, source),
        Some(b) => {
            emit_any_statement(cfg, body_block, &b, source);
            Some(body_block)
        }
        None => Some(body_block),
    };

    // Condition check after body
    let cond_block = cfg.blocks.add_node(BasicBlock::new());
    if let Some(be) = body_exit {
        cfg.blocks.add_edge(be, cond_block, CfgEdge::Normal);
    }

    let cond_vars = node
        .child_by_field_name("condition")
        .map(|c| collect_identifiers(&c, source))
        .unwrap_or_default();

    cfg.blocks[cond_block].statements.push(CfgStatement {
        kind: CfgStatementKind::Guard {
            condition_vars: cond_vars,
        },
        line: line_of(node),
    });

    // True -> back to body
    cfg.blocks
        .add_edge(cond_block, body_block, CfgEdge::TrueBranch);

    // False -> exit
    let exit = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(cond_block, exit, CfgEdge::FalseBranch);
    exit
}

// ── Switch ─────────────────────────────────────────────────────────────────

fn build_switch(
    cfg: &mut FunctionCfg,
    current: NodeIndex,
    node: &Node,
    source: &[u8],
) -> Option<NodeIndex> {
    let cond_vars = node
        .child_by_field_name("condition")
        .map(|c| collect_identifiers(&c, source))
        .unwrap_or_default();

    cfg.blocks[current].statements.push(CfgStatement {
        kind: CfgStatementKind::Guard {
            condition_vars: cond_vars,
        },
        line: line_of(node),
    });

    let body = node.child_by_field_name("body");
    let body = match body {
        Some(b) => b,
        None => {
            let exit = cfg.blocks.add_node(BasicBlock::new());
            cfg.blocks.add_edge(current, exit, CfgEdge::Normal);
            return Some(exit);
        }
    };

    let join = cfg.blocks.add_node(BasicBlock::new());
    let mut has_default = false;
    let mut prev_fallthrough: Option<NodeIndex> = None;

    let mut cursor = body.walk();
    for case in body.children(&mut cursor) {
        if case.kind() != "case_statement" {
            continue;
        }

        let is_default = case
            .child(0)
            .map(|c| c.kind() == "default")
            .unwrap_or(false);

        if is_default {
            has_default = true;
        }

        let case_block = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks
            .add_edge(current, case_block, CfgEdge::TrueBranch);

        // Fallthrough from previous case
        if let Some(prev) = prev_fallthrough {
            cfg.blocks.add_edge(prev, case_block, CfgEdge::Normal);
        }

        // Process statements inside case
        let mut case_current = case_block;
        let mut broke = false;
        let mut case_cursor = case.walk();
        for stmt in case.children(&mut case_cursor) {
            match stmt.kind() {
                "break_statement" => {
                    cfg.blocks.add_edge(case_current, join, CfgEdge::Normal);
                    broke = true;
                    break;
                }
                "return_statement" => {
                    let vars = collect_identifiers(&stmt, source);
                    cfg.blocks[case_current].statements.push(CfgStatement {
                        kind: CfgStatementKind::Return { value_vars: vars },
                        line: line_of(&stmt),
                    });
                    cfg.exits.push(case_current);
                    broke = true;
                    break;
                }
                "expression_statement" => {
                    emit_expression_stmt(cfg, case_current, &stmt, source);
                }
                "declaration" => {
                    emit_declaration(cfg, case_current, &stmt, source);
                }
                "compound_statement" => match build_block(cfg, case_current, &stmt, source) {
                    Some(c) => case_current = c,
                    None => {
                        broke = true;
                        break;
                    }
                },
                _ => {}
            }
        }

        if !broke {
            // Fallthrough to next case
            prev_fallthrough = Some(case_current);
        } else {
            prev_fallthrough = None;
        }
    }

    // Last case without break also joins
    if let Some(ft) = prev_fallthrough {
        cfg.blocks.add_edge(ft, join, CfgEdge::Normal);
    }

    // If no default, the switch condition can fall through
    if !has_default {
        cfg.blocks.add_edge(current, join, CfgEdge::FalseBranch);
    }

    Some(join)
}

// ── Statement emission ─────────────────────────────────────────────────────

fn emit_declaration(cfg: &mut FunctionCfg, block: NodeIndex, node: &Node, source: &[u8]) {
    // Check for resource acquisition patterns: malloc, calloc, fopen
    if let Some(init) = node.child_by_field_name("declarator")
        && init.kind() == "init_declarator"
    {
        let target = init
            .child_by_field_name("declarator")
            .and_then(|d| d.utf8_text(source).ok())
            .unwrap_or_default()
            .to_string();
        let value = init.child_by_field_name("value");

        if let Some(val) = value
            && let Some(resource_call) = detect_c_resource_acquire(&val, source)
        {
            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::ResourceAcquire {
                    target: target.clone(),
                    resource_type: resource_call,
                },
                line: line_of(node),
            });
            return;
        }

        let source_vars = value
            .map(|v| collect_identifiers(&v, source))
            .unwrap_or_default();

        if !target.is_empty() {
            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target,
                    source_vars,
                },
                line: line_of(node),
            });
        }
        return;
    }

    // Simple declaration without initializer
    let target = node
        .child_by_field_name("declarator")
        .and_then(|d| d.utf8_text(source).ok())
        .unwrap_or_default()
        .to_string();

    if !target.is_empty() {
        cfg.blocks[block].statements.push(CfgStatement {
            kind: CfgStatementKind::Assignment {
                target,
                source_vars: vec![],
            },
            line: line_of(node),
        });
    }
}

fn emit_expression_stmt(cfg: &mut FunctionCfg, block: NodeIndex, node: &Node, source: &[u8]) {
    let expr = match node.named_child(0) {
        Some(e) => e,
        None => return,
    };
    emit_expression(cfg, block, &expr, source);
}

fn emit_expression(cfg: &mut FunctionCfg, block: NodeIndex, expr: &Node, source: &[u8]) {
    match expr.kind() {
        "call_expression" => {
            let name = expr
                .child_by_field_name("function")
                .and_then(|f| f.utf8_text(source).ok())
                .unwrap_or_default()
                .to_string();

            let args = expr
                .child_by_field_name("arguments")
                .map(|a| collect_identifiers(&a, source))
                .unwrap_or_default();

            // Check for resource release: free, fclose
            if is_c_resource_release(&name) {
                let target = args.first().cloned().unwrap_or_default();
                cfg.blocks[block].statements.push(CfgStatement {
                    kind: CfgStatementKind::ResourceRelease {
                        target,
                        resource_type: name,
                    },
                    line: line_of(expr),
                });
            } else {
                cfg.blocks[block].statements.push(CfgStatement {
                    kind: CfgStatementKind::Call { name, args },
                    line: line_of(expr),
                });
            }
        }
        "assignment_expression" => {
            let target = expr
                .child_by_field_name("left")
                .and_then(|l| l.utf8_text(source).ok())
                .unwrap_or_default()
                .to_string();
            let right = expr.child_by_field_name("right");

            // Check for resource acquire in assignment
            if let Some(ref r) = right
                && let Some(resource_call) = detect_c_resource_acquire(r, source)
            {
                cfg.blocks[block].statements.push(CfgStatement {
                    kind: CfgStatementKind::ResourceAcquire {
                        target,
                        resource_type: resource_call,
                    },
                    line: line_of(expr),
                });
                return;
            }

            let source_vars = right
                .map(|r| collect_identifiers(&r, source))
                .unwrap_or_default();

            if !target.is_empty() {
                cfg.blocks[block].statements.push(CfgStatement {
                    kind: CfgStatementKind::Assignment {
                        target,
                        source_vars,
                    },
                    line: line_of(expr),
                });
            }
        }
        "update_expression" | "unary_expression" => {
            let vars = collect_identifiers(expr, source);
            if let Some(target) = vars.first() {
                cfg.blocks[block].statements.push(CfgStatement {
                    kind: CfgStatementKind::Assignment {
                        target: target.clone(),
                        source_vars: vars.clone(),
                    },
                    line: line_of(expr),
                });
            }
        }
        "comma_expression" => {
            let mut child_cursor = expr.walk();
            for child in expr.children(&mut child_cursor) {
                if child.is_named() {
                    emit_expression(cfg, block, &child, source);
                }
            }
        }
        _ => {}
    }
}

fn emit_any_statement(cfg: &mut FunctionCfg, block: NodeIndex, node: &Node, source: &[u8]) {
    match node.kind() {
        "expression_statement" => emit_expression_stmt(cfg, block, node, source),
        "declaration" => emit_declaration(cfg, block, node, source),
        "return_statement" => {
            let vars = collect_identifiers(node, source);
            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::Return { value_vars: vars },
                line: line_of(node),
            });
            cfg.exits.push(block);
        }
        _ => {}
    }
}

// ── C resource patterns ────────────────────────────────────────────────────

const C_ALLOC_FUNCTIONS: &[&str] = &["malloc", "calloc", "realloc", "fopen", "fdopen", "tmpfile"];
const C_FREE_FUNCTIONS: &[&str] = &["free", "fclose", "close", "pclose"];

fn detect_c_resource_acquire(node: &Node, source: &[u8]) -> Option<String> {
    if node.kind() == "call_expression" {
        let name = node
            .child_by_field_name("function")
            .and_then(|f| f.utf8_text(source).ok())?;
        if C_ALLOC_FUNCTIONS.contains(&name) {
            return Some(name.to_string());
        }
    }
    None
}

fn is_c_resource_release(name: &str) -> bool {
    C_FREE_FUNCTIONS.contains(&name)
}

// ── Utility ────────────────────────────────────────────────────────────────

/// Recursively find the innermost identifier name from a C declarator node.
/// Handles `pointer_declarator` wrappers (e.g. `*args`) and plain `identifier`.
fn extract_c_param_ident(node: &Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => node.utf8_text(source).unwrap_or("").to_string(),
        "pointer_declarator" | "abstract_pointer_declarator" => {
            // The declarator field holds the inner declarator
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_c_param_ident(&inner, source);
            }
            // Fallback: walk named children
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let name = extract_c_param_ident(&child, source);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        _ => {
            // For array_declarator or other wrappers, recurse into named children
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let name = extract_c_param_ident(&child, source);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
    }
}

fn collect_identifiers(node: &Node, source: &[u8]) -> Vec<String> {
    let mut ids = Vec::new();
    collect_identifiers_rec(node, source, &mut ids);
    ids
}

fn collect_identifiers_rec(node: &Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "identifier" {
        if let Ok(text) = node.utf8_text(source) {
            let s = text.to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers_rec(&child, source, out);
    }
}

fn line_of(node: &Node) -> u32 {
    node.start_position().row as u32 + 1
}
