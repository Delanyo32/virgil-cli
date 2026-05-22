//! Issue #16 Rust pilot — `occurrence` / `scope` / `binding` fact
//! emitter per ADR-0005. The Cozoscript resolver
//! (`src/cozo/resolver.rs`) materialises `references` rows from
//! these facts.
//!
//! Scope model:
//! - Every file gets a top-level `file` scope.
//! - `function_item` / `closure_expression` → `function` scope.
//! - `impl_item` / `trait_item` → `class` scope (logical container).
//! - `mod_item` (inline mod) → `module` scope.
//! - `block` → `block` scope.
//!
//! Binding emission:
//! - Every Symbol becomes a `definition` binding in its enclosing scope.
//! - Function parameters become `parameter` bindings in their function
//!   scope.
//! - `use a::b::c;` → `import` binding of `c` in file scope.
//! - `use a::b as d;` → `import_alias` of `d` in file scope.
//! - `use a::*;` → `wildcard_import` row (symbol_id = null) in file scope.
//!
//! Occurrence emission (Level 3):
//! - Every `identifier` / `type_identifier` / `field_identifier` in
//!   non-declaring position emits an occurrence.
//! - `call_expression` / `method_call_expression` callees → `call`.
//! - LHS of assignment_expression → `write`. Everything else
//!   identifier-shaped → `read`.
//! - Identifier in a type-position node → `type_use`.
//! - Identifier inside a `use_declaration` path → `import_use`.

use tree_sitter::{Node, Tree};

use crate::models::{BindingRow, OccurrenceRow, ReferencesBucket, ScopeRow, SymbolInfo};

pub fn extract_references(
    tree: &Tree,
    source: &[u8],
    file_path: &str,
    symbols: &[SymbolInfo],
) -> ReferencesBucket {
    let mut ctx = Ctx::new(file_path, source, symbols);
    let root = tree.root_node();
    let file_scope_id = ctx.push_scope(root, "file", None);
    ctx.emit_definitions(file_scope_id.clone(), symbols);
    ctx.walk(root, &file_scope_id, None);
    ctx.finish()
}

struct Ctx<'a> {
    file_path: &'a str,
    source: &'a [u8],
    bucket: ReferencesBucket,
    /// Sorted `(start_byte, end_byte, symbol_id)` triples used to find
    /// the innermost enclosing symbol of an occurrence.
    symbol_spans: Vec<(u32, u32, String)>,
}

fn symbol_id(sym: &SymbolInfo) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        sym.file_path, sym.start_line, sym.start_column, sym.name, sym.kind
    )
}

impl<'a> Ctx<'a> {
    fn new(file_path: &'a str, source: &'a [u8], symbols: &[SymbolInfo]) -> Self {
        let mut spans = Vec::with_capacity(symbols.len());
        for s in symbols {
            spans.push((s.start_byte, s.end_byte, symbol_id(s)));
        }
        // Sort by start_byte ASC, end_byte DESC so a linear scan finds
        // the innermost enclosing symbol on the last hit.
        spans.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        Self {
            file_path,
            source,
            bucket: ReferencesBucket::default(),
            symbol_spans: spans,
        }
    }

    fn finish(self) -> ReferencesBucket {
        self.bucket
    }

    fn push_scope(&mut self, node: Node, kind: &str, parent: Option<&str>) -> String {
        let id = format!("{}|{}|{}", self.file_path, node.start_byte(), kind);
        self.bucket.scopes.push(ScopeRow {
            id: id.clone(),
            parent_id: parent.map(|s| s.to_string()),
            file_path: self.file_path.to_string(),
            kind: kind.to_string(),
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        });
        id
    }

    /// Find the innermost symbol whose byte range covers `byte`. None for
    /// occurrences outside every symbol (file-level expressions).
    fn enclosing_symbol(&self, byte: u32) -> Option<&str> {
        let mut hit: Option<&str> = None;
        for (start, end, id) in &self.symbol_spans {
            if *start <= byte && byte <= *end {
                hit = Some(id.as_str());
            } else if *start > byte {
                break;
            }
        }
        hit
    }

    /// Pass 1: every Symbol → `definition` binding in file scope.
    /// Functions, structs, enums, traits, etc. all live in the file
    /// scope for resolution purposes (Rust modules use `::` paths which
    /// the resolver doesn't model here; the `imports` relation covers
    /// cross-file resolution).
    fn emit_definitions(&mut self, file_scope_id: String, symbols: &[SymbolInfo]) {
        for sym in symbols {
            self.bucket.bindings.push(BindingRow {
                scope_id: file_scope_id.clone(),
                name: sym.name.clone(),
                start_byte: sym.start_byte,
                symbol_id: Some(symbol_id(sym)),
                binding_kind: "definition".to_string(),
            });
        }
    }

