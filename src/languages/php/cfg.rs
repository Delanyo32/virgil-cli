use anyhow::Result;
use petgraph::graph::NodeIndex;
use tree_sitter::Node;

use crate::languages::cfg::CfgBuilder;
use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

/// CFG builder for PHP: if/else, for/foreach/while, switch, try/catch/finally,
/// return. Handles function_call_expression and method_call_expression.
pub struct PhpCfgBuilder;

impl CfgBuilder for PhpCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let mut cfg = FunctionCfg::new();

        // Extract parameter names from the function signature.
        // PHP formal_parameters contains simple_parameter (or variadic_parameter) nodes,
        // each of which has a "name" child that is a variable_name node (e.g. "$data").
        // We strip the leading "$" so "data" matches PARAM_PATTERNS in the taint engine.
        if let Some(params_node) = function_node.child_by_field_name("parameters") {
            let mut cursor = params_node.walk();
            for child in params_node.named_children(&mut cursor) {
                // Both simple_parameter and variadic_parameter have a "name" field.
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node
                        .utf8_text(source)
                        .unwrap_or("")
                        .trim_start_matches('$')
                        .to_string();
                    if !name.is_empty() {
                        cfg.param_names.push(name);
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

/// Process a compound_statement and return the last live block index,
/// or None if every path terminates.
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
            "foreach_statement" => {
                current = build_foreach(cfg, current, &child, source);
            }
            "do_statement" => {
                current = build_do_while(cfg, current, &child, source);
            }

            // ── switch ──────────────────────────────────────────────
            "switch_statement" => match build_switch(cfg, current, &child, source) {
                Some(join) => current = join,
                None => return None,
            },

            // ── try/catch/finally ───────────────────────────────────
            "try_statement" => match build_try_catch(cfg, current, &child, source) {
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
                return None;
            }

            // ── throw ───────────────────────────────────────────────
            "throw_expression" | "throw_statement" => {
                let vars = collect_identifiers(&child, source);
                cfg.blocks[current].statements.push(CfgStatement {
                    kind: CfgStatementKind::Call {
                        name: "throw".to_string(),
                        args: vars,
                    },
                    line: line_of(&child),
                });
                return None;
            }

            // ── expression statements ───────────────────────────────
            "expression_statement" => {
                emit_expression_stmt(cfg, current, &child, source);
            }

            // ── nested blocks ───────────────────────────────────────
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

    // True branch (body)
    let true_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks
        .add_edge(current, true_block, CfgEdge::TrueBranch);

    let body = node.child_by_field_name("body");
    let true_exit = match body {
        Some(b) if b.kind() == "compound_statement" => build_block(cfg, true_block, &b, source),
        Some(b) if b.kind() == "colon_block" => build_colon_block(cfg, true_block, &b, source),
        Some(b) => {
            emit_any_statement(cfg, true_block, &b, source);
            Some(true_block)
        }
        None => Some(true_block),
    };

    // False branch (alternative: else / elseif)
    let false_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks
        .add_edge(current, false_block, CfgEdge::FalseBranch);

    let alternative = node.child_by_field_name("alternative");
    let false_exit = match alternative {
        Some(alt) => match alt.kind() {
            "else_clause" => {
                let body = alt.named_child(0);
                match body {
                    Some(b) if b.kind() == "compound_statement" => {
                        build_block(cfg, false_block, &b, source)
                    }
                    Some(b) if b.kind() == "colon_block" => {
                        build_colon_block(cfg, false_block, &b, source)
                    }
                    Some(b) => {
                        emit_any_statement(cfg, false_block, &b, source);
                        Some(false_block)
                    }
                    None => Some(false_block),
                }
            }
            "else_if_clause" => build_if(cfg, false_block, &alt, source),
            _ => Some(false_block),
        },
        None => Some(false_block),
    };

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

/// Handle PHP alternative syntax (if/while/for with colons).
fn build_colon_block(
    cfg: &mut FunctionCfg,
    current: NodeIndex,
    node: &Node,
    source: &[u8],
) -> Option<NodeIndex> {
    // Colon blocks contain statements directly (same as compound_statement)
    build_block(cfg, current, node, source)
}

// ── Loops (for / while) ────────────────────────────────────────────────────

fn build_loop(cfg: &mut FunctionCfg, current: NodeIndex, node: &Node, source: &[u8]) -> NodeIndex {
    let header = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(current, header, CfgEdge::Normal);

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

    let body_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(header, body_block, CfgEdge::TrueBranch);

    let body = node.child_by_field_name("body");
    let body_exit = match body {
        Some(b) if b.kind() == "compound_statement" => build_block(cfg, body_block, &b, source),
        Some(b) if b.kind() == "colon_block" => build_colon_block(cfg, body_block, &b, source),
        Some(b) => {
            emit_any_statement(cfg, body_block, &b, source);
            Some(body_block)
        }
        None => Some(body_block),
    };

    if let Some(be) = body_exit {
        cfg.blocks.add_edge(be, header, CfgEdge::Normal);
    }

    let exit = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(header, exit, CfgEdge::FalseBranch);
    exit
}

// ── foreach ────────────────────────────────────────────────────────────────

fn build_foreach(
    cfg: &mut FunctionCfg,
    current: NodeIndex,
    node: &Node,
    source: &[u8],
) -> NodeIndex {
    let header = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(current, header, CfgEdge::Normal);

    // Guard on the collection
    let collection_vars = node
        .named_children(&mut node.walk())
        .find(|c| c.kind() == "parenthesized_expression" || c.kind() == "binary_expression")
        .map(|c| collect_identifiers(&c, source))
        .unwrap_or_default();

    cfg.blocks[header].statements.push(CfgStatement {
        kind: CfgStatementKind::Guard {
            condition_vars: collection_vars,
        },
        line: line_of(node),
    });

    let body_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(header, body_block, CfgEdge::TrueBranch);

    // PHP foreach value variable: extract $value from `foreach ($arr as $key => $value)`
    // The value and key are stored as children with specific node kinds
    let mut var_cursor = node.walk();
    for child in node.named_children(&mut var_cursor) {
        if child.kind() == "variable_name" || child.kind() == "pair" {
            let target = child
                .utf8_text(source)
                .unwrap_or_default()
                .trim_start_matches('$')
                .to_string();
            if !target.is_empty() {
                cfg.blocks[body_block].statements.push(CfgStatement {
                    kind: CfgStatementKind::Assignment {
                        target,
                        source_vars: vec![],
                    },
                    line: line_of(&child),
                });
            }
        }
    }

    let body = node.child_by_field_name("body");
    let body_exit = match body {
        Some(b) if b.kind() == "compound_statement" => build_block(cfg, body_block, &b, source),
        Some(b) if b.kind() == "colon_block" => build_colon_block(cfg, body_block, &b, source),
        Some(b) => {
            emit_any_statement(cfg, body_block, &b, source);
            Some(body_block)
        }
        None => Some(body_block),
    };

    if let Some(be) = body_exit {
        cfg.blocks.add_edge(be, header, CfgEdge::Normal);
    }

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

    cfg.blocks
        .add_edge(cond_block, body_block, CfgEdge::TrueBranch);

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
        let is_case = match case.kind() {
            "switch_case" | "case_statement" => true,
            "switch_default" | "default_statement" => {
                has_default = true;
                true
            }
            _ => false,
        };

        if !is_case {
            continue;
        }

        let case_block = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks
            .add_edge(current, case_block, CfgEdge::TrueBranch);

        if let Some(prev) = prev_fallthrough {
            cfg.blocks.add_edge(prev, case_block, CfgEdge::Normal);
        }

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
                "throw_expression" | "throw_statement" => {
                    let vars = collect_identifiers(&stmt, source);
                    cfg.blocks[case_current].statements.push(CfgStatement {
                        kind: CfgStatementKind::Call {
                            name: "throw".to_string(),
                            args: vars,
                        },
                        line: line_of(&stmt),
                    });
                    broke = true;
                    break;
                }
                "expression_statement" => {
                    emit_expression_stmt(cfg, case_current, &stmt, source);
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
            prev_fallthrough = Some(case_current);
        } else {
            prev_fallthrough = None;
        }
    }

    if let Some(ft) = prev_fallthrough {
        cfg.blocks.add_edge(ft, join, CfgEdge::Normal);
    }

    if !has_default {
        cfg.blocks.add_edge(current, join, CfgEdge::FalseBranch);
    }

    Some(join)
}

// ── Try / catch / finally ──────────────────────────────────────────────────

fn build_try_catch(
    cfg: &mut FunctionCfg,
    current: NodeIndex,
    node: &Node,
    source: &[u8],
) -> Option<NodeIndex> {
    let try_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(current, try_block, CfgEdge::Normal);

    let try_body = node.child_by_field_name("body");
    let try_exit = match try_body {
        Some(b) if b.kind() == "compound_statement" => build_block(cfg, try_block, &b, source),
        _ => Some(try_block),
    };

    let join = cfg.blocks.add_node(BasicBlock::new());

    if let Some(te) = try_exit {
        cfg.blocks.add_edge(te, join, CfgEdge::Normal);
    }

    // Catch clauses
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "catch_clause" {
            let catch_block = cfg.blocks.add_node(BasicBlock::new());
            cfg.blocks
                .add_edge(try_block, catch_block, CfgEdge::Exception);

            // Extract catch parameter (e.g., `catch (Exception $e)`)
            // PHP catch has type and name as fields
            if let Some(name_node) = child.child_by_field_name("name") {
                let target = name_node
                    .utf8_text(source)
                    .unwrap_or_default()
                    .trim_start_matches('$')
                    .to_string();
                if !target.is_empty() {
                    cfg.blocks[catch_block].statements.push(CfgStatement {
                        kind: CfgStatementKind::Assignment {
                            target,
                            source_vars: vec![],
                        },
                        line: line_of(&child),
                    });
                }
            }

            let catch_body = child.child_by_field_name("body");
            let catch_exit = match catch_body {
                Some(b) if b.kind() == "compound_statement" => {
                    build_block(cfg, catch_block, &b, source)
                }
                _ => Some(catch_block),
            };

            if let Some(ce) = catch_exit {
                cfg.blocks.add_edge(ce, join, CfgEdge::Normal);
            }
        }
    }

    // Finally clause
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        if child.kind() == "finally_clause" {
            let finally_block = cfg.blocks.add_node(BasicBlock::new());
            cfg.blocks.add_edge(join, finally_block, CfgEdge::Cleanup);

            let finally_body = child
                .child_by_field_name("body")
                .or_else(|| child.named_child(0));
            let finally_exit = match finally_body {
                Some(b) if b.kind() == "compound_statement" => {
                    build_block(cfg, finally_block, &b, source)
                }
                _ => Some(finally_block),
            };

            if let Some(fe) = finally_exit {
                let after_finally = cfg.blocks.add_node(BasicBlock::new());
                cfg.blocks.add_edge(fe, after_finally, CfgEdge::Normal);
                return Some(after_finally);
            }
            return None;
        }
    }

    Some(join)
}

