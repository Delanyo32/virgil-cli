use anyhow::Result;
use petgraph::graph::NodeIndex;
use tree_sitter::Node;

use crate::languages::cfg::CfgBuilder;
use crate::graph::cfg::{BasicBlock, CfgEdge, CfgStatement, CfgStatementKind, FunctionCfg};

/// CFG builder for C#: if/else, for/foreach/while, switch, try/catch/finally,
/// using_statement (dispose as ResourceRelease), return.
pub struct CSharpCfgBuilder;

impl CfgBuilder for CSharpCfgBuilder {
    fn build_cfg(&self, function_node: &Node, source: &[u8]) -> Result<FunctionCfg> {
        let mut cfg = FunctionCfg::new();

        // Extract parameter names from the parameter_list.
        // C#: method/constructor declarations have a "parameters" field containing
        // a parameter_list; each parameter child has a "name" field (identifier).
        if let Some(params_node) = function_node.child_by_field_name("parameters") {
            let mut cursor = params_node.walk();
            for child in params_node.named_children(&mut cursor) {
                if child.kind() == "parameter"
                    && let Some(name_node) = child.child_by_field_name("name")
                {
                    let name = name_node.utf8_text(source).unwrap_or("").to_string();
                    if !name.is_empty() {
                        cfg.param_names.push(name);
                    }
                }
            }
        }

        let body = find_block(function_node);
        let body = match body {
            Some(b) => b,
            None => {
                // Arrow-bodied member: treat as single expression
                if let Some(arrow_body) = function_node.child_by_field_name("body") {
                    let entry = cfg.entry;
                    emit_any_statement(&mut cfg, entry, &arrow_body, source);
                }
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

fn find_block<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|&child| child.kind() == "block")
}

/// Process a block and return the last live block index,
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
            "for_each_statement" => {
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

            // ── using statement (IDisposable) ───────────────────────
            "using_statement" => match build_using(cfg, current, &child, source) {
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
            "throw_statement" => {
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

            // ── local declarations ──────────────────────────────────
            "local_declaration_statement" => {
                emit_local_declaration(cfg, current, &child, source);
            }

            // ── expression statements ───────────────────────────────
            "expression_statement" => {
                emit_expression_stmt(cfg, current, &child, source);
            }

            // ── nested blocks ───────────────────────────────────────
            "block" => match build_block(cfg, current, &child, source) {
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

    // True branch (consequence)
    let true_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks
        .add_edge(current, true_block, CfgEdge::TrueBranch);

    let consequence = node.child_by_field_name("consequence");
    let true_exit = match consequence {
        Some(cons) if cons.kind() == "block" => build_block(cfg, true_block, &cons, source),
        Some(cons) => {
            emit_any_statement(cfg, true_block, &cons, source);
            Some(true_block)
        }
        None => Some(true_block),
    };

    // False branch (alternative)
    let false_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks
        .add_edge(current, false_block, CfgEdge::FalseBranch);

    let alternative = node.child_by_field_name("alternative");
    let false_exit = match alternative {
        Some(alt) if alt.kind() == "else_clause" => {
            let body = alt.named_child(0);
            match body {
                Some(b) if b.kind() == "block" => build_block(cfg, false_block, &b, source),
                Some(b) if b.kind() == "if_statement" => build_if(cfg, false_block, &b, source),
                Some(b) => {
                    emit_any_statement(cfg, false_block, &b, source);
                    Some(false_block)
                }
                None => Some(false_block),
            }
        }
        _ => Some(false_block),
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
        Some(b) if b.kind() == "block" => build_block(cfg, body_block, &b, source),
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

    // Guard on the collection expression
    let collection_vars = node
        .child_by_field_name("right")
        .map(|r| collect_identifiers(&r, source))
        .unwrap_or_default();

    cfg.blocks[header].statements.push(CfgStatement {
        kind: CfgStatementKind::Guard {
            condition_vars: collection_vars,
        },
        line: line_of(node),
    });

    let body_block = cfg.blocks.add_node(BasicBlock::new());
    cfg.blocks.add_edge(header, body_block, CfgEdge::TrueBranch);

    // Loop variable assignment
    if let Some(left) = node.child_by_field_name("left") {
        let target = left.utf8_text(source).unwrap_or_default().to_string();
        if !target.is_empty() {
            cfg.blocks[body_block].statements.push(CfgStatement {
                kind: CfgStatementKind::Assignment {
                    target,
                    source_vars: vec![],
                },
                line: line_of(&left),
            });
        }
    }

    let body = node.child_by_field_name("body");
    let body_exit = match body {
        Some(b) if b.kind() == "block" => build_block(cfg, body_block, &b, source),
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
        Some(b) if b.kind() == "block" => build_block(cfg, body_block, &b, source),
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
        .child_by_field_name("value")
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

    // C# switch cases: switch_section nodes
    let mut cursor = body.walk();
    for section in body.children(&mut cursor) {
        if section.kind() != "switch_section" {
            continue;
        }

        // Check if this is a default section
        let mut label_cursor = section.walk();
        for label_child in section.children(&mut label_cursor) {
            if label_child.kind() == "default_switch_label" {
                has_default = true;
                break;
            }
        }

        let case_block = cfg.blocks.add_node(BasicBlock::new());
        cfg.blocks
            .add_edge(current, case_block, CfgEdge::TrueBranch);

        // Process statements inside the switch section
        let mut case_current = case_block;
        let mut terminated = false;
        let mut section_cursor = section.walk();
        for stmt in section.children(&mut section_cursor) {
            match stmt.kind() {
                "break_statement" => {
                    cfg.blocks.add_edge(case_current, join, CfgEdge::Normal);
                    terminated = true;
                    break;
                }
                "return_statement" => {
                    let vars = collect_identifiers(&stmt, source);
                    cfg.blocks[case_current].statements.push(CfgStatement {
                        kind: CfgStatementKind::Return { value_vars: vars },
                        line: line_of(&stmt),
                    });
                    cfg.exits.push(case_current);
                    terminated = true;
                    break;
                }
                "throw_statement" => {
                    let vars = collect_identifiers(&stmt, source);
                    cfg.blocks[case_current].statements.push(CfgStatement {
                        kind: CfgStatementKind::Call {
                            name: "throw".to_string(),
                            args: vars,
                        },
                        line: line_of(&stmt),
                    });
                    terminated = true;
                    break;
                }
                "expression_statement" => {
                    emit_expression_stmt(cfg, case_current, &stmt, source);
                }
                "local_declaration_statement" => {
                    emit_local_declaration(cfg, case_current, &stmt, source);
                }
                "block" => match build_block(cfg, case_current, &stmt, source) {
                    Some(c) => case_current = c,
                    None => {
                        terminated = true;
                        break;
                    }
                },
                _ => {}
            }
        }

        if !terminated {
            // C# requires explicit break/return/throw in switch sections,
            // but handle fallthrough gracefully
            cfg.blocks.add_edge(case_current, join, CfgEdge::Normal);
        }
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
        Some(b) if b.kind() == "block" => build_block(cfg, try_block, &b, source),
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

            // Extract catch parameter
            if let Some(decl) = child.child_by_field_name("parameters") {
                let vars = collect_identifiers(&decl, source);
                if let Some(target) = vars.first() {
                    cfg.blocks[catch_block].statements.push(CfgStatement {
                        kind: CfgStatementKind::Assignment {
                            target: target.clone(),
                            source_vars: vec![],
                        },
                        line: line_of(&child),
                    });
                }
            }

            let catch_body = child.child_by_field_name("body");
            let catch_exit = match catch_body {
                Some(b) if b.kind() == "block" => build_block(cfg, catch_block, &b, source),
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

            let finally_body = child.named_child(0);
            let finally_exit = match finally_body {
                Some(b) if b.kind() == "block" => build_block(cfg, finally_block, &b, source),
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

// ── using statement (IDisposable) ──────────────────────────────────────────

fn build_using(
    cfg: &mut FunctionCfg,
    current: NodeIndex,
    node: &Node,
    source: &[u8],
) -> Option<NodeIndex> {
    // Extract resource variable from the using declaration
    let target = node
        .child_by_field_name("declaration")
        .or_else(|| {
            // using (var x = ...) form
            let mut c = node.walk();
            node.children(&mut c)
                .find(|ch| ch.kind() == "variable_declaration")
        })
        .and_then(|decl| {
            // Try to find the declarator name
            let mut c = decl.walk();
            for child in decl.children(&mut c) {
                if (child.kind() == "variable_declarator" || child.kind() == "variable_declaration")
                    && let Some(name) = child.child_by_field_name("name")
                {
                    return name.utf8_text(source).ok().map(|s| s.to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| "resource".to_string());

    // Emit resource acquire
    cfg.blocks[current].statements.push(CfgStatement {
        kind: CfgStatementKind::ResourceAcquire {
            target: target.clone(),
            resource_type: "IDisposable".to_string(),
        },
        line: line_of(node),
    });

    // Process the body
    let body = node.child_by_field_name("body");
    let body_exit = match body {
        Some(b) if b.kind() == "block" => build_block(cfg, current, &b, source),
        Some(b) => {
            emit_any_statement(cfg, current, &b, source);
            Some(current)
        }
        None => Some(current),
    };

    // Emit resource release (Dispose) at scope exit
    match body_exit {
        Some(exit) => {
            let release_block = cfg.blocks.add_node(BasicBlock::new());
            cfg.blocks.add_edge(exit, release_block, CfgEdge::Cleanup);
            cfg.blocks[release_block].statements.push(CfgStatement {
                kind: CfgStatementKind::ResourceRelease {
                    target,
                    resource_type: "Dispose".to_string(),
                },
                line: line_of(node),
            });
            Some(release_block)
        }
        None => None,
    }
}

// ── Statement emission ─────────────────────────────────────────────────────

fn emit_local_declaration(cfg: &mut FunctionCfg, block: NodeIndex, node: &Node, source: &[u8]) {
    // local_declaration_statement > variable_declaration > variable_declarator
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declaration" {
            let mut decl_cursor = child.walk();
            for declarator in child.children(&mut decl_cursor) {
                if declarator.kind() == "variable_declarator" {
                    let target = declarator
                        .child_by_field_name("name")
                        .or_else(|| {
                            let mut c = declarator.walk();
                            declarator
                                .children(&mut c)
                                .find(|ch| ch.kind() == "identifier")
                        })
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or_default()
                        .to_string();

                    let value = declarator
                        .child_by_field_name("value")
                        .or_else(|| declarator.child_by_field_name("initializer"));

                    let source_vars = value
                        .map(|v| collect_identifiers(&v, source))
                        .unwrap_or_default();

                    if !target.is_empty() {
                        cfg.blocks[block].statements.push(CfgStatement {
                            kind: CfgStatementKind::Assignment {
                                target,
                                source_vars,
                            },
                            line: line_of(&declarator),
                        });
                    }
                }
            }
        }
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
        "invocation_expression" => {
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
        "object_creation_expression" => {
            let type_name = expr
                .child_by_field_name("type")
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
        "assignment_expression" => {
            let target = expr
                .child_by_field_name("left")
                .and_then(|l| l.utf8_text(source).ok())
                .unwrap_or_default()
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
        "postfix_unary_expression" | "prefix_unary_expression" => {
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
        "await_expression" => {
            // Unwrap await to its inner expression
            if let Some(inner) = expr.named_child(0) {
                emit_expression(cfg, block, &inner, source);
            }
        }
        _ => {}
    }
}

fn emit_any_statement(cfg: &mut FunctionCfg, block: NodeIndex, node: &Node, source: &[u8]) {
    match node.kind() {
        "expression_statement" => emit_expression_stmt(cfg, block, node, source),
        "local_declaration_statement" => emit_local_declaration(cfg, block, node, source),
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