    /// Recursive walk. Tracks the active scope so occurrences land in
    /// the right `enclosing_scope_id`.
    fn walk(&mut self, node: Node, scope_id: &str, enclosing_fn: Option<&str>) {
        // Decide whether this node opens a new scope. If so, descend
        // into children with the new scope_id; if not, pass through.
        let new_scope = match node.kind() {
            "function_item" => Some("function"),
            "closure_expression" => Some("function"),
            "impl_item" | "trait_item" => Some("class"),
            "mod_item" => Some("module"),
            "block" => Some("block"),
            _ => None,
        };

        let active_scope = if let Some(kind) = new_scope {
            self.push_scope(node, kind, Some(scope_id))
        } else {
            scope_id.to_string()
        };

        // Emit bindings unique to this node kind.
        match node.kind() {
            "function_item" => {
                // Parameter bindings: walk parameters → identifier names.
                if let Some(params) = node.child_by_field_name("parameters") {
                    let mut c = params.walk();
                    for p in params.named_children(&mut c) {
                        if let Some(name) = parameter_name(p, self.source) {
                            self.bucket.bindings.push(BindingRow {
                                scope_id: active_scope.clone(),
                                name,
                                start_byte: p.start_byte() as u32,
                                symbol_id: None,
                                binding_kind: "parameter".to_string(),
                            });
                        }
                    }
                }
            }
            "use_declaration" => {
                self.emit_use_bindings(node, scope_id);
            }
            "let_declaration" => {
                if let Some(pat) = node.child_by_field_name("pattern")
                    && let Some(name_node) = innermost_identifier(pat)
                    && let Ok(name) = name_node.utf8_text(self.source)
                {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: active_scope.clone(),
                        name: name.to_string(),
                        start_byte: name_node.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "definition".to_string(),
                    });
                }
            }
            _ => {}
        }

        // Occurrence emission.
        if let Some(kind) = occurrence_kind_for(node) {
            self.emit_occurrence(node, &active_scope, kind, enclosing_fn);
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, &active_scope, enclosing_fn);
        }
    }

    fn emit_occurrence(
        &mut self,
        node: Node,
        scope_id: &str,
        kind: &str,
        _enclosing_fn: Option<&str>,
    ) {
        let Ok(name) = node.utf8_text(self.source) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let start = node.start_byte() as u32;
        let id = format!("{}|{}|{}|{}", self.file_path, start, name, kind);
        let enclosing = self.enclosing_symbol(start).map(|s| s.to_string());
        self.bucket.occurrences.push(OccurrenceRow {
            id,
            name: name.to_string(),
            file_path: self.file_path.to_string(),
            start_byte: start,
            end_byte: node.end_byte() as u32,
            enclosing_symbol_id: enclosing,
            enclosing_scope_id: scope_id.to_string(),
            occurrence_kind: kind.to_string(),
        });
    }

    fn emit_use_bindings(&mut self, node: Node, scope_id: &str) {
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        self.emit_use_tree(arg, scope_id);
    }

    fn emit_use_tree(&mut self, node: Node, scope_id: &str) {
        match node.kind() {
            "use_as_clause" => {
                // `use a::b as c` — bind `c` as import_alias.
                let alias = node
                    .child_by_field_name("alias")
                    .and_then(|n| n.utf8_text(self.source).ok());
                if let Some(alias) = alias {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope_id.to_string(),
                        name: alias.to_string(),
                        start_byte: node.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "import_alias".to_string(),
                    });
                }
            }
            "use_wildcard" => {
                self.bucket.bindings.push(BindingRow {
                    scope_id: scope_id.to_string(),
                    name: "*".to_string(),
                    start_byte: node.start_byte() as u32,
                    symbol_id: None,
                    binding_kind: "wildcard_import".to_string(),
                });
            }
            "use_list" | "scoped_use_list" => {
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    self.emit_use_tree(child, scope_id);
                }
            }
            "scoped_identifier" | "identifier" => {
                let text = node.utf8_text(self.source).unwrap_or("");
                let leaf = text.rsplit("::").next().unwrap_or(text).trim();
                if !leaf.is_empty() {
                    self.bucket.bindings.push(BindingRow {
                        scope_id: scope_id.to_string(),
                        name: leaf.to_string(),
                        start_byte: node.start_byte() as u32,
                        symbol_id: None,
                        binding_kind: "import".to_string(),
                    });
                }
            }
            _ => {}
        }
    }
}