// ── Statement emission ─────────────────────────────────────────────────────

fn emit_expression_stmt(cfg: &mut FunctionCfg, block: NodeIndex, node: &Node, source: &[u8]) {
    let expr = match node.named_child(0) {
        Some(e) => e,
        None => return,
    };
    emit_expression(cfg, block, &expr, source);
}

fn emit_expression(cfg: &mut FunctionCfg, block: NodeIndex, expr: &Node, source: &[u8]) {
    match expr.kind() {
        "function_call_expression" => {
            let name = expr
                .child_by_field_name("function")
                .and_then(|f| f.utf8_text(source).ok())
                .unwrap_or_default()
                .to_string();

            let args = expr
                .child_by_field_name("arguments")
                .map(|a| collect_identifiers(&a, source))
                .unwrap_or_default();

            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::Call { name, args },
                line: line_of(expr),
            });
        }
        "method_call_expression" | "member_call_expression" => {
            let name = expr
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .unwrap_or_default()
                .to_string();

            let args = expr
                .child_by_field_name("arguments")
                .map(|a| collect_identifiers(&a, source))
                .unwrap_or_default();

            // Include the object as first arg for data flow tracking
            let mut all_args = Vec::new();
            if let Some(obj) = expr.child_by_field_name("object") {
                let obj_text = obj
                    .utf8_text(source)
                    .unwrap_or_default()
                    .trim_start_matches('$')
                    .to_string();
                if !obj_text.is_empty() {
                    all_args.push(obj_text);
                }
            }
            all_args.extend(args);

            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::Call {
                    name,
                    args: all_args,
                },
                line: line_of(expr),
            });
        }
        "scoped_call_expression" => {
            // Static method calls like ClassName::method()
            let name = expr.utf8_text(source).unwrap_or_default().to_string();

            let args = expr
                .child_by_field_name("arguments")
                .map(|a| collect_identifiers(&a, source))
                .unwrap_or_default();

            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::Call { name, args },
                line: line_of(expr),
            });
        }
        "object_creation_expression" => {
            let type_name = expr
                .child_by_field_name("type")
                .or_else(|| {
                    let mut c = expr.walk();
                    expr.children(&mut c)
                        .find(|ch| ch.kind() == "name" || ch.kind() == "qualified_name")
                })
                .and_then(|t| t.utf8_text(source).ok())
                .unwrap_or("unknown")
                .to_string();

            cfg.blocks[block].statements.push(CfgStatement {
                kind: CfgStatementKind::Call {
                    name: format!("new {}", type_name),
                    args: expr
                        .child_by_field_name("arguments")
                        .map(|a| collect_identifiers(&a, source))
                        .unwrap_or_default(),
                },
                line: line_of(expr),
            });
        }
        "assignment_expression" | "augmented_assignment_expression" => {
            let target = expr
                .child_by_field_name("left")
                .and_then(|l| l.utf8_text(source).ok())
                .unwrap_or_default()
                .trim_start_matches('$')
                .to_string();

            let source_vars = expr
                .child_by_field_name("right")
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
        "update_expression" => {
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
        _ => {}
    }
}

fn emit_any_statement(cfg: &mut FunctionCfg, block: NodeIndex, node: &Node, source: &[u8]) {
    match node.kind() {
        "expression_statement" => emit_expression_stmt(cfg, block, node, source),
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

// ── Utility ────────────────────────────────────────────────────────────────

fn collect_identifiers(node: &Node, source: &[u8]) -> Vec<String> {
    let mut ids = Vec::new();
    collect_identifiers_rec(node, source, &mut ids);
    ids
}

fn collect_identifiers_rec(node: &Node, source: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "variable_name" | "name" => {
            if let Ok(text) = node.utf8_text(source) {
                let s = text.trim_start_matches('$').to_string();
                if !s.is_empty() && !out.contains(&s) {
                    out.push(s);
                }
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers_rec(&child, source, out);
    }
}

fn line_of(node: &Node) -> u32 {
    node.start_position().row as u32 + 1
}
