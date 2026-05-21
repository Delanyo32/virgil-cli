//! Issue #16 — shared minimal references-emitter helper for the
//! non-Rust, non-TS languages. Mirrors the Rust pilot's structure but
//! parameterized by per-language node-kind matchers.
//!
//! Coverage:
//! - File scope at the tree root.
//! - Function/class/block scopes per the language's matcher.
//! - One `definition` binding per `SymbolInfo` in the file scope.
//! - Occurrences for identifier-shaped nodes in non-declaring
//!   positions: `call`, `write`, `read`, `type_use`, `import_use` per
//!   the language's classifier.
//!
//! This is the MVP shape. Language-specific binding kinds beyond
//! `definition` (parameter / import / import_alias / wildcard_import)
//! land in follow-ups; the resolver still produces unresolved
//! `references` rows for now.

use tree_sitter::{Node, Tree};

use crate::models::{BindingRow, OccurrenceRow, ReferencesBucket, ScopeRow, SymbolInfo};

/// Per-language matchers used by `emit_minimal_references`.
pub struct LangRefs {
    /// Node-kind names that open a `function` scope.
    pub function_scope_kinds: &'static [&'static str],
    /// Node-kind names that open a `class` scope.
    pub class_scope_kinds: &'static [&'static str],
    /// Node-kind names that open a `block` scope.
    pub block_scope_kinds: &'static [&'static str],
    /// Identifier-shaped leaf node kinds.
    pub identifier_kinds: &'static [&'static str],
    /// Type-position identifier kinds (subset of identifier_kinds).
    pub type_identifier_kinds: &'static [&'static str],
    /// Parent node kinds whose children that match identifier_kinds
    /// represent declarations (skip occurrence emission).
    pub declaration_parents: &'static [&'static str],
    /// Parent node kind of a call-expression style construct.
    pub call_parents: &'static [&'static str],
    /// Parent node kind of an assignment-style construct.
    pub assignment_parents: &'static [&'static str],
    /// Parent node kind of an import-statement style construct.
    pub import_parents: &'static [&'static str],
}

pub fn emit_minimal_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
    cfg: &LangRefs,
) -> ReferencesBucket {
    let mut bucket = ReferencesBucket::default();

    // 1. File scope.
    let root = tree.root_node();
    let file_scope_id = format!("{}|{}|file", file_path, root.start_byte());
    bucket.scopes.push(ScopeRow {
        id: file_scope_id.clone(),
        parent_id: None,
        file_path: file_path.to_string(),
        kind: "file".to_string(),
        start_byte: root.start_byte() as u32,
        end_byte: root.end_byte() as u32,
    });

    // 2. Definition bindings (one per symbol, all in file scope).
    let mut symbol_spans: Vec<(u32, u32, String)> = symbols
        .iter()
        .map(|s| {
            let id = format!(
                "{}|{}|{}|{}|{}",
                s.file_path, s.start_line, s.start_column, s.name, s.kind
            );
            (s.start_byte, s.end_byte, id)
        })
        .collect();
    symbol_spans.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    for sym in symbols {
        bucket.bindings.push(BindingRow {
            scope_id: file_scope_id.clone(),
            name: sym.name.clone(),
            start_byte: sym.start_byte,
            symbol_id: Some(format!(
                "{}|{}|{}|{}|{}",
                sym.file_path, sym.start_line, sym.start_column, sym.name, sym.kind
            )),
            binding_kind: "definition".to_string(),
        });
    }

    // 3. Walk for scopes + occurrences.
    walk(
        root,
        &file_scope_id,
        file_path,
        source,
        cfg,
        &symbol_spans,
        &mut bucket,
    );

    bucket
}

fn walk(
    node: Node,
    scope_id: &str,
    file_path: &str,
    source: &[u8],
    cfg: &LangRefs,
    symbol_spans: &[(u32, u32, String)],
    bucket: &mut ReferencesBucket,
) {
    let kind = node.kind();
    let new_scope_kind = if cfg.function_scope_kinds.contains(&kind) {
        Some("function")
    } else if cfg.class_scope_kinds.contains(&kind) {
        Some("class")
    } else if cfg.block_scope_kinds.contains(&kind) {
        Some("block")
    } else {
        None
    };
    let active_scope = if let Some(sk) = new_scope_kind {
        let id = format!("{}|{}|{}", file_path, node.start_byte(), sk);
        bucket.scopes.push(ScopeRow {
            id: id.clone(),
            parent_id: Some(scope_id.to_string()),
            file_path: file_path.to_string(),
            kind: sk.to_string(),
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        });
        id
    } else {
        scope_id.to_string()
    };

    if cfg.identifier_kinds.contains(&kind)
        && let Some(occ_kind) = classify_occurrence(node, cfg)
        && let Ok(name) = node.utf8_text(source)
        && !name.is_empty()
    {
        let start = node.start_byte() as u32;
        let id = format!("{}|{}|{}|{}", file_path, start, name, occ_kind);
        let enclosing = enclosing_symbol(symbol_spans, start);
        bucket.occurrences.push(OccurrenceRow {
            id,
            name: name.to_string(),
            file_path: file_path.to_string(),
            start_byte: start,
            end_byte: node.end_byte() as u32,
            enclosing_symbol_id: enclosing,
            enclosing_scope_id: active_scope.clone(),
            occurrence_kind: occ_kind.to_string(),
        });
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(
            child,
            &active_scope,
            file_path,
            source,
            cfg,
            symbol_spans,
            bucket,
        );
    }
}

fn classify_occurrence(node: Node, cfg: &LangRefs) -> Option<&'static str> {
    let Some(parent) = node.parent() else {
        return Some("read");
    };
    let pk = parent.kind();
    // Declaring positions — skip.
    if cfg.declaration_parents.contains(&pk) {
        // Skip if THIS node is the parent's `name` field.
        if parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id()) {
            return None;
        }
    }
    // Import statements → import_use.
    if cfg.import_parents.contains(&pk) {
        return Some("import_use");
    }
    // Type position.
    if cfg.type_identifier_kinds.contains(&node.kind()) {
        return Some("type_use");
    }
    // Call position.
    if cfg.call_parents.contains(&pk) {
        let field = parent
            .child_by_field_name("function")
            .or_else(|| parent.child_by_field_name("name"))
            .or_else(|| parent.child_by_field_name("method"));
        if field.map(|n| n.id()) == Some(node.id()) {
            return Some("call");
        }
    }
    // Assignment LHS → write.
    if cfg.assignment_parents.contains(&pk)
        && parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id())
    {
        return Some("write");
    }
    Some("read")
}

fn enclosing_symbol(spans: &[(u32, u32, String)], byte: u32) -> Option<String> {
    let mut hit: Option<&str> = None;
    for (start, end, id) in spans {
        if *start <= byte && byte <= *end {
            hit = Some(id.as_str());
        } else if *start > byte {
            break;
        }
    }
    hit.map(|s| s.to_string())
}