fn parameter_name(p: Node, source: &[u8]) -> Option<String> {
    match p.kind() {
        "self_parameter" => Some("self".to_string()),
        "parameter" => {
            let pat = p.child_by_field_name("pattern")?;
            let name_node = innermost_identifier(pat)?;
            name_node.utf8_text(source).ok().map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Walk a pattern node to the innermost `identifier` token (handles
/// `mut_pattern`, `ref_pattern`, simple identifier, etc.).
fn innermost_identifier(node: Node) -> Option<Node> {
    match node.kind() {
        "identifier" => Some(node),
        _ => {
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                if let Some(n) = innermost_identifier(child) {
                    return Some(n);
                }
            }
            None
        }
    }
}

/// Classify the occurrence_kind of an identifier-ish node based on its
/// parent context. Returns `None` for nodes that are NOT occurrences
/// (declarations, modifiers, scopes themselves).
fn occurrence_kind_for(node: Node) -> Option<&'static str> {
    // Only consider identifier-shaped leaves.
    let kind = node.kind();
    if !matches!(
        kind,
        "identifier" | "type_identifier" | "field_identifier" | "shorthand_field_identifier"
    ) {
        return None;
    }
    let parent = node.parent()?;
    match parent.kind() {
        // Declaring positions — these names are bindings, not occurrences.
        "function_item"
        | "function_signature_item"
        | "struct_item"
        | "enum_item"
        | "trait_item"
        | "type_item"
        | "union_item"
        | "const_item"
        | "static_item"
        | "macro_definition"
        | "mod_item"
        | "field_declaration"
        | "parameter"
        | "self_parameter"
        | "let_declaration"
        | "mut_pattern"
        | "enum_variant" => {
            // Identifier that IS the declared name → skip.
            if parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id()) {
                return None;
            }
            // Inside a parameter's pattern → skip; bindings handle it.
            if parent.kind() == "parameter"
                && parent.child_by_field_name("pattern").map(|n| n.id()) == Some(node.id())
            {
                return None;
            }
            Some("read")
        }
        // Inside a `use ...` path → import_use.
        "use_declaration" | "use_list" | "scoped_use_list" | "use_as_clause" | "use_wildcard" => {
            Some("import_use")
        }
        // Inside a call's function position → call.
        "call_expression" => {
            if parent.child_by_field_name("function").map(|n| n.id()) == Some(node.id()) {
                Some("call")
            } else {
                Some("read")
            }
        }
        "method_call_expression" => {
            if parent.child_by_field_name("method").map(|n| n.id()) == Some(node.id()) {
                Some("call")
            } else {
                Some("read")
            }
        }
        // LHS of assignment → write.
        "assignment_expression" => {
            if parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id()) {
                Some("write")
            } else {
                Some("read")
            }
        }
        // Compound assignment ops `x += y` → also write per ADR-0003.
        "compound_assignment_expr" => {
            if parent.child_by_field_name("left").map(|n| n.id()) == Some(node.id()) {
                Some("write")
            } else {
                Some("read")
            }
        }
        // Type positions.
        "type_identifier" => Some("type_use"),
        _ => {
            // type_identifier nodes always denote type uses regardless of context.
            if node.kind() == "type_identifier" {
                Some("type_use")
            } else {
                Some("read")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::languages;
    use crate::parser::create_parser;

    fn run(src: &str, path: &str) -> ReferencesBucket {
        let mut parser = create_parser(Language::Rust).expect("parser");
        let tree = parser.parse(src.as_bytes(), None).expect("parse");
        let q = languages::compile_symbol_query(Language::Rust).expect("query");
        let symbols = languages::extract_symbols(&tree, src.as_bytes(), &q, path, Language::Rust);
        extract_references(&tree, src.as_bytes(), path, &symbols)
    }

    #[test]
    fn file_scope_emitted() {
        let b = run("fn main() {}", "src/main.rs");
        assert!(
            b.scopes
                .iter()
                .any(|s| s.kind == "file" && s.parent_id.is_none())
        );
    }

    #[test]
    fn function_scope_emitted() {
        let b = run("fn main() {}", "src/main.rs");
        assert!(b.scopes.iter().any(|s| s.kind == "function"));
    }

    #[test]
    fn definition_binding_emitted() {
        let b = run("fn main() {}", "src/main.rs");
        assert!(
            b.bindings
                .iter()
                .any(|x| x.binding_kind == "definition" && x.name == "main")
        );
    }

    #[test]
    fn parameter_binding_emitted() {
        let b = run("fn f(x: i32) {}", "src/lib.rs");
        let p = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "parameter" && x.name == "x");
        assert!(p.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn use_import_binding_emitted() {
        let b = run("use std::collections::HashMap;", "src/lib.rs");
        let imp = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import" && x.name == "HashMap");
        assert!(imp.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn use_alias_binding_emitted() {
        let b = run("use foo::Bar as Baz;", "src/lib.rs");
        let a = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "import_alias" && x.name == "Baz");
        assert!(a.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn wildcard_import_binding_emitted() {
        let b = run("use foo::*;", "src/lib.rs");
        let w = b
            .bindings
            .iter()
            .find(|x| x.binding_kind == "wildcard_import");
        assert!(w.is_some(), "got: {:?}", b.bindings);
    }

    #[test]
    fn call_occurrence_emitted() {
        let b = run("fn f() { g(); } fn g() {}", "src/lib.rs");
        let c = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "call" && o.name == "g");
        assert!(c.is_some(), "got: {:?}", b.occurrences);
    }

    #[test]
    fn assignment_write_occurrence_emitted() {
        let b = run("fn f() { let mut x = 1; x = 2; }", "src/lib.rs");
        let w = b
            .occurrences
            .iter()
            .find(|o| o.occurrence_kind == "write" && o.name == "x");
        assert!(w.is_some(), "got: {:?}", b.occurrences);
    }
}
